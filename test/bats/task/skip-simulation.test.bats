setup() {
  cd test/fixture || exit
  forge build
  anvil &
  ANVIL_PID=$!
  sleep 2
}

teardown() {
  kill "$ANVIL_PID" 2>/dev/null
  wait "$ANVIL_PID" 2>/dev/null
}

forge_deploy() {
  forge script script/Deploy.sol:Deploy \
    -vvvvv \
    --broadcast \
    ${DEPLOY_SKIP_SIMULATION:+--skip-simulation} \
    --rpc-url http://127.0.0.1:8545 \
    --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
    2>&1
}

@test "forge script should simulate on-chain by default" {
  output=$(forge_deploy)
  status=$?

  [ "$status" -eq 0 ]
  [[ "$output" == *"Simulated On-chain Traces"* ]]
  [[ "$output" == *"ONCHAIN EXECUTION COMPLETE & SUCCESSFUL"* ]]
}

@test "DEPLOY_SKIP_SIMULATION should skip on-chain simulation" {
  DEPLOY_SKIP_SIMULATION=1
  output=$(forge_deploy)
  status=$?

  [ "$status" -eq 0 ]
  [[ "$output" == *"SKIPPING ON CHAIN SIMULATION"* ]]
  [[ "$output" == *"ONCHAIN EXECUTION COMPLETE & SUCCESSFUL"* ]]
}
