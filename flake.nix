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

        baseBuildInputs = [
          pkgs.rust-bin.stable."1.75.0".default
          pkgs.foundry-bin
          pkgs.slither-analyzer
        ] ++ (pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
        ]);

        # https://ertt.ca/nix/shell-scripts/
        mkCITask = name: pkgs.symlinkJoin {
            name = name;
            paths = [
              ((pkgs.writeScriptBin name (builtins.readFile ./ci/${name}.sh)).overrideAttrs(old: {
                buildCommand = "${old.buildCommand}\n patchShebangs $out";
              }))
            ] ++ baseBuildInputs;
            buildInputs = [ pkgs.makeWrapper ];
            postBuild = "wrapProgram $out/bin/${name} --prefix PATH : $out/bin";
          };

      in {
        pkgs = pkgs;
        buildInputs = baseBuildInputs;

        packages = {
          ci-sol-test = pkgs.writeShellScriptBin "ci-sol-test" ''
            ${forge-bin} test -vvv
          '';

          ci-sol-artifacts = pkgs.writeShellScriptBin "ci-sol-artifacts" ''
            ${forge-bin} selectors up --all
          '';

          # Slither first to avoid any potential conflicts with other checks.
          ci-sol-static = pkgs.writeShellScriptBin "ci-sol-static" ''
            ${slither-bin} --ignore-compile --skip-clean .
            ${forge-bin} fmt --check
          '';

          ci-rs-test = mkCITask "ci-rs-test";
          ci-rs-artifacts = mkCITask "ci-rs-artifacts";

          ci-rs-static = pkgs.writeShellScriptBin "ci-rs-static" ''
            ${cargo-bin} fmt --check
            ${cargo-bin} clippy
          '';
        };

        devShells.default = pkgs.mkShell {
          buildInputs = baseBuildInputs;
        };
      }
    );
}