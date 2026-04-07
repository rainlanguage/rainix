{
  description = "Rain lang development toolchains and infra.";

  # TODO: Check for substituters.

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    rust-overlay.url = "github:oxalica/rust-overlay";
    foundry.url = "github:shazow/foundry.nix";
    solc.url = "github:hellwolf/solc.nix";
    # old nixpkgs, pinned for webkitgtk and libsoup-2.4 needed for tauri shell and build
    nixpkgs-25_05.url = "github:nixos/nixpkgs?rev=48975d7f9b9960ed33c4e8561bcce20cc0c2de5b";
    git-hooks-nix.url = "github:cachix/git-hooks.nix";
  };

  outputs =
    { self
    , nixpkgs
    , git-hooks-nix
    , ...
    }@inputs:
    let
      systems = [
        "x86_64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      eachSystem = f: nixpkgs.lib.genAttrs systems (system:
        let
          pkgs = import nixpkgs {
            localSystem = { inherit system; };
            overlays = [ overlays.default ];
          };
        in
        f system pkgs);

      overlays =
        let
          rust-overlay = inputs.rust-overlay.overlays.default;
          solc = inputs.solc.overlay;
          foundry = inputs.foundry.overlay;
          the-graph = _final: prev: {
            the-graph = prev.callPackage ./nix/packages/the-graph { };
          };
          goldsky = _final: prev: {
            goldsky = prev.callPackage ./nix/packages/goldsky { };
          };
          rainix =
            let
              rainix-sol = final: prev:
                let solc = final.solc_0_8_25; in {
                  foundry-bin = prev.foundry-bin.overrideAttrs (old: {
                    buildInputs = old.buildInputs or [ ] ++ [ solc ];
                  });
                  slither-analyzer = prev.slither-analyzer.override {
                    inherit solc;
                    withSolc = true;
                  };
                };
              rainix-rs = _final: prev:
                let RUST_VERSION = "1.94.0"; in {
                  rainix = prev.rainix or { } // {
                    rust-toolchain =
                      prev.rust-bin.stable.${RUST_VERSION}.default.override (old: {
                        targets = old.targets or [ ] ++ [ "wasm32-unknown-unknown" ];
                        extensions = old.extensions or [ ] ++ [
                          "rust-src"
                          "rust-analyzer"
                        ];
                      });
                  };
                };
              rainix-apps = final: prev:
                let
                  inherit (final.rainix) rust-toolchain;
                  inherit (final.rainix.lib) subgraph;
                  nodejs = final.nodejs_22;
                  cargo-tauri = final.cargo-tauri_1;
                  wasm-bindgen-cli = final.wasm-bindgen-cli_0_2_100;
                in
                {
                  rainix = prev.rainix or { } // {
                    lib.subgraph = prev.callPackage ./nix/packages/subgraph { };
                    envs.tauri-release-env = prev.callPackage ./nix/packages/tauri-release-env {
                      inherit (final.rainix) rust-toolchain;
                      inherit
                        nodejs
                        cargo-tauri
                        wasm-bindgen-cli
                        ;
                    };
                    apps = {
                      rainix-sol-prelude = prev.callPackage ./nix/apps/rainix-sol-prelude { };
                      rainix-sol-static = prev.callPackage ./nix/apps/rainix-sol-static { };
                      rainix-sol-legal = prev.callPackage ./nix/apps/rainix-sol-legal { };
                      rainix-sol-test = prev.callPackage ./nix/apps/rainix-sol-test { };
                      rainix-sol-artifacts = prev.callPackage ./nix/apps/rainix-sol-artifacts { };
                      rainix-rs-prelude = prev.callPackage ./nix/apps/rainix-rs-prelude {
                        inherit rust-toolchain;
                        inherit wasm-bindgen-cli;
                      };
                      rainix-rs-static = prev.callPackage ./nix/apps/rainix-rs-static { inherit rust-toolchain; };
                      rainix-rs-test = prev.callPackage ./nix/apps/rainix-rs-test { inherit rust-toolchain; };
                      rainix-rs-artifacts = prev.callPackage ./nix/apps/rainix-rs-artifacts { inherit rust-toolchain; };
                      subgraph-build = prev.callPackage ./nix/apps/subgraph-build { inherit nodejs subgraph; };
                      subgraph-test = prev.callPackage ./nix/apps/subgraph-test { };
                      subgraph-deploy = prev.callPackage ./nix/apps/subgraph-deploy { inherit nodejs subgraph; };
                    };
                  };
                };
            in
            nixpkgs.lib.composeManyExtensions [
              rust-overlay
              solc
              foundry
              the-graph
              goldsky
              rainix-sol
              rainix-rs
              rainix-apps
            ];
        in
        {
          inherit
            rust-overlay
            solc
            foundry
            the-graph
            goldsky
            rainix
            ;
          default = rainix;
        };
    in
    {
      inherit overlays;

      legacyPackages = eachSystem (_system: pkgs: pkgs);

      # TODO: Add commit-hook check
      checks = eachSystem (system: _pkgs: {
        inherit (self.packages.${system}) default;
      });

      packages = eachSystem (_system: pkgs:
        let
          packages = pkgs.rainix.envs // pkgs.rainix.apps;
        in
        packages // {
          default = pkgs.symlinkJoin {
            name = "rainix";
            description = "Builds all Rainix packages";
            paths = builtins.attrValues packages;
          };
        });

      apps = eachSystem (_system: pkgs: pkgs.lib.mapAttrs (_: pkg: {
        inherit (pkg) meta;
        type = "app";
        program = "${pkg}/bin/${pkg.meta.mainProgram or pkg.name}";
      })
        pkgs.rainix.apps);

      # TODO:
      # devShells = eachSystem (system: pkgs: {
      #
      # });
    };
}
