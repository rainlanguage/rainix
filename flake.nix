{
  description = "Rainix is a flake for Rain.";

  inputs = {
    # Pinned because someone broke python in main. :(
    nixpkgs.url = "github:nixos/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix/9ecf12199280f738eaaad2d1224e54403dbdf426";
    rain.url = "github:rainlanguage/rain.cli";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, foundry, rain }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays =[ (import rust-overlay) foundry.overlay ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rust-version = "1.75.0";
        rust-toolchain = pkgs.rust-bin.stable.${rust-version}.default.override (previous: {
          targets = previous.targets ++ [ "wasm32-unknown-unknown" ];
        });

        rust-build-inputs = [
          rust-toolchain
          pkgs.cargo-release
          pkgs.gmp
          pkgs.openssl
          pkgs.libusb
          pkgs.pkg-config
          pkgs.wasm-bindgen-cli
          pkgs.gettext
          pkgs.libiconv
        ]
        ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
          # pkgs.glibc
        ])
        ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          pkgs.darwin.apple_sdk.frameworks.AppKit
          pkgs.darwin.apple_sdk.frameworks.WebKit
        ]);

        sol-build-inputs = [
          pkgs.foundry-bin
          pkgs.slither-analyzer
          rain.defaultPackage.${system}
        ];

        node-build-inputs = [
            pkgs.nodejs_21
        ];

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

            if [[ -z "''${DEPLOY_VERIFIER:-}" ]]; then
              forge script script/Deploy.sol:Deploy \
                -vvvvv \
                --slow \
                --legacy \
                --rpc-url "''${ETH_RPC_URL}" \
                ;
            else
              forge script script/Deploy.sol:Deploy \
                -vvvvv \
                --slow \
                --legacy \
                --broadcast \
                --rpc-url "''${ETH_RPC_URL}" \
                --verify \
                --verifier "''${DEPLOY_VERIFIER}" \
                ''${DEPLOY_VERIFIER_URL:+--verifier-url "''${DEPLOY_VERIFIER_URL}"} \
                --etherscan-api-key "''${ETHERSCAN_API_KEY}" \
                ;
            fi


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

          rainix-rs-prelude
          rainix-rs-static
          rainix-rs-test
          rainix-rs-artifacts
        ];
      in {
        pkgs = pkgs;
        rust-toolchain = rust-toolchain;
        rust-build-inputs = rust-build-inputs;
        sol-build-inputs = sol-build-inputs;
        node-build-inputs = node-build-inputs;
        mkTask = mkTask;

        packages = {
          rainix-sol-prelude = rainix-sol-prelude;
          rainix-sol-static = rainix-sol-static;
          rainix-sol-test = rainix-sol-test;
          rainix-sol-artifacts = rainix-sol-artifacts;

          rainix-rs-prelude = rainix-rs-prelude;
          rainix-rs-static = rainix-rs-static;
          rainix-rs-test = rainix-rs-test;
          rainix-rs-artifacts = rainix-rs-artifacts;

          tauri-release-env = tauri-release-env;
        };

        devShells.default = pkgs.mkShell {
          packages = sol-build-inputs ++ rust-build-inputs ++ rainix-tasks;
        };

        # https://tauri.app/v1/guides/getting-started/prerequisites/#setting-up-linux
        devShells.tauri-shell = let
          tauri-libraries = [
            pkgs.gtk3
            pkgs.cairo
            pkgs.gdk-pixbuf
            pkgs.glib
            pkgs.dbus
            pkgs.openssl_3_1
            pkgs.librsvg
            pkgs.gettext
            pkgs.libiconv
          ]
          ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
            # This is probably needed but is is marked as broken in nixpkgs
            pkgs.webkitgtk
            # pkgs.glibc
          ]);
        in pkgs.mkShell {
          packages = sol-build-inputs ++ rust-build-inputs ++ node-build-inputs ++ tauri-build-inputs;
          # nativeBuildInputs = [pkgs.pkg-config];
          # buildInputs = [ pkgs.gtk3 pkgs.glib ];
          # buildInputs = tauri-libraries;
          buildInputs = [pkgs.pkg-config];
          shellHook =
            ''
              export PKG_CONFIG_PATH=${tauri-libraries.openssl_3_1}/lib/pkgconfig:$PKG_CONFIG_PATH;
              echo "pkg config path"
              echo $PKG_CONFIG_PATH
              export PATH="/usr/bin:$PATH"
              export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath tauri-libraries}:$LD_LIBRARY_PATH
              export WEBKIT_DISABLE_COMPOSITING_MODE=1
              export XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:$XDG_DATA_DIRS
            '';
        };
      }
    );
}