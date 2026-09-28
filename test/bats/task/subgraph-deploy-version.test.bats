setup() {
  # shellcheck disable=SC1091
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

@test "ormi_query_is_deployed accepts a live _meta response" {
  run ormi_query_is_deployed '{"data":{"_meta":{"block":{"number":51907769}}}}'
  [ "$status" -eq 0 ]
}

@test "ormi_query_is_deployed rejects a missing version" {
  run ormi_query_is_deployed '{"error":"subgraph name/version error"}'
  [ "$status" -eq 1 ]
}

@test "ormi_query_is_deployed rejects an empty response" {
  run ormi_query_is_deployed ""
  [ "$status" -eq 1 ]
}
