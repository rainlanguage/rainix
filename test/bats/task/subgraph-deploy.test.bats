# subgraph_deploy (lib/subgraph-deploy.sh) with graph, npm and the Ormi probe
# stubbed: the stubs log what they were called with and what ORMI_DEPLOY_KEY
# they could see, so the tests pin both WHEN a deploy happens and WHERE the
# credential is visible. The build hash is the real
# `rainix-static subgraph-build-hash`, over what the graph stub "builds".

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  work="$(mktemp -d)"
  log="$work/calls.log"
  hosted="$work/hosted"
  : >"$log"
  : >"$hosted"
  mkdir -p "$work/subgraph" "$work/bin"
  printf '%s' '{"mainnet":{"Counter":{"address":"0xabc","startBlock":0}}}' \
    >"$work/subgraph/networks.json"

  REAL_STATIC="$(command -v rainix-static)"
  export REAL_STATIC

  # Each stub records `<tool> <args> key=<visible ORMI_DEPLOY_KEY>`.
  cat >"$work/bin/npm" <<'EOF'
#!/usr/bin/env bash
echo "npm $* key=${ORMI_DEPLOY_KEY-unset}" >>"$CALLS_LOG"
EOF
  # graph build writes "$BUILD_<NETWORK>" (default: one fixed text per network)
  # as the build output, writes nothing for a network listed in $BUILD_EMPTY,
  # and fails for one listed in $BUILD_FAILS. graph
  # deploy fails for a name listed in $DEPLOY_FAILS and otherwise records
  # <name>/<label> as hosted, as Ormi would.
  cat >"$work/bin/graph" <<'EOF'
#!/usr/bin/env bash
echo "graph $* key=${ORMI_DEPLOY_KEY-unset}" >>"$CALLS_LOG"
cmd="$1" name="$2"
while [ $# -gt 0 ]; do
  case "$1" in
  --network) network="$2" ;;
  --output-dir) out="$2" ;;
  --version-label) label="$2" ;;
  esac
  shift
done
case "$cmd" in
build)
  case " ${BUILD_FAILS:-} " in *" $network "*) exit 1 ;; esac
  var="BUILD_$(echo "$network" | tr a-z A-Z)"
  mkdir -p "$out"
  case " ${BUILD_EMPTY:-} " in *" $network "*) exit 0 ;; esac
  printf '%s' "${!var:-build of $network}" >"$out/subgraph.yaml"
  ;;
deploy)
  case " ${DEPLOY_FAILS:-} " in *" $name "*) exit 1 ;; esac
  echo "$name/$label" >>"$HOSTED"
  ;;
esac
EOF
  # The rainix-static stub runs the real subgraph-build-hash. ormi-probe exits
  # with PROBE_RC_<NETWORK> for that network, else $PROBE_RC, else answers like
  # Ormi from what graph deploy recorded: 0 hosted, 10 missing.
  cat >"$work/bin/rainix-static" <<'EOF'
#!/usr/bin/env bash
if [ "$1" = subgraph-build-hash ]; then
  echo "hash $* key=${ORMI_DEPLOY_KEY-unset}" >>"$CALLS_LOG"
  exec "$REAL_STATIC" "$@"
fi
echo "probe $* key=${ORMI_DEPLOY_KEY-unset}" >>"$CALLS_LOG"
net="${5#raindex-}"
var="PROBE_RC_$(echo "$net" | tr a-z A-Z)"
rc="${!var:-${PROBE_RC:-}}"
if [ -n "$rc" ]; then exit "$rc"; fi
if grep -qxF "$5/$7" "$HOSTED"; then exit 0; fi
exit 10
EOF
  chmod +x "$work/bin/"*

  export CALLS_LOG="$log"
  export HOSTED="$hosted"
  export ORMI_DEPLOY_KEY="s3cr3t-deploy-key-do-not-print"
  export SUBGRAPH_NAME="raindex"
  export ORMI_QUERY_BASE="https://example.invalid/api/public/id"
  cd "$work" || exit
}

teardown() {
  rm -rf "$work"
}

# Number of log lines matching a pattern (0 when none; a bare `! grep` would
# not fail a bats test).
logged() {
  grep -c "$1" "$log" || true
}

two_networks() {
  printf '%s' '{"base":{"C":{"address":"0xbbb"}},"mainnet":{"C":{"address":"0xabc"}}}' \
    >"$work/subgraph/networks.json"
}

