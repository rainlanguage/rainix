{
  description = "Rainix is a flake for Rain.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/ec750fd01963ab6b20ee1f0cb488754e8036d89d";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix/main";
    rain.url = "github:rainprotocol/rain.cli/6a912680be6d967fd6114aafab793ebe8503d27b";
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
        ] ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
        ]);

        # https://ertt.ca/nix/shell-scripts/
        mkTask = { name, additionalBuildInputs ? [], body ? (builtins.readFile ./task/${name}.sh) }: pkgs.symlinkJoin {
          name = name;
          paths = [
            ((pkgs.writeScriptBin name body).overrideAttrs(old: {
              buildCommand = "${old.buildCommand}\n patchShebangs $out";
            }))
          ] ++ baseBuildInputs ++ additionalBuildInputs;
          buildInputs = [ pkgs.makeWrapper ];
          postBuild = "wrapProgram $out/bin/${name} --prefix PATH : $out/bin";
        };

      in {
        pkgs = pkgs;
        buildInputs = baseBuildInputs;
        mkTask = mkTask;

        packages = {
          rainix-prelude = mkTask { name = "rainix-prelude"; };

          rainix-sol-test = mkTask { name = "rainix-sol-test"; };
          rainix-sol-artifacts = mkTask { name = "rainix-sol-artifacts"; };
          rainix-sol-static = mkTask { name = "rainix-sol-static"; };

          rainix-rs-test = mkTask { name = "rainix-rs-test"; };
          rainix-rs-artifacts = mkTask { name = "rainix-rs-artifacts"; };
          rainix-rs-static = mkTask { name = "rainix-rs-static"; };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = baseBuildInputs;
        };
      }
    );
}