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

        # wasm-bindgen CLI and crate versions must match exactly or
        # `wasm-bindgen` over the wasm file fails. nixpkgs' default lags behind
        # what current lockfiles resolve to, so pin the CLI to that version via
        # the nixpkgs builder (no nixpkgs bump, no other tool churn).
        wasm-bindgen-cli =
          let
            pname = "wasm-bindgen-cli";
            version = "0.2.122";
          in
          pkgs.buildWasmBindgenCli rec {
            # `fetchCrate` defaults to the crates.io API endpoint
            # (`/api/v1/crates/.../download`), which 403s generic User-Agents —
            # including nix's fetcher — so any build that has to fetch this crate
            # (i.e. a nix store cache miss) fails with "cannot download ... from
            # any mirror". Override the URL to the static.crates.io CDN, which
            # serves the byte-identical .crate with no User-Agent gate. fetchCrate
            # still unpacks the tarball and sets pname/version passthru, so the
            # NAR hash and all downstream usage are unchanged.
            src = pkgs.fetchCrate {
              inherit pname version;
              url = "https://static.crates.io/crates/${pname}/${pname}-${version}.crate";
              hash = "sha256-vO4RSxi/sMWxmsEs3GuljdMfIRSu75A+Q+c5wgYToRU=";
            };
            cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
              inherit src;
              inherit (src) pname version;
              hash = "sha256-Inup6vvJSG5ghNyeDPyZbfZo4d0LsMG2OJfStoaeDBs=";
            };
          };

        rust-build-inputs = [
          rust-toolchain
          pkgs.cargo-release
          pkgs.cargo-expand
          pkgs.foundry-bin
          pkgs.gmp
          pkgs.openssl
          pkgs.libusb1
          pkgs.sqlite
          pkgs.pkg-config
          wasm-bindgen-cli
          pkgs.gettext
          pkgs.libiconv
          pkgs.cargo-flamegraph
        ]
        ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
          # pkgs.glibc
        ])
        ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.DarwinTools ]);

        # The Solidity toolchain shared by sol-shell and default. sol-only
        # repos (rain.solmem, rain.deploy, rain.datacontract, etc.) consume
        # this set via `nix develop github:rainlanguage/rainix#sol-shell` and
        # nothing else, so it carries no browser (chromium/playwright), node,
        # rust, or wasm dependency — keeping the sol-shell closure slim. The
        # sol-shell closure test asserts chromium, node, and the rust
        # toolchain stay absent.
        sol-build-inputs = [
          pkgs.git
          pkgs.foundry-bin
          pkgs.slither-analyzer
          pkgs.solc_0_8_25
          pkgs.reuse
          # jq is the canonical tool for extracting stable subsets of
          # forge build artifacts via `vm.ffi` in CopyArtifacts.sol-style
          # scripts.
          pkgs.jq
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

        # Enforces Rain's one-contract-per-file convention (rainix#214). Runs
        # over git-tracked .sol files so it works the same locally and in CI.
        # `library`/`interface` are not counted; only `contract` and
        # `abstract contract` declarations.
        rainix-sol-single-contract = mkTask {
          name = "rainix-sol-single-contract";
          body = ''
            set -euo pipefail
            source ${./lib/sol-single-contract.sh}
            sol_single_contract_check_tracked
          '';
          additionalBuildInputs = [ pkgs.git ];
        };

        # Enforces Rain's pragma convention (rainix#250): ^ for library/
        # interface/abstract contract files so downstream soldeer consumers can
        # compile them; = for concrete contract files to pin the compiler.
        # Skips src/generated/ and dependencies/ (vendored).
        rainix-sol-pragma-convention = mkTask {
          name = "rainix-sol-pragma-convention";
          body = ''
            set -euo pipefail
            source ${./lib/sol-pragma-convention.sh}
            sol_pragma_convention_check_tracked
          '';
          additionalBuildInputs = [ pkgs.git ];
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
          rainix-sol-single-contract
          rainix-sol-pragma-convention
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
            (cd ./subgraph && ${pkgs.nodejs_22}/bin/npm ci && docker compose up --abort-on-container-exit)
          '';
        };

        subgraph-deploy = mkTask {
          name = "subgraph-deploy";
          body = ''
            set -euxo pipefail
            source ${./lib/subgraph.sh}

            # subgraph/abis and subgraph/generated are committed, so the deploy
            # builds the subgraph directly from them with just the graph +
            # goldsky toolchain — the same committed-artifact path as
            # subgraph-test, slim enough for the subgraph shell.
            (cd ./subgraph && ${pkgs.nodejs_22}/bin/npm ci)

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
          additionalBuildInputs = node-build-inputs;
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
            bats test/bats/devshell/default/prettier-bundle.test.bats
            bats test/bats/task/skip-simulation.test.bats
            bats test/bats/task/subgraph-build.test.bats
            bats test/bats/task/subgraph-deploy-version.test.bats
            bats test/bats/task/sol-single-contract.test.bats
            bats test/bats/task/sol-pragma-convention.test.bats
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
            bats test/bats/devshell/sol-shell/jq.test.bats
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
            bats test/bats/devshell/rust-shell/sqlite.test.bats
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
            rainix-sol-single-contract
            rainix-sol-pragma-convention
            rainix-rs-static
            prettier-bundle
            sol-shell-test
            rust-shell-test
            ;
        };

        devShells = {
          # Slim shell for Solidity-only repos: no rust, node,
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
          # rs-tasks, no sol/node. Mirror of sol-shell for the
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

          # Slim shell for repos that ship a Rust crate plus a node/WASM
          # binding (e.g. rain.math.float's @rainlanguage/float npm
          # package built from the rust-shell-compiled WASM). Adds node
          # to rust-shell without dragging in the heavy sol/subgraph
          # closure. Keeps everything under nix so PATH ordering and
          # version drift between actions/setup-node and nix go away.
          rust-node-shell = pkgs.mkShell {
            buildInputs = rust-build-inputs ++ node-build-inputs ++ rs-tasks ++ common-shell-inputs;
            shellHook = ''
              ${pre-commit.shellHook}
              ${source-dotenv}
            '';
          };

          # Slim shell for repos that ship a wasm-pack browser test or an
          # npm wrapper around the rust-compiled WASM. rust-node-shell +
          # wasm-pack. No sol-tasks, no subgraph, no sqlite/yq/age.
          wasm-shell = pkgs.mkShell {
            buildInputs =
              rust-build-inputs ++ node-build-inputs ++ rs-tasks ++ common-shell-inputs ++ [ pkgs.wasm-pack ];
            shellHook = ''
              ${pre-commit.shellHook}
              ${source-dotenv}
            '';
          };

          # Slim shell for subgraph repos: node + the-graph + goldsky +
          # subgraph-tasks. No rust, no foundry, no sqlite/yq/age. Lets
          # consumers avoid the heavy default closure when CI is just
          # subgraph-test.
          subgraph-shell = pkgs.mkShell {
            buildInputs =
              node-build-inputs
              ++ subgraph-tasks
              ++ common-shell-inputs
              ++ [
                the-graph
                goldsky
              ];
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
                pkgs.sqlite
                pkgs.yq-go
                pkgs.age
                default-shell-test
              ];
            shellHook = ''
              export RAINIX_PRETTIER_BUNDLE_DIR=${prettier-bundle}
              ${pre-commit.shellHook}
              ${source-dotenv}

              if [ -f ./package.json ] && ! cmp -s ./package-lock.json ./node_modules/.package-lock.json 2>/dev/null; then
                npm ci --ignore-scripts;
              fi
            '';
          };

        };
      }
    );
}
