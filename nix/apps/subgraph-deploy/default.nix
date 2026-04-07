{ writeShellApplication
, foundry-bin
, the-graph
, goldsky
, nodejs
, git
, subgraph
}:
writeShellApplication {
  name = "subgraph-deploy";
  meta.description = ''
    Builds and deploys versioned subgraphs to Goldsky for each network
    defined in networks.json, skipping any that are already deployed.
  '';
  runtimeInputs = [
    foundry-bin
    the-graph
    goldsky
    nodejs
    git
    subgraph
  ];
  extraShellCheckFlags = [ "-x" ];
  text = ''
    source ${subgraph}/lib/subgraph.sh

    forge build
    (cd ./subgraph && npm ci && graph codegen)

    commit="$(git rev-parse --short HEAD)"
    for network in $(subgraph_networks ./subgraph/networks.json); do
      address=$(subgraph_network_address ./subgraph/networks.json "$network")
      version=$(subgraph_deploy_version "$address" "$commit")
      name_and_version="''${GOLDSKY_SUBGRAPH_NAME}-$network/$version"

      if goldsky --token "''${GOLDSKY_TOKEN}" subgraph list "$name_and_version" 2>/dev/null | grep -q "$name_and_version"; then
        echo "Subgraph $name_and_version already deployed, skipping."
      else
        echo "Building subgraph for $network..."
        (cd ./subgraph && graph build --network "$network")
        echo "Deploying subgraph $name_and_version..."
        (cd ./subgraph && goldsky --token "''${GOLDSKY_TOKEN}" subgraph deploy "$name_and_version")
      fi
    done
  '';
}
