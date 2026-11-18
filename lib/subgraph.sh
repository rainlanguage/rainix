#!/usr/bin/env bash

# The deploy version label for a network: its label prefix (a networks.json
# address, or `manifest`; see `rainix-static subgraph-targets`) and the hash of
# its build (`rainix-static subgraph-build-hash`).
# Usage: subgraph_deploy_version <label-prefix> <build-hash>
subgraph_deploy_version() {
  local address="$1"
  local build_hash="$2"
  echo "${address}-${build_hash}"
}
