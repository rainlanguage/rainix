#!/usr/bin/env bash

# Orchestration for the `subgraph-deploy` task. Every decision that reads data
# (is this version live? is the name safe in a URL?) is made by
# `rainix-static ormi-probe`; this file only wires steps together.
# Requires lib/subgraph.sh to be sourced first.

# Fail (naming the variable, never its value) unless every named env var is set
# and non-empty.
# Usage: subgraph_require_env <VAR>...
subgraph_require_env() {
  local var
  for var in "$@"; do
    if [ -z "${!var:-}" ]; then
      echo "$var is required" >&2
      return 1
    fi
  done
}

# Deploy every network in subgraph/networks.json to Ormi as
# <SUBGRAPH_NAME>-<network> at version label <address>-<commit>, skipping a
# version Ormi already hosts. The deployment name and label keep the Goldsky-era
# shape so an Ormi tag can move between versions.
#
# The tools are arguments, not PATH lookups, so the task pins them to store
# paths and tests can substitute stubs.
# Usage: subgraph_deploy <graph> <npm> <git> <rainix-static>
#
# Secret handling, since ORMI_DEPLOY_KEY is a credential:
#   - Tracing is switched off first. `set -x` (or `bash -x`) would print the key
#     the moment it is expanded, including in the presence check below.
#   - The key is removed from the environment of everything but `graph deploy`.
#     `npm ci` runs dependency install scripts and `graph build` runs the
#     subgraph's own toolchain; none of git, the probe, npm or build has any
#     use for it.
#   - Only the hardcoded Ormi deploy endpoint ever receives the key. The
#     configurable ORMI_QUERY_BASE is used for the read-only probe alone.
#   - `graph deploy` takes the key as an argument (no env or stdin form exists),
#     so it is visible in the process table of the runner for that one command.
subgraph_deploy() {
  set +x
  local graph="$1" npm="$2" git="$3" static="$4"
  local ormi_node="https://subgraph.api.ormilabs.com/deploy"
  local ormi_ipfs="https://subgraph.api.ormilabs.com/ipfs"

  subgraph_require_env ORMI_DEPLOY_KEY SUBGRAPH_NAME ORMI_QUERY_BASE || return 1

  # subgraph/abis and subgraph/generated are committed, so the deploy builds
  # the subgraph directly from them.
  (cd ./subgraph && env -u ORMI_DEPLOY_KEY "$npm" ci)

  local commit network address version name rc
  commit="$(env -u ORMI_DEPLOY_KEY "$git" rev-parse --short HEAD)"
  for network in $(subgraph_networks ./subgraph/networks.json); do
    address="$(subgraph_network_address ./subgraph/networks.json "$network")"
    version="$(subgraph_deploy_version "$address" "$commit")"
    name="${SUBGRAPH_NAME}-${network}"

    # 0 = already deployed, 10 = confirmed missing. Anything else (the probe
    # could not get a trustworthy answer) must NOT fall through to a deploy.
    rc=0
    env -u ORMI_DEPLOY_KEY "$static" ormi-probe --base "$ORMI_QUERY_BASE" --name "$name" --version "$version" || rc=$?
    case "$rc" in
    0)
      echo "Subgraph $name/$version already deployed, skipping."
      ;;
    10)
      echo "Building subgraph for $network..."
      (cd ./subgraph && env -u ORMI_DEPLOY_KEY "$graph" build --network "$network")
      echo "Deploying subgraph $name/$version..."
      (cd ./subgraph && "$graph" deploy "$name" \
        --node "$ormi_node" \
        --ipfs "$ormi_ipfs" \
        --deploy-key "$ORMI_DEPLOY_KEY" \
        --version-label "$version")
      ;;
    *)
      echo "Could not determine whether $name/$version is deployed (probe exit $rc); not deploying." >&2
      return 1
      ;;
    esac
  done
}
