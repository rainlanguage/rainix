{ writeTextFile
, jq
}:
writeTextFile {
  name = "subgraph.sh";
  destination = "/lib/subgraph.sh";
  derivationArgs.propagatedBuildInputs = [ jq ];
  text = builtins.readFile ../../../lib/subgraph.sh;
}
