#!/usr/bin/env bash

# Extract the contract address for a network from networks.json.
# Usage: subgraph_network_address <networks_json_path> <network>
subgraph_network_address() {
  local networks_json="$1"
  local network="$2"
  jq -r --arg net "$network" '.[$net] | to_entries[0].value.address' "$networks_json"
}

# The deploy version label for a network: its contract address and the hash of
# its build (`rainix-static subgraph-build-hash`).
# Usage: subgraph_deploy_version <address> <build-hash>
subgraph_deploy_version() {
  local address="$1"
  local build_hash="$2"
  echo "${address}-${build_hash}"
}

# List all networks defined in networks.json.
# Usage: subgraph_networks <networks_json_path>
subgraph_networks() {
  local networks_json="$1"
  jq -r 'keys[]' "$networks_json"
}
