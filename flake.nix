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
        mkTaskLocal = name: inputs: mkTask { name = name; body = (builtins.readFile ./task/${name}.sh); additionalBuildInputs = inputs; };

      in {
        pkgs = pkgs;
        rust-build-inputs = rust-build-inputs;
        sol-build-inputs = sol-build-inputs;
        all-build-inputs = all-build-inputs;
        mkTask = mkTask;

        packages = {
          rainix-sol-prelude = mkTaskLocal "rainix-sol-prelude" sol-build-inputs;
          rainix-sol-test = mkTaskLocal "rainix-sol-test" sol-build-inputs;
          rainix-sol-artifacts = mkTaskLocal "rainix-sol-artifacts" sol-build-inputs;
          rainix-sol-static = mkTaskLocal "rainix-sol-static" sol-build-inputs;

          rainix-rs-prelude = mkTaskLocal "rainix-rs-prelude" rust-build-inputs;
          rainix-rs-test = mkTaskLocal "rainix-rs-test" rust-build-inputs;
          rainix-rs-artifacts = mkTaskLocal "rainix-rs-artifacts" rust-build-inputs;
          rainix-rs-static = mkTaskLocal "rainix-rs-static" rust-build-inputs;
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