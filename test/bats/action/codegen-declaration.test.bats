setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/codegen-declaration/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
  log="$work/codegen.log"
}

teardown() {
  rm -rf "$work"
}

run_action() {
  RUNNER_TEMP="$1" \
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

@test "the action checks the log the workflow tees the hooks into" {
  run run_action /tmp/runner-temp

  [ "$status" -eq 0 ]
  [[ "$output" == *"<codegen-declaration> <--log> </tmp/runner-temp/rainix-codegen.log>" ]]
}

# Staged, not committed: git ls-files and git diff read the index.
mk_consumer() {
  git -C "$work" init -q
  mkdir -p "$work/src/generated" "$work/src/lib"
  printf 'GENERATED SNAPSHOT\n' >"$work/src/generated/Thing.sol"
  printf 'GENERATED AGGREGATE\n' >"$work/src/lib/LibReleasedSuites.sol"
  git -C "$work" add -A
}

# The issue's control/mutant pair as a generator: ownership is computed from the
# repo's contract list, so removing the CALL that emits a path leaves the `owns`
# line and drops the write and the `wrote` line. Output is indented the way
# `forge script` indents console.log under `== Logs ==`.
#
# $1, when given, is the path whose emitter was removed.
generate() {
  local dead="${1:-}" path
  echo "== Logs =="
  for path in src/generated/Thing.sol src/lib/LibReleasedSuites.sol; do
    printf '  rainix-codegen owns %s\n' "$path"
    if [ "$path" != "$dead" ]; then
      printf '%s\n' "$(cat "$work/$path")" >"$work/$path"
      printf '  rainix-codegen wrote %s\n' "$path"
    fi
  done
  echo "Script ran successfully."
}

@test "a live generator rewriting identical bytes is clean, and so is git diff" {
  mk_consumer
  generate >"$log"

  run rainix-static codegen-declaration --root "$work" --log "$log"
  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — 2 declared generated paths"* ]]

  run git -C "$work" diff --exit-code
  [ "$status" -eq 0 ]
}

@test "a generator that stopped emitting a file fails, though git diff is clean" {
  mk_consumer
  generate src/lib/LibReleasedSuites.sol >"$log"

  run rainix-static codegen-declaration --root "$work" --log "$log"
  [ "$status" -eq 1 ]
  [[ "$output" == *"src/lib/LibReleasedSuites.sol"* ]]
  [[ "$output" == *"emitter is dead"* ]]
  [[ "$output" != *"declare src/generated/Thing.sol generated, but nothing wrote"* ]]

  run git -C "$work" diff --exit-code
  [ "$status" -eq 0 ]
}

@test "a declared path that is not on disk at all is named as such" {
  mk_consumer
  rm "$work/src/lib/LibReleasedSuites.sol"
  generate src/lib/LibReleasedSuites.sol >"$log"

  run rainix-static codegen-declaration --root "$work" --log "$log"
  [ "$status" -eq 1 ]
  [[ "$output" == *"no such file exists"* ]]
}

@test "a hook that reports writing a path that never appeared fails" {
  mk_consumer
  printf 'rainix-codegen owns src/lib/Absent.sol\nrainix-codegen wrote src/lib/Absent.sol\n' >"$log"

  run rainix-static codegen-declaration --root "$work" --log "$log"
  [ "$status" -eq 1 ]
  [[ "$output" == *"reported writing src/lib/Absent.sol"* ]]
}

@test "a repo whose hooks declare nothing passes, and is told it is unprotected" {
  mk_consumer
  printf 'Script ran successfully.\n' >"$log"

  run rainix-static codegen-declaration --root "$work" --log "$log"
  [ "$status" -eq 0 ]
  [[ "$output" == *"declares no generated paths"* ]]
}

@test "a repo that ran no codegen hook at all passes" {
  mk_consumer

  run rainix-static codegen-declaration --root "$work" --log "$work/never-written.log"
  [ "$status" -eq 0 ]
  [[ "$output" == *"declares no generated paths"* ]]
}

@test "a path written but not declared is a note, not a failure" {
  mk_consumer
  printf 'rainix-codegen owns src/generated/Thing.sol\nrainix-codegen wrote src/generated/Thing.sol\nrainix-codegen wrote soldeer.lock\n' >"$log"
  printf 'x\n' >"$work/soldeer.lock"

  run rainix-static codegen-declaration --root "$work" --log "$log"
  [ "$status" -eq 0 ]
  [[ "$output" == *"note — soldeer.lock was written but not declared"* ]]
}

@test "a verb this check does not know fails rather than being ignored" {
  mk_consumer
  printf 'rainix-codegen skipped src/generated/Thing.sol\n' >"$log"

  run rainix-static codegen-declaration --root "$work" --log "$log"
  [ "$status" -eq 1 ]
  [[ "$output" == *"unreadable declaration"* ]]
}
