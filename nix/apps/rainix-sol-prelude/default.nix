{ writeShellApplication
, foundry-bin
, slither-analyzer
, reuse
, git
}:
writeShellApplication {
  name = "rainix-sol-prelude";
  meta.description = "Rainix Solidity prelude";
  runtimeInputs = [
    foundry-bin
    slither-analyzer
    reuse
    git
  ];
  text = ''
    # We do NOT do a shallow clone in the prelude because nix flakes
    # seem to not be compatible with shallow clones.
    # The reason we do a forge build here is that the output of the
    # build is a set of artifacts that other tasks often need to use,
    # such as the ABI and the bytecode.

    forge install
    forge build
  '';
}
