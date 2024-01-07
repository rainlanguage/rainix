{
  description = "Example Rainix consumer.";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    rainix.url = "github:rainlanguage/rainix";
  };

  outputs = inputs@{ self, flake-utils, rainix, ...}:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = rainix.pkgs.${system};
      in {
        packages = {
          # Replace this with whatever you want as the default for `nix run`.
          default = pkgs.writeShellScriptBin "hello" ''
            echo "Hello, world!";
          '';
        }
        // rainix.packages.${system};

        devShells = rainix.devShells.${system};
      }
    );
}