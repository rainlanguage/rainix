{ writeShellApplication
, foundry-bin
}:
writeShellApplication {
  name = "rainix-sol-test";
  meta.description = "Rainix Solidity tests";
  runtimeInputs = [
    foundry-bin
  ];
  text = ''
    forge test -vvv
  '';
}
