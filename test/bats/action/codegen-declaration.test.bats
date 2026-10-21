setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/codegen-declaration/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
  log="$work/codegen.log"
  state="$(mktemp)"
}

teardown() {
  rm -rf "$work" "$state"
}

run_action() {
  RUNNER_TEMP="$1" \
    RAINIX_CODEGEN_PHASE="$2" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/codegen-declaration" \
    ACTION_SCRIPT="$action_script" \
    bash -c '
      nix() {
        printf "nix"
        printf " <%s>" "$@"
        printf "\n"
      }
      export -f nix
      bash -c "$ACTION_SCRIPT"
    '
}

@test "the action passes its phase, state and the log the workflow tees into" {
  run run_action /tmp/runner-temp verify

  [ "$status" -eq 0 ]
  [[ "$output" == *"<codegen-declaration> <verify> <--state> </tmp/runner-temp/rainix-codegen-mtimes.json> <--log> </tmp/runner-temp/rainix-codegen.log>" ]]
}

@test "the action's phase input reaches the script" {
  # shellcheck disable=SC2016
  [ "$(yq -r '.runs.steps[0].env.RAINIX_CODEGEN_PHASE' "$action")" = '${{ inputs.phase }}' ]
}

# Staged, not committed: git ls-files reads the index. mtimes are set in the
# past so a write during the test always moves them.
mk_consumer() {
  git -C "$work" init -q
  mkdir -p "$work/src/generated" "$work/src/lib"
  printf 'GENERATED SNAPSHOT\n' >"$work/src/generated/Thing.sol"
  printf 'GENERATED AGGREGATE\n' >"$work/src/lib/LibReleasedSuites.sol"
  printf 'HAND WRITTEN\n' >"$work/src/lib/Hand.sol"
  git -C "$work" add -A
  touch -d '2000-01-01' "$work/src/generated/Thing.sol" "$work/src/lib/LibReleasedSuites.sol" "$work/src/lib/Hand.sol"
}

mark() {
  rainix-static codegen-declaration mark --root "$work" --state "$state" >/dev/null
}

verify() {
  run rainix-static codegen-declaration verify --root "$work" --state "$state" --log "${1:-$log}"
}

# The issue's control/mutant pair as a generator: ownership is computed from the
# repo's contract list, so removing the CALL that emits a path leaves its `owns`
# line and drops only the write. Output is indented the way `forge script`
# indents console.log under `== Logs ==`.
#
# $1, when given, is the path whose emitter was removed.
generate() {
  local dead="${1:-}" path
  echo "== Logs =="
  for path in src/generated/Thing.sol src/lib/LibReleasedSuites.sol; do
    printf '  rainix-codegen owns %s\n' "$path"
    if [ "$path" != "$dead" ]; then
      printf '%s\n' "$(cat "$work/$path")" >"$work/$path"
    fi
  done
  echo "Script ran successfully."
}

@test "a live generator rewriting identical bytes is clean, and so is git diff" {
  mk_consumer
  mark
  generate >"$log"

  verify
  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — 2 declared generated paths"* ]]

  run git -C "$work" diff --exit-code
  [ "$status" -eq 0 ]
}

@test "a generator that stopped emitting a file fails, though git diff is clean" {
  mk_consumer
  mark
  generate src/lib/LibReleasedSuites.sol >"$log"

  verify
  [ "$status" -eq 1 ]
  [[ "$output" == *"src/lib/LibReleasedSuites.sol"* ]]
  [[ "$output" == *"emitter is dead"* ]]
  [[ "$output" != *"declare src/generated/Thing.sol generated, but nothing wrote"* ]]

  run git -C "$work" diff --exit-code
  [ "$status" -eq 0 ]
}

@test "a generator claiming a write it did not make cannot pass" {
  mk_consumer
  mark
  generate src/lib/LibReleasedSuites.sol >"$log"
  printf '  rainix-codegen wrote src/lib/LibReleasedSuites.sol\n' >>"$log"

  verify
  [ "$status" -eq 1 ]
  [[ "$output" == *"unreadable declaration: rainix-codegen wrote src/lib/LibReleasedSuites.sol"* ]]
}

@test "a declared path that is not on disk at all is named as such" {
  mk_consumer
  mark
  rm "$work/src/lib/LibReleasedSuites.sol"
  generate src/lib/LibReleasedSuites.sol >"$log"

  verify
  [ "$status" -eq 1 ]
  [[ "$output" == *"no such file exists"* ]]
}

@test "a declared path the run created is written" {
  mk_consumer
  mark
  printf 'NEW\n' >"$work/src/lib/New.sol"
  printf 'rainix-codegen owns src/lib/New.sol\n' >"$log"

  verify
  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — 1 declared generated paths"* ]]
}

@test "a tracked file written but not declared is a note, not a failure" {
  mk_consumer
  mark
  generate >"$log"
  printf 'touched\n' >>"$work/src/lib/Hand.sol"

  verify
  [ "$status" -eq 0 ]
  [[ "$output" == *"note — src/lib/Hand.sol was written but not declared"* ]]
}

@test "a repo whose hooks declare nothing passes, and is told it is unprotected" {
  mk_consumer
  mark
  printf 'Script ran successfully.\n' >"$log"

  verify
  [ "$status" -eq 0 ]
  [[ "$output" == *"declares no generated paths"* ]]
}

@test "a repo that ran no codegen hook at all passes" {
  mk_consumer
  mark

  verify "$work/never-written.log"
  [ "$status" -eq 0 ]
  [[ "$output" == *"declares no generated paths"* ]]
}

@test "verify without a mark fails rather than calling everything written" {
  mk_consumer
  rm -f "$state"
  generate >"$log"

  verify
  [ "$status" -eq 1 ]
  [[ "$output" == *"must run before the codegen hooks"* ]]
}

@test "a verb this check does not know fails rather than being ignored" {
  mk_consumer
  mark
  printf 'rainix-codegen skipped src/generated/Thing.sol\n' >"$log"

  verify
  [ "$status" -eq 1 ]
  [[ "$output" == *"unreadable declaration"* ]]
}

@test "a declared path outside the repo fails even though the file exists" {
  mk_consumer
  mark
  outside="$(mktemp)"
  printf 'rainix-codegen owns %s\n' "$outside" >"$log"

  verify
  rm -f "$outside"
  [ "$status" -eq 1 ]
  [[ "$output" == *"unreadable declaration"* ]]
}

@test "an unknown phase fails" {
  run rainix-static codegen-declaration audit --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"phase must be"* ]]
}
