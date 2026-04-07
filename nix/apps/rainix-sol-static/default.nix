{ writeShellApplication
, foundry-bin
, slither-analyzer
}:
writeShellApplication {
  name = "rainix-sol-static";
  meta.description = "Rainix Solidity static analysis";
  runtimeInputs = [
    foundry-bin
    slither-analyzer
  ];
  text = ''
    slither .
    forge fmt --check
  '';
}
