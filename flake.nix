{
  description = "Rainix is a flake for Rain.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix";
    solc.url = "github:hellwolf/solc.nix";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, foundry, solc }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays =[ (import rust-overlay) foundry.overlay solc.overlay ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rust-version = "1.79.0";
        rust-toolchain = pkgs.rust-bin.stable.${rust-version}.default.override (previous: {
          targets = previous.targets ++ [ "wasm32-unknown-unknown" ];
        });

        # need to build from source since it errors on macos with current rainix rust version 1.79 on rainix
        # and the version available on rainix.pkgs is 1.0.100 which is not compatible with rust 1.79,
        # the latest version that works with rust 1.79 is v1.0.95 so we build form source
        cargo-expand = (pkgs.makeRustPlatform{
          rustc = rust-toolchain;
          cargo = rust-toolchain;
        }).buildRustPackage rec {
          pname = "cargo-expand";
          version = "1.0.95";
          src = pkgs.fetchFromGitHub {
            executable = true;
            owner = "dtolnay";
            repo = "cargo-expand";
            tag = "1.0.95";
            hash = "sha256-VEjgSmZcy/CZ8EO/mJ2nBOpQviF4A/QQ8SpLLF/9x4c=";
          };
          useFetchCargoVendor = true;
          cargoHash = "sha256-ow5Zy0tv9W5w+Pib2yW1nPj2pUZt0HhplHxjIZZZzU8=";
        };

        # need to build latest version of wbg-cli from source
        # since updating flake cause error for tauri v1 build
        wasm-bindgen-cli = pkgs.rustPlatform.buildRustPackage rec {
          pname = "wasm-bindgen-cli";
          version = "0.2.100";

          src = pkgs.fetchCrate {
            inherit pname version;
            hash = "sha256-3RJzK7mkYFrs7C/WkhW9Rr4LdP5ofb2FdYGz1P7Uxog=";
          };
          cargoHash = "sha256-tD0OY2PounRqsRiFh8Js5nyknQ809ZcHMvCOLrvYHRE=";
          doCheck = false;

          nativeBuildInputs = [ pkgs.pkg-config ];
          nativeCheckInputs = [ pkgs.nodejs_latest ];
          buildInputs = [
            pkgs.openssl
          ] ++ (with pkgs; lib.optionals stdenv.hostPlatform.isDarwin [
            curl
            darwin.apple_sdk.frameworks.Security
          ]);
        };

        rust-build-inputs = [
          rust-toolchain
          pkgs.cargo-release
          pkgs.gmp
          pkgs.openssl
          pkgs.libusb1
          pkgs.pkg-config
          wasm-bindgen-cli
          pkgs.gettext
          pkgs.libiconv
          pkgs.cargo-flamegraph
          cargo-expand
        ]
        ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
          # pkgs.glibc
        ])
        ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.DarwinTools
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          pkgs.darwin.apple_sdk.frameworks.AppKit
          pkgs.darwin.apple_sdk.frameworks.WebKit
          pkgs.darwin.apple_sdk.frameworks.Security
        ]);

        sol-build-inputs = [
          pkgs.git
          pkgs.foundry-bin
          pkgs.slither-analyzer
          pkgs.solc_0_8_19
          pkgs.reuse
        ];

        node-build-inputs = [
            pkgs.nodejs_22
        ];
        network-list = [
          "base"
          "flare"
        ];
        the-graph = pkgs.stdenv.mkDerivation rec {
          pname = "the-graph";
          version = "0.69.2";
          src = let
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
          buildInputs = [];
          installPhase = ''
            mkdir -p $out
            cp -r $src/* $out
          '';
        };

        goldsky = pkgs.stdenv.mkDerivation rec {
          pname = "goldsky";
          version = "8.6.6";
          src = let
              release-name = "8.6.6";
              system-mapping = {
                x86_64-linux = "linux";
                x86_64-darwin = "macos";
                aarch64-darwin = "macos";
              };
              system-sha = {
                x86_64-linux = "sha256:1cqbinax63w07qxvmgni52qw4cd83ywkhjikw3rd4wgd2fh36027";
                x86_64-darwin = "sha256:0yznf81yxc3a9vnfjdmmzdb59mh9bwrpxw87lrlhlchfr0jmnjk4";
                aarch64-darwin = "sha256:0yznf81yxc3a9vnfjdmmzdb59mh9bwrpxw87lrlhlchfr0jmnjk4";
              };
            in
            builtins.fetchurl {
              url = "https://cli.goldsky.com/${release-name}/${system-mapping.${system}}/goldsky";
              sha256 = system-sha.${system};
            };
          buildInputs = [];
          phases = ["installPhase"];
          installPhase = ''
            mkdir -p $out/bin
            cp $src $out/bin/goldsky
            chmod +x $out/bin/goldsky
          '';
        };

        tauri-build-inputs = [
          pkgs.cargo-tauri
          pkgs.curl
          pkgs.wget
          pkgs.pkg-config
          pkgs.dbus
          pkgs.glib
          pkgs.gtk3
          pkgs.libsoup
          pkgs.librsvg
          pkgs.gettext
          pkgs.libiconv
          pkgs.glib-networking
        ]
        ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
          # This is probably needed but is is marked as broken in nixpkgs
          pkgs.webkitgtk
        ]);

        tauri-release-env = pkgs.buildEnv {
          name = "Tauri release environment";
          # Currently we don't use the tauri build inputs as above because
          # it doesn't seem to be totally supported by the github action, even
          # though the above is as documented by tauri.
          paths = [pkgs.cargo-tauri] ++ rust-build-inputs ++ node-build-inputs;
        };

        # https://ertt.ca/nix/shell-scripts/
        mkTask = { name, body, additionalBuildInputs ? [] }: pkgs.symlinkJoin {
          name = name;
          paths = [
            ((pkgs.writeScriptBin name body).overrideAttrs(old: {
              buildCommand = "${old.buildCommand}\n patchShebangs $out";
            }))
          ] ++ additionalBuildInputs;
          buildInputs = [ pkgs.makeWrapper ] ++ additionalBuildInputs;
          postBuild = "wrapProgram $out/bin/${name} --prefix PATH : $out/bin";
        };

        rainix-sol-prelude = mkTask {
          name = "rainix-sol-prelude";
          # We do NOT do a shallow clone in the prelude because nix flakes
          # seem to not be compatible with shallow clones.
          # The reason we do a forge build here is that the output of the
          # build is a set of artifacts that other tasks often need to use,
          # such as the ABI and the bytecode.
          body = ''
            set -euxo pipefail
            forge install
            forge build
          '';
          additionalBuildInputs = sol-build-inputs;
        };

        rainix-sol-static = mkTask {
          name = "rainix-sol-static";
          body = ''
            set -euxo pipefail
            slither .
            forge fmt --check
          '';
          additionalBuildInputs = sol-build-inputs;
        };

        rainix-sol-legal = mkTask {
          name = "rainix-sol-legal";
          body = ''
            set -euxo pipefail
            reuse lint
          '';
          additionalBuildInputs = sol-build-inputs;
        };

        rainix-sol-test = mkTask {
          name = "rainix-sol-test";
          body = ''
            set -euxo pipefail
            forge test -vvv
          '';
          additionalBuildInputs = sol-build-inputs;
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

        rainix-rs-prelude = mkTask {
          name = "rainix-rs-prelude";
          body = ''
            set -euxo pipefail
          '';
          additionalBuildInputs = rust-build-inputs;
        };

        rainix-rs-static = mkTask {
          name = "rainix-rs-static";
          body = ''
            set -euxo pipefail
            cargo fmt --all -- --check
            cargo clippy --all-targets --all-features -- -D clippy::all
          '';
          additionalBuildInputs = rust-build-inputs;
        };

        rainix-rs-test = mkTask {
          name = "rainix-rs-test";
          body = ''
            set -euxo pipefail
            cargo test
          '';
          additionalBuildInputs = rust-build-inputs;
        };

        rainix-rs-artifacts = mkTask {
          name = "rainix-rs-artifacts";
          body = ''
            set -euxo pipefail
            cargo build --release
          '';
          additionalBuildInputs = rust-build-inputs;
        };

        rainix-tasks = [
          rainix-sol-prelude
          rainix-sol-static
          rainix-sol-test
          rainix-sol-artifacts
          rainix-sol-legal

          rainix-rs-prelude
          rainix-rs-static
          rainix-rs-test
          rainix-rs-artifacts
        ];

        subgraph-build = mkTask {
          name = "subgraph-build";
          body = ''
            set -euxo pipefail
            forge build
            cd ./subgraph;
            npm install;
            ${the-graph}/bin/graph codegen;
            ${the-graph}/bin/graph build;
            cd -;
          '';
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
            set -euo pipefail
            ${subgraph-build}/bin/subgraph-build

            (cd ./subgraph && ${goldsky}/bin/goldsky --token ''${GOLDSKY_TOKEN} subgraph deploy ''${GOLDSKY_NAME_AND_VERSION})
          '';
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
      in {
        pkgs = pkgs;
        rust-toolchain = rust-toolchain;
        rust-build-inputs = rust-build-inputs;
        sol-build-inputs = sol-build-inputs;
        node-build-inputs = node-build-inputs;
        mkTask = mkTask;
        network-list = network-list;

        packages = {
          rainix-sol-prelude = rainix-sol-prelude;
          rainix-sol-static = rainix-sol-static;
          rainix-sol-test = rainix-sol-test;
          rainix-sol-artifacts = rainix-sol-artifacts;
          rainix-sol-legal = rainix-sol-legal;

          rainix-rs-prelude = rainix-rs-prelude;
          rainix-rs-static = rainix-rs-static;
          rainix-rs-test = rainix-rs-test;
          rainix-rs-artifacts = rainix-rs-artifacts;

          tauri-release-env = tauri-release-env;

        };

        devShells.default = pkgs.mkShell {
          buildInputs = sol-build-inputs ++ rust-build-inputs ++ node-build-inputs ++ rainix-tasks ++ subgraph-tasks ++ [ the-graph goldsky ];
          shellHook =
          ''
          ${source-dotenv}

          if [ -f ./package.json ]; then
            npm install --ignore-scripts;
          fi
          '';
        };

        # https://tauri.app/v1/guides/getting-started/prerequisites/#setting-up-linux
        devShells.tauri-shell = let
          tauri-libraries = [
            pkgs.gtk3
            pkgs.cairo
            pkgs.gdk-pixbuf
            pkgs.glib
            pkgs.dbus
            pkgs.openssl_3
            pkgs.librsvg
            pkgs.gettext
            pkgs.libiconv
          ]
          ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
            # This is probably needed but is is marked as broken in nixpkgs
            pkgs.webkitgtk
          ]);
        in pkgs.mkShell {
          buildInputs = sol-build-inputs ++ rust-build-inputs ++ node-build-inputs ++ tauri-build-inputs;
          shellHook =
            ''
              ${source-dotenv}

              export TMP_BASE64_PATH=$(mktemp -d)
              cp /usr/bin/base64 "$TMP_BASE64_PATH/base64"
              export PATH="$TMP_BASE64_PATH:$PATH:/usr/bin:/usr/bin/SetFile"
              export WEBKIT_DISABLE_COMPOSITING_MODE=1
              export XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:$XDG_DATA_DIRS
              export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules/";
            '';
        };
      }
    );
}