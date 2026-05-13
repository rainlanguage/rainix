{
  description = "Rainix is a flake for Rain.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix";
    solc.url = "github:hellwolf/solc.nix";
    git-hooks-nix.url = "github:cachix/git-hooks.nix";
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      foundry,
      solc,
      git-hooks-nix,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [
          (import rust-overlay)
          foundry.overlay
          solc.overlay
        ];
        pkgs = import nixpkgs { inherit system overlays; };

        rust-version = "1.94.0";
        rust-toolchain = pkgs.rust-bin.stable.${rust-version}.default.override (previous: {
          targets = previous.targets ++ [ "wasm32-unknown-unknown" ];
          extensions = previous.extensions ++ [
            "rust-src"
            "rust-analyzer"
          ];
        });

        rust-build-inputs = [
          rust-toolchain
          pkgs.cargo-release
          pkgs.cargo-expand
          pkgs.foundry-bin
          pkgs.gmp
          pkgs.openssl
          pkgs.libusb1
          pkgs.pkg-config
          pkgs.wasm-bindgen-cli
          pkgs.gettext
          pkgs.libiconv
          pkgs.cargo-flamegraph
        ]
        ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
          # pkgs.glibc
        ])
        ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.DarwinTools ]);

        sol-build-inputs = [
          pkgs.git
          pkgs.foundry-bin
          pkgs.slither-analyzer
          pkgs.solc_0_8_25
          pkgs.reuse
        ];

        node-build-inputs = [
          pkgs.nodejs_22
          pkgs.jq
        ];
        the-graph = pkgs.stdenv.mkDerivation {
          pname = "the-graph";
          version = "0.69.2";
          src =
            let
              release-name = "%40graphprotocol%2Fgraph-cli%400.69.2";
              system-mapping = {
                x86_64-linux = "linux-x64";
                x86_64-darwin = "darwin-x64";
                aarch64-darwin = "darwin-arm64";
              };
              system-sha = {
                x86_64-linux = "sha256:07grrdrx8w3m8sqwdmf9z9zymwnnzxckgnnjzfndk03a8r2d826m";
                x86_64-darwin = "sha256:0j4p2bkx6pflkif6xkvfy4vj1v183mkg59p2kf3rk48wqfclids8";
                aarch64-darwin = "sha256:0pq0g0fq1myp0s58lswhcab6ccszpi5sx6l3y9a18ai0c6yzxim0";
              };
            in
            builtins.fetchTarball {
              url = "https://github.com/graphprotocol/graph-tooling/releases/download/${release-name}/graph-${system-mapping.${system}}.tar.gz";
              sha256 = system-sha.${system};
            };
          buildInputs = [ ];
          installPhase = ''
            mkdir -p $out
            cp -r $src/* $out
          '';
        };

        goldsky = pkgs.stdenv.mkDerivation rec {
          pname = "goldsky";
          version = "13.3.4";
          src =
            let
              release-name = "13.3.4";
              system-mapping = {
                x86_64-linux = "linux";
                x86_64-darwin = "macos";
                aarch64-darwin = "macos";
              };
              system-sha = {
                x86_64-linux = "sha256:1wg09vz652hv3hb0w7mx7hjxm00c857h2a8kd2vj11wnik8gh73m";
                x86_64-darwin = "sha256:048w06x56lk84h9x8q2jf7mdxx8lyzd9nrkxsmfkj39rns1nr4yk";
                aarch64-darwin = "sha256:048w06x56lk84h9x8q2jf7mdxx8lyzd9nrkxsmfkj39rns1nr4yk";
              };
            in
            builtins.fetchurl {
              url = "https://cli.goldsky.com/${release-name}/${system-mapping.${system}}/goldsky";
              sha256 = system-sha.${system};
            };
          buildInputs = [ ];
          phases = [ "installPhase" ];
          installPhase = ''
            mkdir -p $out/bin
            cp $src $out/bin/goldsky
            chmod +x $out/bin/goldsky
          '';
        };

        # Cross-platform `chromium` binary for headless rendering inside the dev
        # shell (e.g. dumping rendered DOM of a deployed SPA preview to debug
        # JS-side errors). On Linux, defer to the nixpkgs chromium build. On
        # Darwin, nixpkgs has no chromium, so wrap the system Chrome.app — fails
        # at invocation time with a clear message when Chrome is not installed.
        chromium = pkgs.writeShellScriptBin "chromium" (
          if pkgs.stdenv.isDarwin then
            ''
              chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
              if [ ! -x "$chrome" ]; then
                echo "rainix chromium wrapper: Google Chrome.app not found at $chrome" >&2
                echo "Install Chrome (https://www.google.com/chrome/) or use a Linux dev shell." >&2
                exit 127
              fi
              exec "$chrome" "$@"
            ''
          else
            ''
              exec ${pkgs.chromium}/bin/chromium "$@"
            ''
        );

        # rainix-curated prettier bundle: a single nix-built node_modules
        # tree containing prettier + the standardized plugins, plus a
        # .prettierrc.json picked up via PRETTIER_BUNDLE_DIR. Consumers
        # MUST NOT ship their own prettier or prettier-plugin-* packages —
        # the no-consumer-prettier pre-commit hook below enforces that.
        prettier-bundle = pkgs.buildNpmPackage {
          pname = "rainix-prettier-bundle";
          version = "0.0.0";
          src = ./prettier-bundle;
          npmDepsHash = "sha256-64dISGPfTPK7LUSL43CKoHM5SPZYqx6Ngg2dBgsqIyg=";
          dontNpmBuild = true;
          installPhase = ''
            runHook preInstall
            mkdir -p $out
            cp -r node_modules $out/
            cp .prettierrc.json $out/
            mkdir -p $out/bin
            ln -s $out/node_modules/.bin/prettier $out/bin/prettier
            runHook postInstall
          '';
        };

        # https://ertt.ca/nix/shell-scripts/
        mkTask =
          {
            name,
            body,
            additionalBuildInputs ? [ ],
          }:
          pkgs.symlinkJoin {
            inherit name;
            paths = [
              ((pkgs.writeScriptBin name body).overrideAttrs (old: {
                buildCommand = ''
                  ${old.buildCommand}
                   patchShebangs $out'';
              }))
            ]
            ++ additionalBuildInputs;
            buildInputs = [ pkgs.makeWrapper ] ++ additionalBuildInputs;
            postBuild = "wrapProgram $out/bin/${name} --prefix PATH : $out/bin";
          };

        rainix-sol-artifacts = mkTask {
          name = "rainix-sol-artifacts";
          body = ''
            set -euxo pipefail

            # Upload all function selectors to the registry.
            forge selectors up --all

            # Deploy all contracts to testnet.
            # Assumes the existence of a `Deploy.sol` script in the `script` directory.
            # Echos the deploy pubkey to stdout to make it easy to add gas to the account.
            echo 'deploy pubkey:';
            cast wallet address "''${DEPLOYMENT_KEY}";
            # Need to set --rpc-url explicitly due to an upstream bug.
            # https://github.com/foundry-rs/foundry/issues/6731

            attempts=;
            do_deploy() {
              forge script script/Deploy.sol:Deploy \
                -vvvvv \
                --slow \
                ''${DEPLOY_LEGACY:+--legacy} \
                ''${DEPLOY_BROADCAST:+--broadcast} \
                ''${DEPLOY_SKIP_SIMULATION:+--skip-simulation} \
                --rpc-url "''${ETH_RPC_URL}" \
                ''${DEPLOY_VERIFY:+--verify} \
                ''${DEPLOY_VERIFIER:+--verifier "''${DEPLOY_VERIFIER}"} \
                ''${DEPLOY_VERIFIER_URL:+--verifier-url "''${DEPLOY_VERIFIER_URL}"} \
                ''${ETHERSCAN_API_KEY:+--etherscan-api-key "''${ETHERSCAN_API_KEY}"} \
                ''${attempts:+--resume} \
                ;
            }

            until do_deploy; do
              attempts=$((''${attempts:-0} + 1));
              echo "Deploy failed, retrying in 5 seconds... (attempt ''${attempts})";
              sleep 5;
              if [[ ''${attempts} -gt 10 ]]; then
                echo "Deploy failed after 10 attempts, aborting.";
                exit 1;
              fi
            done
          '';
          additionalBuildInputs = sol-build-inputs;
        };

        rainix-rs-static = mkTask {
          name = "rainix-rs-static";
          body = ''
            set -euxo pipefail
            cargo fmt --all -- --check
            cargo clippy --all-targets --all-features -- -D warnings -D clippy::all
          '';
          additionalBuildInputs = rust-build-inputs;
        };

        sol-tasks = [
          rainix-sol-artifacts
        ];

        rs-tasks = [
          rainix-rs-static
        ];

        rainix-tasks = sol-tasks ++ rs-tasks;

        # Dev tooling shared between sol-shell and default.
        common-shell-inputs = [
          pkgs.gh
          pkgs.pre-commit
        ]
        ++ pre-commit.enabledPackages;

        subgraph-build = mkTask {
          name = "subgraph-build";
          body = ''
            set -euxo pipefail
            source ${./lib/subgraph.sh}

            ${pkgs.foundry-bin}/bin/forge build
            (cd ./subgraph && ${pkgs.nodejs_22}/bin/npm ci && ${the-graph}/bin/graph codegen)
            for network in $(subgraph_networks ./subgraph/networks.json); do
              echo "Building subgraph for $network..."
              (cd ./subgraph && ${the-graph}/bin/graph build --network "$network")
            done
          '';
          additionalBuildInputs = sol-build-inputs ++ node-build-inputs;
        };

        subgraph-test = mkTask {
          name = "subgraph-test";
          body = ''
            set -euxo pipefail
            (cd ./subgraph && docker compose up --abort-on-container-exit)
          '';
        };

        subgraph-deploy = mkTask {
          name = "subgraph-deploy";
          body = ''
            set -euxo pipefail
            source ${./lib/subgraph.sh}

            ${pkgs.foundry-bin}/bin/forge build
            (cd ./subgraph && ${pkgs.nodejs_22}/bin/npm ci && ${the-graph}/bin/graph codegen)

            commit="$(${pkgs.git}/bin/git rev-parse --short HEAD)"
            for network in $(subgraph_networks ./subgraph/networks.json); do
              address=$(subgraph_network_address ./subgraph/networks.json "$network")
              version=$(subgraph_deploy_version "$address" "$commit")
              name_and_version="''${GOLDSKY_SUBGRAPH_NAME}-$network/$version"

              if ${goldsky}/bin/goldsky --token ''${GOLDSKY_TOKEN} subgraph list "$name_and_version" 2>/dev/null | grep -q "$name_and_version"; then
                echo "Subgraph $name_and_version already deployed, skipping."
              else
                echo "Building subgraph for $network..."
                (cd ./subgraph && ${the-graph}/bin/graph build --network "$network")
                echo "Deploying subgraph $name_and_version..."
                (cd ./subgraph && ${goldsky}/bin/goldsky --token ''${GOLDSKY_TOKEN} subgraph deploy "$name_and_version")
              fi
            done
          '';
          additionalBuildInputs = sol-build-inputs ++ node-build-inputs;
        };

        subgraph-tasks = [
          subgraph-build
          subgraph-test
          subgraph-deploy
        ];

        source-dotenv = ''
          if [ -f ./.env ]; then
            set -a
            source .env
            set +a
          fi
        '';

        default-shell-test = mkTask {
          name = "default-shell-test";
          body = ''
            bats test/bats/devshell/default/solc.test.bats
            bats test/bats/devshell/default/gh.test.bats
            bats test/bats/devshell/default/age.test.bats
            bats test/bats/devshell/default/chromium.test.bats
            bats test/bats/devshell/default/prettier-bundle.test.bats
            bats test/bats/task/skip-simulation.test.bats
            bats test/bats/task/subgraph-build.test.bats
            bats test/bats/task/subgraph-deploy-version.test.bats
          '';
          additionalBuildInputs = [ pkgs.bats ] ++ sol-build-inputs ++ node-build-inputs;
        };

        sol-shell-test = mkTask {
          name = "sol-shell-test";
          body = ''
            bats test/bats/devshell/sol-shell/forge.test.bats
            bats test/bats/devshell/sol-shell/slither.test.bats
            bats test/bats/devshell/sol-shell/solc.test.bats
            bats test/bats/devshell/sol-shell/reuse.test.bats
            bats test/bats/devshell/sol-shell/gh.test.bats
            bats test/bats/devshell/sol-shell/sol-tasks.test.bats
            bats test/bats/devshell/sol-shell/slim.test.bats
            bats test/bats/devshell/sol-shell/closure.test.bats
          '';
          additionalBuildInputs = [ pkgs.bats ] ++ sol-build-inputs;
        };

        rust-shell-test = mkTask {
          name = "rust-shell-test";
          body = ''
            bats test/bats/devshell/rust-shell/closure.test.bats
            bats test/bats/devshell/rust-shell/cargo-expand.test.bats
            bats test/bats/devshell/rust-shell/anvil.test.bats
          '';
          additionalBuildInputs = [ pkgs.bats ] ++ rust-build-inputs;
        };

        pre-commit = git-hooks-nix.lib.${system}.run {
          src = ./.;
          hooks = {
            # Nix
            nil.enable = true;
            nixfmt.enable = true;

            deadnix.enable = true;
            # excluded for files generated by bun2nix
            deadnix.excludes = [ "bun\\.nix$" ];

            statix.enable = true;
            statix.settings.ignore = [ "lib/" ];

            # Rust
            taplo.enable = true;
            # Conditional rustfmt — runs only if there is rust source AND
            # cargo-fmt is on PATH. Resolving cargo-fmt via PATH at runtime
            # (instead of nix-store interpolation) keeps rust-toolchain out
            # of the hook's nix closure, so consumers of sol-shell — which
            # have no rust to format — do not pull the rust toolchain in.
            rustfmt-conditional = {
              enable = true;
              name = "rustfmt";
              entry = "${pkgs.writeShellScript "rustfmt-conditional" ''
                command -v cargo-fmt >/dev/null 2>&1 || exit 0
                if [ -f Cargo.toml ] || [ -f */Cargo.toml ]; then
                  exec cargo-fmt fmt
                fi
              ''}";
              files = "\\.rs$";
              pass_filenames = false;
            };

            # Svelte/JS/TS — use the rainix-curated prettier bundle so all
            # consumers run identical prettier core + plugin versions AND
            # identical formatting rules. `no-consumer-prettier` below blocks
            # consumers from shipping their own prettier, plugins, or
            # prettierrc, so the bundle is the only prettier in play.
            #
            # The bundle is resolved via $RAINIX_PRETTIER_BUNDLE_DIR rather
            # than nix-store interpolation. Default shell exports the var;
            # sol-shell does not, and the hook silently no-ops there. This
            # keeps prettier-bundle (which transitively brings nodejs) out
            # of the sol-shell closure for consumers that have no JS to
            # format.
            # Custom hook name (rather than overriding the built-in
            # `prettier` hook) so git-hooks.nix does not splice its default
            # `package = pkgs.nodePackages.prettier` into enabledPackages —
            # that default is what kept nodejs in sol-shell's closure.
            prettier-rainix = {
              enable = true;
              name = "prettier-rainix";
              entry = toString (
                pkgs.writeShellScript "prettier-rainix-bundled" ''
                  set -e
                  if [ -z "''${RAINIX_PRETTIER_BUNDLE_DIR:-}" ]; then
                    exit 0
                  fi
                  exec "$RAINIX_PRETTIER_BUNDLE_DIR/bin/prettier" \
                    --config="$RAINIX_PRETTIER_BUNDLE_DIR/.prettierrc.json" \
                    --plugin="$RAINIX_PRETTIER_BUNDLE_DIR/node_modules/prettier-plugin-svelte/plugin.js" \
                    --plugin="$RAINIX_PRETTIER_BUNDLE_DIR/node_modules/prettier-plugin-tailwindcss/dist/index.mjs" \
                    --ignore-unknown --list-different --write "$@"
                ''
              );
              types_or = [
                "svelte"
                "ts"
                "javascript"
                "json"
              ];
            };

            # Block consumers from shipping their own prettier, prettier
            # plugins, or prettierrc. The rainix bundle is the canonical
            # source; any consumer-side prettier setup either reintroduces
            # the version-skew bug (#117) or lets formatting drift from the
            # rainix standard.
            no-consumer-prettier = {
              enable = true;
              name = "no-consumer-prettier";
              entry = toString (
                pkgs.writeShellScript "no-consumer-prettier" ''
                  set -e
                  if [ -f package.json ]; then
                    forbidden_deps="$(${pkgs.jq}/bin/jq -r '
                      [.dependencies // {}, .devDependencies // {}, .optionalDependencies // {}, .peerDependencies // {}]
                      | add // {}
                      | keys[]
                      | select(. == "prettier" or startswith("prettier-plugin-"))
                    ' package.json)"
                    if [ -n "$forbidden_deps" ]; then
                      echo "ERROR: package.json must not declare prettier or prettier-plugin-*." >&2
                      echo "rainix supplies these via the pre-commit hook bundle." >&2
                      echo "Forbidden packages found:" >&2
                      printf '  - %s\n' $forbidden_deps >&2
                      exit 1
                    fi
                    if ${pkgs.jq}/bin/jq -e 'has("prettier")' package.json > /dev/null 2>&1; then
                      echo "ERROR: package.json must not contain a top-level \"prettier\" key." >&2
                      echo "Prettier config inlined in package.json drifts from the rainix canon; delete the key." >&2
                      exit 1
                    fi
                  fi
                  for cfg in \
                    .prettierrc \
                    .prettierrc.json \
                    .prettierrc.json5 \
                    .prettierrc.yaml \
                    .prettierrc.yml \
                    .prettierrc.toml \
                    .prettierrc.js \
                    .prettierrc.cjs \
                    .prettierrc.mjs \
                    .prettierrc.ts \
                    .prettierrc.cts \
                    .prettierrc.mts \
                    prettier.config.js \
                    prettier.config.cjs \
                    prettier.config.mjs \
                    prettier.config.ts \
                    prettier.config.cts \
                    prettier.config.mts; do
                    if [ -e "$cfg" ]; then
                      echo "ERROR: $cfg is present in the repo." >&2
                      echo "rainix supplies the canonical prettier config via the bundle." >&2
                      echo "Delete $cfg." >&2
                      exit 1
                    fi
                  done
                ''
              );
              # always_run so the check fires on every commit, not just commits
              # that happen to stage package.json or a prettierrc. A consumer
              # could otherwise sneak in a forbidden file in one commit and
              # introduce drift in the next.
              always_run = true;
              pass_filenames = false;
            };

            # Misc
            denofmt = {
              enable = true;
              excludes = [
                ".*\\.ts$"
                ".*\\.js$"
                ".*\\.json$"
                ".*\\.svelte$"
              ];
            };
            yamlfmt.enable = true;
            yamlfmt.settings.lint-only = false;
            shellcheck.enable = true;
          };
        };

      in
      {
        checks.pre-commit = pre-commit;

        inherit
          pkgs
          rust-toolchain
          rust-build-inputs
          sol-build-inputs
          node-build-inputs
          mkTask
          ;

        packages = {
          inherit
            rainix-sol-artifacts
            rainix-rs-static
            prettier-bundle
            sol-shell-test
            rust-shell-test
            ;
        };

        devShells = {
          # Slim shell for Solidity-only repos: no chromium, rust, node,
          # subgraph, sqlite, age. Lets sol-only consumers (rain.solmem,
          # rain.deploy, rain.datacontract, etc.) avoid the heavy default
          # closure when their CI is just rainix-sol-{test,static,legal}.
          sol-shell = pkgs.mkShell {
            buildInputs = sol-build-inputs ++ sol-tasks ++ common-shell-inputs;
            shellHook = ''
              ${pre-commit.shellHook}
              ${source-dotenv}
            '';
          };

          # Slim shell for Rust-only repos: rust toolchain + cargo + the
          # rs-tasks, no sol/node/chromium. Mirror of sol-shell for the
          # Rust side. Consumers like rain.cli that ship a pure rust
          # binary can alias `default = rust-shell` and skip the heavy
          # default closure.
          rust-shell = pkgs.mkShell {
            buildInputs = rust-build-inputs ++ rs-tasks ++ common-shell-inputs;
            shellHook = ''
              ${pre-commit.shellHook}
              ${source-dotenv}
            '';
          };

          default = pkgs.mkShell {
            buildInputs =
              sol-build-inputs
              ++ rust-build-inputs
              ++ node-build-inputs
              ++ rainix-tasks
              ++ subgraph-tasks
              ++ common-shell-inputs
              ++ [
                the-graph
                goldsky
                chromium
                pkgs.sqlite
                pkgs.yq-go
                pkgs.age
                default-shell-test
              ];
            shellHook = ''
              export RAINIX_PRETTIER_BUNDLE_DIR=${prettier-bundle}
              ${pre-commit.shellHook}
              ${source-dotenv}

              if [ -f ./package.json ]; then
                npm ci --ignore-scripts;
              fi
            '';
          };

        };
      }
    );
}
