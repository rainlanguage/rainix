{
  description = "Rainix is a flake for Rain.";

  inputs = {
    # Fork containing a fix for cargo-tauri on mac.
    # https://github.com/NixOS/nixpkgs/pull/279771
    nixpkgs.url = "github:nixos/nixpkgs/7a28f3cd1bb9176ff1cc21e5d120d4ef4be5cf7b";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix/monthly";
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
          pkgs.rust-bin.stable."1.75.0".default

          pkgs.foundry-bin
          pkgs.slither-analyzer
          rain.defaultPackage.${system}

          # This is needed to even do things like clippy when tauri is in the
          # workspace.
          # pkgs.curl
          # pkgs.wget
          # pkgs.pkg-config
          # pkgs.dbus
          # pkgs.openssl_3
          # pkgs.glib
          # pkgs.gtk3
          # pkgs.libsoup
          # pkgs.librsvg
        ]
        # ++ (pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
        #   # for tauri
        #   pkgs.webkitgtk
        # ])
        ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
        ]);

        tauriBuildInputs = [
          # https://tauri.app/v1/guides/getting-started/prerequisites/#setting-up-linux
          pkgs.cargo-tauri
          pkgs.curl
          pkgs.wget
          pkgs.pkg-config
          pkgs.dbus
          pkgs.openssl_3
          pkgs.glib
          pkgs.gtk3
          pkgs.libsoup
          pkgs.webkitgtk
          pkgs.librsvg
          pkgs.nodejs_21
        ];

        tauriLibraries = [
          pkgs.webkitgtk
          pkgs.gtk3
          pkgs.cairo
          pkgs.gdk-pixbuf
          pkgs.glib
          pkgs.dbus
          pkgs.openssl_3
          pkgs.librsvg
        ];

        # https://ertt.ca/nix/shell-scripts/
        mkTask = { name, body, additionalBuildInputs ? [] }: pkgs.symlinkJoin {
          name = name;
          paths = [
            ((pkgs.writeScriptBin name body).overrideAttrs(old: {
              buildCommand = "${old.buildCommand}\n patchShebangs $out";
            }))
          ] ++ baseBuildInputs ++ additionalBuildInputs;
          buildInputs = [ pkgs.makeWrapper ];
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

          # rainix-tauri-artifacts = mkTask rec {
          #   name = "rainix-tauri-artifacts";
          #   body = (builtins.readFile ./task/${name}.sh);
          #   additionalBuildInputs = tauriBuildInputs;
          # };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = baseBuildInputs;
        };

        devShells.tauri-shell = pkgs.mkShell {
          buildInputs = baseBuildInputs ++ tauriBuildInputs;
          shellHook =
            ''
              export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath tauriLibraries}:$LD_LIBRARY_PATH
              export XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:$XDG_DATA_DIRS
            '';
        };
      }
    );
}