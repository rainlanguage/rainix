  [[ "$output" != *"/bin/forge build"* ]]
setup() {
  cd test/fixture || exit
  rm -rf subgraph/generated subgraph/build subgraph/node_modules
}

@test "subgraph-build should codegen and compile subgraph" {
  run subgraph-build
  [ "$status" -eq 0 ]
  [[ "$output" == *"Types generated successfully"* ]]
  [[ "$output" == *"Build completed"* ]]
  [ -f subgraph/generated/schema.ts ]
  [ -f subgraph/build/subgraph.yaml ]
}

# A subgraph whose data sources differ per network: manifests/<network>.yaml,
# no networks.json. mainnet's manifest has two data sources, the case one
# networks.json address per network cannot express.
@test "subgraph-build builds each manifest of a manifests subgraph" {
  rm -rf manifests-subgraph/generated manifests-subgraph/build manifests-subgraph/node_modules
  SUBGRAPH_DIR=manifests-subgraph run subgraph-build
  [ "$status" -eq 0 ]
  [[ "$output" == *"Building subgraph for mainnet"* ]]
  [[ "$output" == *"Building subgraph for sepolia"* ]]
  [[ "$output" != *"--network"* ]]
  [ "$(grep -c "Build completed" <<<"$output")" -eq 2 ]
  [ -f manifests-subgraph/generated/schema.ts ]
}

@test "subgraph-build refuses a SUBGRAPH_DIR that is not a path of plain names" {
  SUBGRAPH_DIR=../x run subgraph-build
  [ "$status" -ne 0 ]
  [[ "$output" == *"not a path of plain directory names"* ]]
  # Refused before anything is built.
  [[ "$output" != *"/bin/forge build"* ]]
}