# Run subgraph_deploy in a fresh bash, optionally under `bash -x`.
deploy() {
  local flags="${1:-}"
  # shellcheck disable=SC2086
  run bash $flags -c '
    source "$1/lib/subgraph.sh"
    source "$1/lib/subgraph-deploy.sh"
    subgraph_deploy "$2/graph" "$2/npm" "$2/rainix-static"
  ' _ "$repo_root" "$work/bin"
}

# The label subgraph_deploy must use for a build whose output is $2.
label_for() {
  local dir
  dir="$(mktemp -d)"
  printf '%s' "$2" >"$dir/subgraph.yaml"
  echo "$1-$("$REAL_STATIC" subgraph-build-hash "$dir")"
  rm -rf "$dir"
}

@test "a confirmed-missing version is built, labelled by its build and deployed" {
  PROBE_RC=10 deploy
  [ "$status" -eq 0 ]
  version="$(label_for 0xabc "build of mainnet")"
  [[ "$version" =~ ^0xabc-[0-9a-f]{16}$ ]]
  out="$(sed -n 's/^graph build --network mainnet --output-dir \([^ ]*\) key=unset$/\1/p' "$log")"
  [ -n "$out" ]
  grep -q "^hash subgraph-build-hash $out key=unset$" "$log"
  grep -q "^probe ormi-probe --base $ORMI_QUERY_BASE --name raindex-mainnet --version $version key=unset$" "$log"
  grep -q "^graph deploy raindex-mainnet --network mainnet --output-dir $out --node https://subgraph.api.ormilabs.com/deploy --ipfs https://subgraph.api.ormilabs.com/ipfs --deploy-key $ORMI_DEPLOY_KEY --version-label $version key=unset$" "$log"
  # build, hash, probe, deploy: in that order.
  [ "$(sed 's/ .*//' "$log" | tr '\n' ' ')" = "npm graph hash probe graph " ]
  # The build dir does not outlive the task.
  [ ! -e "$out" ]
}

@test "an already-deployed version is skipped" {
  PROBE_RC=0 deploy
  [ "$status" -eq 0 ]
  [[ "$output" == *"already deployed, skipping"* ]]
  [ "$(logged '^graph deploy')" -eq 0 ]
}

@test "an unchanged build is skipped on the next run" {
  deploy
  [ "$status" -eq 0 ]
  [ "$(logged '^graph deploy raindex-mainnet ')" -eq 1 ]
  # Nothing the build depends on changed (a new repo commit, say): the label is
  # the one Ormi already hosts, so there is nothing to deploy. A label that
  # differs here is a deploy Ormi refuses as a duplicate.
  : >"$log"
  deploy
  [ "$status" -eq 0 ]
  [[ "$output" == *"Subgraph raindex-mainnet/$(label_for 0xabc "build of mainnet") already deployed, skipping."* ]]
  [ "$(logged '^graph deploy')" -eq 0 ]
}

@test "a changed build is deployed under a new label; unchanged networks are skipped" {
  two_networks
  deploy
  [ "$status" -eq 0 ]
  [ "$(logged '^graph deploy')" -eq 2 ]
  : >"$log"
  BUILD_BASE="base, new mapping" deploy
  [ "$status" -eq 0 ]
  [ "$(logged '^graph deploy')" -eq 1 ]
  grep -q "^graph deploy raindex-base .* --version-label $(label_for 0xbbb "base, new mapping") key=unset$" "$log"
  [[ "$output" == *"Subgraph raindex-mainnet/$(label_for 0xabc "build of mainnet") already deployed, skipping."* ]]
}

@test "the label follows the address and the build, nothing else" {
  two_networks
  a="$(label_for 0xabc same)"
  [ "$a" != "$(label_for 0xbbb same)" ]
  [ "$a" != "$(label_for 0xabc other)" ]
  BUILD_BASE=same BUILD_MAINNET=same PROBE_RC=10 deploy
  [ "$status" -eq 0 ]
  grep -q "^graph deploy raindex-base .* --version-label $(label_for 0xbbb same) key=unset$" "$log"
  grep -q "^graph deploy raindex-mainnet .* --version-label $a key=unset$" "$log"
}

@test "a failed build fails the task, is never probed or deployed, and does not stop the others" {
  two_networks
  BUILD_FAILS="base" PROBE_RC=10 deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"Build of raindex-base failed"* ]]
  [[ "$output" == *"Did not complete: raindex-base"* ]]
  [ "$(logged '^probe .*--name raindex-base ')" -eq 0 ]
  [ "$(logged '^graph deploy raindex-base ')" -eq 0 ]
  [ "$(logged '^graph deploy raindex-mainnet ')" -eq 1 ]
}

