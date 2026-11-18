setup() {
  # shellcheck disable=SC1091
  source lib/subgraph.sh
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

@test "subgraph_deploy_version should differ for different build hashes" {
  v1=$(subgraph_deploy_version "0xabc" "abc1234")
  v2=$(subgraph_deploy_version "0xabc" "def5678")
  [ "$v1" != "$v2" ]
}

@test "subgraph_deploy_version should be address-build hash" {
  v=$(subgraph_deploy_version "0xabc" "abc1234")
  [ "$v" = "0xabc-abc1234" ]
}
