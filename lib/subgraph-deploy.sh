#!/usr/bin/env bash

# Orchestration for the `subgraph-deploy` task. Every decision that reads data
# (is this version live? is the name safe in a URL?) is made by
# `rainix-static subgraph-probe`; this file only wires steps together.
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

# Deploy every network of the subgraph in $SUBGRAPH_DIR (default `subgraph`;
# see `rainix-static subgraph-targets`) to that network's host (Ormi, or
# Goldsky for a network Ormi does not index) as <SUBGRAPH_NAME>-<network> at
# version label <address or `manifest`>-<build hash>, skipping a version the
# host already serves. The build hash is `rainix-static subgraph-build-hash`
# over that network's `graph build` output, so a network whose build did not
# change keeps its label and is skipped: Ormi refuses an identical build under
# a second label, and Goldsky refuses a version that exists.
#
# Each host's settings are required only when a target deploys there: Ormi
# needs ORMI_DEPLOY_KEY and ORMI_QUERY_BASE, Goldsky needs GOLDSKY_TOKEN and
# GOLDSKY_QUERY_BASE (`https://api.goldsky.com/api/public/<project id>`).
#
# One network failing does not stop the others: a probe that cannot get a
# trustworthy answer (including a deployed version that is failed or still
# syncing), a failed build or a failed deploy is recorded, the loop moves on,
# and the task fails at the end naming every network that did not complete. A
# network whose build or probe failed is never deployed.
#
# The tools are arguments, not PATH lookups, so the task pins them to store
# paths and tests can substitute stubs.
# Usage: subgraph_deploy <graph> <npm> <rainix-static> <goldsky>
#
# Secret handling, since ORMI_DEPLOY_KEY and GOLDSKY_TOKEN are credentials:
#   - Tracing is switched off first. `set -x` (or `bash -x`) would print a
#     credential the moment it is expanded, including in the presence check.
#   - Both are moved out of the environment into locals before anything runs,
#     so npm, the probe, graph-cli and the Goldsky CLI never inherit them.
#   - Each credential goes only to its own host's deploy: the Ormi key to the
#     hardcoded Ormi deploy endpoint, the Goldsky token to the Goldsky CLI. The
#     configurable query bases are used for the read-only probe alone.
#   - Not closed: `graph deploy` and `goldsky subgraph deploy` take the
#     credential as an argument (no env or stdin form), so it is visible in the
#     process table while they run, and `graph deploy` compiles the subgraph
#     itself, so the subgraph's AssemblyScript toolchain runs alongside it.
#     Install scripts from `npm ci` also run on this runner, and one that leaves
#     a background process behind could read it. A subgraph repo's dependencies
#     are therefore trusted with these credentials.
subgraph_deploy() {
  set +x
  local graph="$1" npm="$2" static="$3" goldsky="$4"
  local ormi_node="https://subgraph.api.ormilabs.com/deploy"
  local ormi_ipfs="https://subgraph.api.ormilabs.com/ipfs"

  subgraph_require_env SUBGRAPH_NAME || return 1
  local deploy_key="${ORMI_DEPLOY_KEY:-}" goldsky_token="${GOLDSKY_TOKEN:-}"
  unset ORMI_DEPLOY_KEY GOLDSKY_TOKEN

  # rainix-static refuses a SUBGRAPH_DIR that is not one plain name, so $dir is
  # safe to cd into once it has answered.
  local dir="${SUBGRAPH_DIR:-subgraph}" targets
  targets="$("$static" subgraph-targets "$dir")" || return 1

  # The credentials were unset above, so check the locals by name.
  case "$targets" in
  *$'\tormi\t'*)
    if [ -z "$deploy_key" ]; then
      echo "ORMI_DEPLOY_KEY is required" >&2
      return 1
    fi
    subgraph_require_env ORMI_QUERY_BASE || return 1
    ;;
  esac
  case "$targets" in
  *$'\tgoldsky\t'*)
    if [ -z "$goldsky_token" ]; then
      echo "GOLDSKY_TOKEN is required" >&2
      return 1
    fi
    subgraph_require_env GOLDSKY_QUERY_BASE || return 1
    ;;
  esac

  # <dir>/abis and <dir>/generated are committed, so the deploy compiles the
  # subgraph directly from them.
  (cd "./$dir" && "$npm" ci)

  local builds network host manifest prefix build hash version name rc base
  local -a source
  local failed="" i=0
  builds="$(mktemp -d)"
  while IFS=$'\t' read -r network host prefix manifest <&3; do
    name="${SUBGRAPH_NAME}-${network}"
    # A networks.json subgraph builds subgraph.yaml for the network; a
    # manifests one builds that network's own manifest.
    if [ -z "$manifest" ]; then
      source=(--network "$network")
    else
      source=("$manifest")
    fi
    # Numbered, not named by network: a network name is repo content and
    # must not pick a path.
    i=$((i + 1))
    build="$builds/$i"

    # The label is the digest of this network's own build, so an unchanged
    # build gets the label its host already serves and is skipped below.
    if ! (cd "./$dir" && "$graph" build "${source[@]}" --output-dir "$build"); then
      echo "Build of $name failed; not deploying it, continuing with the other networks." >&2
      failed="$failed $name"
      continue
    fi
    if ! hash="$("$static" subgraph-build-hash "$build")"; then
      echo "Could not hash the build of $name; not deploying it, continuing with the other networks." >&2
      failed="$failed $name"
      continue
    fi
    version="$(subgraph_deploy_version "$prefix" "$hash")"

    if [ "$host" = goldsky ]; then
      base="$GOLDSKY_QUERY_BASE"
    else
      base="$ORMI_QUERY_BASE"
    fi
    # 0 = already deployed, 10 = confirmed missing. Anything else (the probe
    # could not get a trustworthy answer) must NOT fall through to a deploy.
    rc=0
    "$static" subgraph-probe --host "$host" --base "$base" --name "$name" --version "$version" || rc=$?
    case "$rc" in
    0)
      echo "Subgraph $name/$version already deployed, skipping."
      ;;
    10)
      echo "Deploying subgraph $name/$version to $host..."
      if [ "$host" = goldsky ]; then
        # Goldsky deploys the build as built, so it is exactly what the label
        # names.
        if ! (cd "./$dir" && "$goldsky" --token "$goldsky_token" \
          subgraph deploy "$name/$version" --path "$build"); then
          echo "Deploy of $name/$version failed; continuing with the other networks." >&2
          failed="$failed $name"
        fi
      # graph deploy compiles the manifest itself, so it needs the network.
      # It recompiles into the hashed output dir; the compile is deterministic
      # (an unchanged build re-uploads to the same IPFS hash), so what it
      # deploys is the build the label names.
      elif ! (cd "./$dir" && "$graph" deploy "$name" "${source[@]}" \
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
  done 3<<<"$targets"

  rm -rf -- "$builds"

  if [ -n "$failed" ]; then
    echo "Did not complete:$failed" >&2
    return 1
  fi
}
