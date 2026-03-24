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
