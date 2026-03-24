setup() {
  source lib/subgraph.sh
}

@test "subgraph_networks should list networks from networks.json" {
  run subgraph_networks test/fixture/subgraph/networks.json
  [ "$status" -eq 0 ]
  [[ "$output" == *"mainnet"* ]]
}

@test "subgraph_network_address should extract address from networks.json" {
  run subgraph_network_address test/fixture/subgraph/networks.json mainnet
  [ "$status" -eq 0 ]
  [ "$output" = "0x0000000000000000000000000000000000000000" ]
}

@test "subgraph_deploy_version should be deterministic" {
  v1=$(subgraph_deploy_version "0xabc" "abc1234")
  v2=$(subgraph_deploy_version "0xabc" "abc1234")
  [ "$v1" = "$v2" ]
}

@test "subgraph_deploy_version should differ for different addresses" {
  v1=$(subgraph_deploy_version "0xaaa" "abc1234")
  v2=$(subgraph_deploy_version "0xbbb" "abc1234")
  [ "$v1" != "$v2" ]
}

@test "subgraph_deploy_version should differ for different commits" {
  v1=$(subgraph_deploy_version "0xabc" "abc1234")
  v2=$(subgraph_deploy_version "0xabc" "def5678")
  [ "$v1" != "$v2" ]
}

@test "subgraph_deploy_version should contain address and commit" {
  v=$(subgraph_deploy_version "0xabc" "abc1234")
  [ "$v" = "0xabc-abc1234" ]
}
