{
  description = "Rainix is a flake for Rain.";

  inputs = {
    # Pinned because someone broke python in main. :(
    nixpkgs.url = "github:nixos/nixpkgs/9e68f1146cacc5f45b6646e73c54c88c73e8df12";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix/9ecf12199280f738eaaad2d1224e54403dbdf426";
    rain.url = "github:rainprotocol/rain.cli";
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
          pkgs.gmp
          pkgs.openssl_3
          pkgs.libusb
        ] ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          pkgs.darwin.apple_sdk.frameworks.AppKit
          pkgs.darwin.apple_sdk.frameworks.WebKit
        ]);

        sol-build-inputs = [
          pkgs.foundry-bin
          pkgs.slither-analyzer
          rain.defaultPackage.${system}
        ];

        all-build-inputs = rust-build-inputs ++ sol-build-inputs;

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

      in {
        pkgs = pkgs;
        rust-toolchain = rust-toolchain;
        rust-build-inputs = rust-build-inputs;
        sol-build-inputs = sol-build-inputs;
        all-build-inputs = all-build-inputs;
        mkTask = mkTask;

        packages = {

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
              # Mind the bash-fu on --verify.
              # https://stackoverflow.com/questions/42985611/how-to-conditionally-add-flags-to-shell-scripts
              if [ ! -z "''${BLOCKSCOUT_URL}" ]
              then
                forge script script/Deploy.sol:Deploy \
                  -vvvvv \
                  --slow \
                  --legacy \
                  --verify \
                  --verifier blockscout \
                  --verify-url "''${BLOCKSCOUT_URL}" \
                  --broadcast \
                  --rpc-url "''${ETH_RPC_URL}"

              else
                forge script script/Deploy.sol:Deploy \
                    -vvvvv \
                    --slow \
                    --legacy \
                    ''${ETHERSCAN_API_KEY:+--verify} \
                    --broadcast \
                    --rpc-url "''${ETH_RPC_URL}"
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
        };

        devShells.default = pkgs.mkShell {
          buildInputs = all-build-inputs;
        };

        # https://tauri.app/v1/guides/getting-started/prerequisites/#setting-up-linux
        devShells.tauri-shell = let
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
            pkgs.nodejs_21
          ]
          ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
            # This is probably needed but is is marked as broken in nixpkgs
            pkgs.webkitgtk
          ]);

          tauri-libraries = [
            pkgs.gtk3
            pkgs.cairo
            pkgs.gdk-pixbuf
            pkgs.glib
            pkgs.dbus
            pkgs.openssl_3
            pkgs.librsvg
          ]
          ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
            # This is probably needed but is is marked as broken in nixpkgs
            pkgs.webkitgtk
          ]);
        in pkgs.mkShell {
          buildInputs = all-build-inputs ++ tauri-build-inputs;
          shellHook =
            ''
              export WEBKIT_DISABLE_COMPOSITING_MODE=1
              export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath tauri-libraries}:$LD_LIBRARY_PATH
              export XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:$XDG_DATA_DIRS
            '';
        };
      }
    );
}