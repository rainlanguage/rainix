{ writeShellApplication
, foundry-bin
, the-graph
, nodejs
, subgraph
}:
writeShellApplication {
  name = "subgraph-build";
  meta.description = "Build the subgraph for all networks";
  runtimeInputs = [
    foundry-bin
    the-graph
    nodejs
    subgraph
  ];
  extraShellCheckFlags = [ "-x" ];
  text = ''
    source ${subgraph}/lib/subgraph.sh

    forge build
    (cd ./subgraph && npm ci && graph codegen)
    for network in $(subgraph_networks ./subgraph/networks.json); do
      echo "Building subgraph for $network..."
      (cd ./subgraph && graph build --network "$network")
    done
  '';
}
