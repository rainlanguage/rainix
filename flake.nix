{
  description = "Rainix is a flake for Rain.";

  inputs = {
    # Fork containing a fix for cargo-tauri on mac.
    # https://github.com/NixOS/nixpkgs/pull/279771
    nixpkgs.url = "github:nixos/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix/324fe20d07ce9c0f237dc2727454f04204e85c00";
    rain.url = "github:rainprotocol/rain.cli";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, foundry, rain }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays =[ (import rust-overlay) foundry.overlay ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        baseBuildInputs = [
          pkgs.rust-bin.stable."1.74.0".default
          # Needed by common rust deps
          pkgs.gmp

          pkgs.foundry-bin
          pkgs.slither-analyzer
          rain.defaultPackage.${system}
        ]
        ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          pkgs.darwin.apple_sdk.frameworks.AppKit
        ]);

        # https://ertt.ca/nix/shell-scripts/
        mkTask = { name, body, additionalBuildInputs ? [] }: pkgs.symlinkJoin {
          name = name;
          paths = [
            ((pkgs.writeScriptBin name body).overrideAttrs(old: {
              buildCommand = "${old.buildCommand}\n patchShebangs $out";
            }))
          ] ++ baseBuildInputs ++ additionalBuildInputs;
          buildInputs = [ pkgs.makeWrapper ] ++ baseBuildInputs ++ additionalBuildInputs;
          postBuild = "wrapProgram $out/bin/${name} --prefix PATH : $out/bin";
        };
        mkTaskLocal = name: mkTask { name = name; body = (builtins.readFile ./task/${name}.sh); };

      in {
        pkgs = pkgs;
        buildInputs = baseBuildInputs;
        mkTask = mkTask;

        packages = {
          rainix-prelude = mkTaskLocal "rainix-prelude";

          rainix-sol-test = mkTaskLocal "rainix-sol-test";
          rainix-sol-artifacts = mkTaskLocal "rainix-sol-artifacts";
          rainix-sol-static = mkTaskLocal "rainix-sol-static";

          rainix-rs-test = mkTaskLocal "rainix-rs-test";
          rainix-rs-artifacts = mkTaskLocal "rainix-rs-artifacts";
          rainix-rs-static = mkTaskLocal "rainix-rs-static";
        };

        devShells.default = pkgs.mkShell {
          buildInputs = baseBuildInputs;
        };

        # https://tauri.app/v1/guides/getting-started/prerequisites/#setting-up-linux
        devShells.tauri-shell = let
          tauriBuildInputs = [
            pkgs.cargo-tauri
            pkgs.curl
            pkgs.wget
            pkgs.pkg-config
            pkgs.dbus
            pkgs.openssl_3
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

          tauriLibraries = [
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
          buildInputs = baseBuildInputs ++ tauriBuildInputs;
          shellHook =
            ''
              export WEBKIT_DISABLE_COMPOSITING_MODE=1
              export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath tauriLibraries}:$LD_LIBRARY_PATH
              export XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:$XDG_DATA_DIRS
            '';
        };
      }
    );
}