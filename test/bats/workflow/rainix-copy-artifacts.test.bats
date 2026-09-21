# Nothing in this repo executes rainix-copy-artifacts.yaml — it is
# `workflow_call` only, so its only runners are the consumer repos.
#
# The declaration check's verdict is covered by the Rust unit tests and
# test/bats/action/codegen-declaration.test.bats; what is asserted here is the
# wiring it cannot see: that the hooks' stdout actually reaches the log the
# action reads, and that teeing did not swallow a hook's exit status.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-copy-artifacts.yaml"
  action="$repo_root/.github/actions/codegen-declaration/action.yml"
  runs="$(yq -r '.jobs.copy-artifacts.steps[] | select(.run) | .run' "$workflow")"
  uses="$(yq -r '.jobs.copy-artifacts.steps[] | select(.uses) | .uses' "$workflow")"
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
  # The one path the two files have to agree on.
  log_path="$(yq -r '.runs.steps[0].run' "$action" | grep -o '\--log "[^"]*"' | sed 's/--log "//; s/"$//')"
  teed="$(yq -r '.jobs.copy-artifacts.steps[] | select(.run) | select(.run | contains("tee")) | .run' "$workflow")"
}

@test "the codegen declaration is checked, at the ref the check ships from" {
  echo "$uses" | grep -q '^rainlanguage/rainix/.github/actions/codegen-declaration@main$'
}

@test "the action reads a log under the runner temp dir, never the working tree" {
  [ -n "$log_path" ]
  # The literal the action script carries, not an expansion of it.
  # shellcheck disable=SC2016
  [[ "$log_path" == '$RUNNER_TEMP/'* ]]
}

@test "every codegen hook tees into the log the action reads" {
  local teed_steps occurrences
  teed_steps="$(yq -r '[.jobs.copy-artifacts.steps[] | select(.run) | select(.run | contains("tee -a"))] | length' "$workflow")"
  [ "$teed_steps" -eq 4 ]
  occurrences="$(grep -cF "tee -a \"$log_path\"" "$workflow")"
  [ "$occurrences" -eq 4 ]
}

@test "every codegen hook reaches the log — none is left undeclarable" {
  local hook
  for hook in script/build-meta.sh script/Build.sol script/CopyArtifacts.sol script/build.sh; do
    echo "$teed" | grep -qF "$hook"
  done
}

# Without pipefail bash reports tee's status, so a failing generator would pass
# the step it just failed.
@test "every teed step sets pipefail" {
  local n_teed n_pipefail
  n_teed="$(yq -r '[.jobs.copy-artifacts.steps[] | select(.run) | select(.run | contains("tee -a"))] | length' "$workflow")"
  n_pipefail="$(yq -r '[.jobs.copy-artifacts.steps[] | select(.run) | select(.run | contains("tee -a")) | select(.run | contains("set -euo pipefail"))] | length' "$workflow")"
  [ "$n_teed" -gt 0 ]
  [ "$n_pipefail" -eq "$n_teed" ]
}

@test "the declaration is checked after the last codegen hook and before the diff" {
  local last_hook check diff
  last_hook="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.run) | select(.value.run | contains("tee -a")) | .key] | max' "$workflow")"
  check="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.uses) | select(.value.uses | contains("codegen-declaration")) | .key] | .[0]' "$workflow")"
  diff="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.run) | select(.value.run | contains("git diff --exit-code")) | .key] | .[0]' "$workflow")"
  [ "$last_hook" -lt "$check" ]
  [ "$check" -lt "$diff" ]
}

@test "every rainix-copy-artifacts run step resolves rainix through the pinned sha" {
  [ -n "$sha" ]
  [ "$sha" != "null" ]
  local unpinned
  unpinned="$(echo "$runs" | grep 'github:rainlanguage/rainix' | grep -v 'env.RAINIX_SHA' || true)"
  if [ -n "$unpinned" ]; then
    echo "FAIL: unpinned rainix refs in rainix-copy-artifacts.yaml:" >&2
    echo "$unpinned" >&2
    return 1
  fi
}
