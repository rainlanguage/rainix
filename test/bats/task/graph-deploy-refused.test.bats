# The `graph` this flake ships must exit non-zero when the Graph node refuses a
# deploy: subgraph-deploy reads that status to tell a deployed network from one
# that did not complete. Real graph-cli, real compile, against
# fake-graph-node.mjs, which accepts the IPFS upload and refuses the deploy.

setup() {
  fixture="$BATS_TEST_DIRNAME/../../fixture"
  work="$(mktemp -d)"
  cp -r "$fixture/foundry.toml" "$fixture/src" "$work/"
  mkdir "$work/subgraph"
  (cd "$fixture/subgraph" && cp -r package.json package-lock.json networks.json schema.graphql subgraph.yaml src "$work/subgraph/")
  (cd "$work" && forge build)
  (cd "$work/subgraph" && npm ci && graph codegen)

  node "$BATS_TEST_DIRNAME/fake-graph-node.mjs" "$work/port" &
  node_pid=$!
  for _ in $(seq 1 50); do
    [ -s "$work/port" ] && break
    sleep 0.1
  done
  port="$(cat "$work/port")"
}

teardown() {
  kill "$node_pid" 2>/dev/null || true
  rm -rf "$work"
}

@test "graph deploy exits non-zero when the node refuses the deploy" {
  cd "$work/subgraph"
  run graph deploy counter \
    --network mainnet \
    --node "http://127.0.0.1:$port/deploy" \
    --ipfs "http://127.0.0.1:$port/ipfs" \
    --deploy-key not-a-key \
    --version-label v
  [[ "$output" == *"Build completed"* ]]
  [[ "$output" == *"E017"* ]]
  [ "$status" -ne 0 ]
}