@test "a build that cannot be hashed fails the task and is never probed or deployed" {
  # An empty build output is refused by subgraph-build-hash.
  BUILD_EMPTY="mainnet" PROBE_RC=10 deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"Could not hash the build of raindex-mainnet"* ]]
  [[ "$output" == *"Did not complete: raindex-mainnet"* ]]
  [ "$(logged '^probe ')" -eq 0 ]
  [ "$(logged '^graph deploy')" -eq 0 ]
}

@test "a failed probe fails the task and never deploys" {
  # 1 is what ormi-probe exits for transport/HTTP failure, an unrecognised body
  # or a wrong query base; the rest guard against any other status being
  # mistaken for "missing".
  for rc in 1 2 7 101 127; do
    : >"$log"
    PROBE_RC=$rc deploy
    [ "$status" -ne 0 ]
    [[ "$output" == *"not deploying"* ]]
    [[ "$output" == *"Did not complete: raindex-mainnet"* ]]
    [ "$(logged '^graph deploy')" -eq 0 ]
  done
}

@test "one network failing its probe does not stop the others" {
  two_networks
  PROBE_RC_BASE=1 PROBE_RC_MAINNET=10 deploy
  # The task still fails, naming the network that did not complete...
  [ "$status" -ne 0 ]
  [[ "$output" == *"Did not complete: raindex-base"* ]]
  # ...but the failed one was never deployed and the healthy one was.
  [ "$(logged '^graph deploy raindex-base ')" -eq 0 ]
  [ "$(logged '^graph deploy raindex-mainnet ')" -eq 1 ]
}

@test "a failed network after a good one still reports and deploys the rest" {
  two_networks
  PROBE_RC_BASE=10 PROBE_RC_MAINNET=1 deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"Did not complete: raindex-mainnet"* ]]
  [ "$(logged '^graph deploy raindex-base ')" -eq 1 ]
}

@test "a failed deploy does not stop the other networks and fails the task" {
  two_networks
  DEPLOY_FAILS="raindex-base" PROBE_RC=10 deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"Did not complete: raindex-base"* ]]
  [ "$(logged '^graph deploy raindex-mainnet ')" -eq 1 ]
}

@test "every network that did not complete is named" {
  printf '%s' '{"a":{"C":{"address":"0x1"}},"b":{"C":{"address":"0x2"}},"c":{"C":{"address":"0x3"}},"d":{"C":{"address":"0x4"}}}' \
    >"$work/subgraph/networks.json"
  BUILD_FAILS="a" PROBE_RC_B=1 DEPLOY_FAILS="raindex-c" PROBE_RC_C=10 PROBE_RC_D=10 deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"Did not complete: raindex-a raindex-b raindex-c"* ]]
  [ "$(logged '^graph deploy raindex-d ')" -eq 1 ]
}

@test "the deploy key is only ever an argument of graph deploy" {
  PROBE_RC=10 deploy
  [ "$status" -eq 0 ]
  # Every tool, graph deploy included, sees an empty environment for it...
  [ "$(logged 'key=unset$')" -eq "$(wc -l <"$log" | tr -d ' ')" ]
  # ...and the only place the value appears is graph deploy's --deploy-key.
  [ "$(logged 's3cr3t')" -eq 1 ]
  [ "$(logged '^graph deploy .*--deploy-key s3cr3t')" -eq 1 ]
}

@test "the deploy key never appears in output, even under bash -x" {
  for mode in "" "-x"; do
    for rc in 10 0 1; do
      PROBE_RC=$rc deploy "$mode"
      [[ "$output" != *"$ORMI_DEPLOY_KEY"* ]]
    done
  done
}

@test "a missing deploy key fails before any tool runs" {
  unset ORMI_DEPLOY_KEY
  PROBE_RC=10 deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"ORMI_DEPLOY_KEY is required"* ]]
  [ ! -s "$log" ]
}

@test "missing name or query base fails before any tool runs" {
  for var in SUBGRAPH_NAME ORMI_QUERY_BASE; do
    : >"$log"
    (
      unset "$var"
      PROBE_RC=10 deploy
      [ "$status" -ne 0 ]
      [[ "$output" == *"$var is required"* ]]
    )
    [ ! -s "$log" ]
  done
}
