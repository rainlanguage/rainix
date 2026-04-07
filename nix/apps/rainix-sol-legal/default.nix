{ writeShellApplication
, reuse
}:
writeShellApplication {
  name = "rainix-sol-legal";
  meta.description = "Rainix Solidity licensing";
  runtimeInputs = [ reuse ];
  text = ''
    reuse lint
  '';
}
