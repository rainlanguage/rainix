{
  description = "Rainix is a flake for Rain.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/ec750fd01963ab6b20ee1f0cb488754e8036d89d";
    rain.url = "github:rainprotocol/rain.cli/6a912680be6d967fd6114aafab793ebe8503d27b";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix/main";
  };

  outputs = inputs@{ self, nixpkgs, rain, flake-utils, rust-overlay, foundry, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays =[ (import rust-overlay) foundry.overlay ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        forge-bin = "${pkgs.foundry-bin}/bin/forge";

      in {
        pkgs = pkgs;
        # There's a collision with pkgs.rain so here's our binary.
        rainlang = rain;

        packages = {
          ci-test-sol = pkgs.writeShellScriptBin "ci-test-sol" ''
            ${forge-bin} test -vvv
          '';
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            pkgs.rust-bin.stable."1.75.0".default
            pkgs.foundry-bin
            pkgs.slither-analyzer
          ] ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ]);
        };
      }
    );
}