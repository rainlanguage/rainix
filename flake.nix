{
  description = "Rainix is a flake for Rain.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/ec750fd01963ab6b20ee1f0cb488754e8036d89d";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix/main";
  };

  outputs = inputs@{ self, nixpkgs, flake-utils, rust-overlay, foundry, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays =[ (import rust-overlay) foundry.overlay ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        forge-bin = "${pkgs.foundry-bin}/bin/forge";
        slither-bin = "${pkgs.slither-analyzer}/bin/slither";
        rust-bin-pin = pkgs.rust-bin.stable."1.75.0".default;
        cargo-bin = "${rust-bin-pin}/bin/cargo";
      in {
        pkgs = pkgs;

        packages = {
          ci-test-sol = pkgs.writeShellScriptBin "ci-test-sol" ''
            ${forge-bin} test -vvv
          '';

          ci-test-rs = pkgs.writeShellScriptBin "ci-test-rs" ''
            ${cargo-bin} fmt --check
            ${cargo-bin} clippy
            ${cargo-bin} test
          '';

          ci-slither = pkgs.writeShellScriptBin "ci-slither" ''
            ${slither-bin} .
          '';
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            rust-bin-pin
            pkgs.foundry-bin
            pkgs.slither-analyzer
          ] ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ]);
        };
      }
    );
}