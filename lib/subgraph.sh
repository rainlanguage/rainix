#!/usr/bin/env bash

# Extract the contract address for a network from networks.json.
# Usage: subgraph_network_address <networks_json_path> <network>
subgraph_network_address() {
  local networks_json="$1"
  local network="$2"
  jq -r --arg net "$network" '.[$net] | to_entries[0].value.address' "$networks_json"
}

# Derive a deterministic deploy version from contract address and git commit.
# Usage: subgraph_deploy_version <address> <commit>
subgraph_deploy_version() {
  local address="$1"
  local commit="$2"
  echo "${address}-${commit}"
}

# List all networks defined in networks.json.
# Usage: subgraph_networks <networks_json_path>
subgraph_networks() {
  local networks_json="$1"
  jq -r 'keys[]' "$networks_json"
}

# Exit 0 when an Ormi subgraph query response is a deployed version.
# A missing name/version returns {"error":"..."}; a live one returns data._meta.
# Usage: ormi_query_is_deployed <json>
ormi_query_is_deployed() {
  local json="${1:-}"
  if [ -z "$json" ]; then
    return 1
  fi
  jq -e '.data._meta.block.number != null' <<<"$json" >/dev/null
}
