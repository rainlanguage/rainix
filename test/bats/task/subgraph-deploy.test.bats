# subgraph_deploy (lib/subgraph-deploy.sh) with every tool stubbed: the stubs log
# what they were called with and what ORMI_DEPLOY_KEY they could see, so the
# tests pin both WHEN a deploy happens and WHERE the credential is visible.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  work="$(mktemp -d)"
  log="$work/calls.log"
  : >"$log"
  mkdir -p "$work/subgraph" "$work/bin"
  printf '%s' '{"mainnet":{"Counter":{"address":"0xabc","startBlock":0}}}' \
    >"$work/subgraph/networks.json"

  # Each stub records `<tool> <args> key=<visible ORMI_DEPLOY_KEY>`. graph
  # fails when it is a deploy for a name listed in $DEPLOY_FAILS.
  for tool in npm graph git; do
    cat >"$work/bin/$tool" <<'EOF'
#!/usr/bin/env bash
if [ "$(basename "$0")" = git ]; then echo abc1234; fi
echo "$(basename "$0") $* key=${ORMI_DEPLOY_KEY-unset}" >>"$CALLS_LOG"
case " ${DEPLOY_FAILS:-} " in *" $2 "*) exit 1 ;; esac
EOF
  done
  # The probe stub stands in for rainix-static: it exits with
  # PROBE_RC_<NETWORK> for that network, else with $PROBE_RC.
  cat >"$work/bin/rainix-static" <<'EOF'
#!/usr/bin/env bash
echo "probe $* key=${ORMI_DEPLOY_KEY-unset}" >>"$CALLS_LOG"
net="${5#raindex-}"
var="PROBE_RC_$(echo "$net" | tr a-z A-Z)"
exit "${!var:-${PROBE_RC:?}}"
EOF
  chmod +x "$work/bin/"*

  export CALLS_LOG="$log"
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
    subgraph_deploy "$2/graph" "$2/npm" "$2/git" "$2/rainix-static"
  ' _ "$repo_root" "$work/bin"
}

@test "a confirmed-missing version is deployed with its network, in one compile" {
  PROBE_RC=10 deploy
  [ "$status" -eq 0 ]
  grep -q "^probe ormi-probe --base $ORMI_QUERY_BASE --name raindex-mainnet --version 0xabc-abc1234 key=unset$" "$log"
  grep -q "^graph deploy raindex-mainnet --network mainnet --node https://subgraph.api.ormilabs.com/deploy --ipfs https://subgraph.api.ormilabs.com/ipfs --deploy-key $ORMI_DEPLOY_KEY --version-label 0xabc-abc1234 key=unset$" "$log"
  # graph deploy compiles the manifest itself; a separate build is a second,
  # redundant compile.
  [ "$(logged '^graph build')" -eq 0 ]
}

@test "an already-deployed version is skipped" {
  PROBE_RC=0 deploy
  [ "$status" -eq 0 ]
  [[ "$output" == *"already deployed, skipping"* ]]
  [ "$(logged '^graph ')" -eq 0 ]
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
    [ "$(logged '^graph ')" -eq 0 ]
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
