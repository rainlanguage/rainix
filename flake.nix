{
  description = "Rainix is a flake for Rain.";

  inputs = {
    # Fork containing a fix for cargo-tauri on mac.
    # https://github.com/NixOS/nixpkgs/pull/279771
    nixpkgs.url = "github:nixos/nixpkgs/7a28f3cd1bb9176ff1cc21e5d120d4ef4be5cf7b";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix/main";
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

          # https://tauri.app/v1/guides/getting-started/prerequisites/#setting-up-linux
          pkgs.cargo-tauri
          pkgs.gtk4

          pkgs.foundry-bin
          pkgs.slither-analyzer
          rain.defaultPackage.${system}
        ] ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
        ]);

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

          rainix-tauri-artifacts = mkTaskLocal "rainix-tauri-artifacts";
        };

        devShells.default = pkgs.mkShell {
          buildInputs = baseBuildInputs;
        };
      }
    );
}