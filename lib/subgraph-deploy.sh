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
# <SUBGRAPH_NAME>-<network> at version label <address>-<build hash>, skipping a
# version Ormi already hosts. The build hash is `rainix-static
# subgraph-build-hash` over that network's `graph build` output, so a network
# whose build did not change keeps its label and is skipped: Ormi refuses an
# identical build under a second label. The deployment name and label keep the
# Goldsky-era <name>/<version> shape so an Ormi tag can move between versions.
#
# One network failing does not stop the others: a probe that cannot get a
# trustworthy answer (including a deployed version that is failed or still
# syncing), a failed build or a failed deploy is recorded, the loop moves on,
# and the task fails at the end naming every network that did not complete. A
# network whose build or probe failed is never deployed.
#
# The tools are arguments, not PATH lookups, so the task pins them to store
# paths and tests can substitute stubs.
# Usage: subgraph_deploy <graph> <npm> <rainix-static>
#
# Secret handling, since ORMI_DEPLOY_KEY is a credential:
#   - Tracing is switched off first. `set -x` (or `bash -x`) would print the key
#     the moment it is expanded, including in the presence check below.
#   - The key is moved out of the environment into a local before anything runs,
#     so jq, npm, the probe and graph-cli never inherit it.
#   - Only the hardcoded Ormi deploy endpoint ever receives the key. The
#     configurable ORMI_QUERY_BASE is used for the read-only probe alone.
#   - Not closed: `graph deploy` takes the key as an argument (no env or stdin
#     form), and it compiles the subgraph itself, so the subgraph's
#     AssemblyScript toolchain runs with the key visible in the process table.
#     Install scripts from `npm ci` also run on this runner, and one that leaves
#     a background process behind could read it. A subgraph repo's dependencies
#     are therefore trusted with this credential.
subgraph_deploy() {
  set +x
  local graph="$1" npm="$2" static="$3"
  local ormi_node="https://subgraph.api.ormilabs.com/deploy"
  local ormi_ipfs="https://subgraph.api.ormilabs.com/ipfs"

  subgraph_require_env ORMI_DEPLOY_KEY SUBGRAPH_NAME ORMI_QUERY_BASE || return 1
  local deploy_key="$ORMI_DEPLOY_KEY"
  unset ORMI_DEPLOY_KEY

  # subgraph/abis and subgraph/generated are committed, so the deploy compiles
  # the subgraph directly from them.
  (cd ./subgraph && "$npm" ci)

  local builds network address build hash version name rc
  local failed="" i=0
  builds="$(mktemp -d)"
  for network in $(subgraph_networks ./subgraph/networks.json); do
    address="$(subgraph_network_address ./subgraph/networks.json "$network")"
    name="${SUBGRAPH_NAME}-${network}"
    # Numbered, not named by network: a networks.json key is repo content and
    # must not pick a path.
    i=$((i + 1))
    build="$builds/$i"

    # The label is the digest of this network's own build, so an unchanged
    # build gets the label Ormi already hosts and is skipped below.
    if ! (cd ./subgraph && "$graph" build --network "$network" --output-dir "$build"); then
      echo "Build of $name failed; not deploying it, continuing with the other networks." >&2
      failed="$failed $name"
      continue
    fi
    if ! hash="$("$static" subgraph-build-hash "$build")"; then
      echo "Could not hash the build of $name; not deploying it, continuing with the other networks." >&2
      failed="$failed $name"
      continue
    fi
    version="$(subgraph_deploy_version "$address" "$hash")"

    # 0 = already deployed, 10 = confirmed missing. Anything else (the probe
    # could not get a trustworthy answer) must NOT fall through to a deploy.
    rc=0
    "$static" ormi-probe --base "$ORMI_QUERY_BASE" --name "$name" --version "$version" || rc=$?
    case "$rc" in
    0)
      echo "Subgraph $name/$version already deployed, skipping."
      ;;
    10)
      # graph deploy compiles the manifest itself, so it needs the network.
      # It recompiles into the hashed output dir; the compile is deterministic
      # (an unchanged build re-uploads to the same IPFS hash), so what it
      # deploys is the build the label names.
      echo "Deploying subgraph $name/$version..."
      if ! (cd ./subgraph && "$graph" deploy "$name" \
        --network "$network" \
        --output-dir "$build" \
        --node "$ormi_node" \
        --ipfs "$ormi_ipfs" \
        --deploy-key "$deploy_key" \
        --version-label "$version"); then
        echo "Deploy of $name/$version failed; continuing with the other networks." >&2
        failed="$failed $name"
      fi
      ;;
    *)
      echo "Could not determine whether $name/$version is deployed (probe exit $rc); not deploying it, continuing with the other networks." >&2
      failed="$failed $name"
      ;;
    esac
  done

  rm -rf -- "$builds"

  if [ -n "$failed" ]; then
    echo "Did not complete:$failed" >&2
    return 1
  fi
}
