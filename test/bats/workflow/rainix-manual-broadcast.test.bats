# rainix-manual-broadcast.yaml is `workflow_call` only and broadcasts with a
# live key, so nothing here runs it; its broadcast step runs with nix stubbed.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-manual-broadcast.yaml"
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
  broadcast="$(yq -r '.jobs.broadcast.steps[] | select(.name == "Broadcast script") | .run' "$workflow")"
}

# The broadcast step as bash runs it, with every expression stubbed and nix
# logging its arguments to $BATS_TEST_TMPDIR/calls.
run_broadcast() {
  local script="$broadcast"
  while [[ "$script" =~ \$\{\{[^}]*\}\} ]]; do
    script="${script/"${BASH_REMATCH[0]}"/x}"
  done
  NETWORK="$1" LEGACY="$2" RPC_THROTTLE=false SCRIPT=s SIG='run()' PRIVATE_KEY=k \
    CALLS="$BATS_TEST_TMPDIR/calls" BROADCAST_SCRIPT="$script" bash -c '
      nix() { printf "%s\n" "$*" >>"$CALLS"; }
      export -f nix
      bash -c "$BROADCAST_SCRIPT"
    '
}

@test "the workflow can only be called, never triggered" {
  [ "$(yq -r '.on | keys | join(",")' "$workflow")" = "workflow_call" ]
}

@test "every rainix ref in a run step resolves through the pinned sha" {
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]]
  local unpinned
  unpinned="$(yq -r '.jobs.broadcast.steps[] | select(.run) | .run' "$workflow" | grep 'github:rainlanguage/rainix' | grep -v 'env.RAINIX_SHA' || true)"
  [ -z "$unpinned" ]
}

@test "the legacy input sends --legacy" {
  run_broadcast base true
  grep -q -- "--legacy" "$BATS_TEST_TMPDIR/calls"
}

@test "no network sends --legacy without the legacy input" {
  run_broadcast hyperevm false
  grep -q -- "--broadcast" "$BATS_TEST_TMPDIR/calls"
  run grep -c -- "--legacy" "$BATS_TEST_TMPDIR/calls"
  [ "$output" = 0 ]
}
