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
  local n
  n="$(echo "$uses" | grep -c '^rainlanguage/rainix/.github/actions/codegen-declaration@main$')"
  [ "$n" -eq 2 ]
}

@test "tracked files are marked before the first codegen hook" {
  local first_hook mark
  first_hook="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.run) | select(.value.run | contains("tee -a")) | .key] | min' "$workflow")"
  mark="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.uses) | select(.value.uses | contains("codegen-declaration")) | select(.value.with.phase == "mark") | .key] | .[0]' "$workflow")"
  [ "$mark" != "null" ]
  [ "$mark" -lt "$first_hook" ]
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
  for hook in script/build-meta.sh script/Build.sol script/CopyArtifacts.sol REGENERATE_COMMAND; do
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

# Before fmt: fmt rewrites sources of its own, so a file it touched would look
# like one a generator wrote.
@test "the declaration is verified after the last codegen hook and before forge fmt" {
  local last_hook check fmt
  last_hook="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.run) | select(.value.run | contains("tee -a")) | .key] | max' "$workflow")"
  check="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.uses) | select(.value.uses | contains("codegen-declaration")) | select(.value.with.phase == "verify") | .key] | .[0]' "$workflow")"
  fmt="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.run) | select(.value.run | contains("forge fmt")) | .key] | .[0]' "$workflow")"
  [ "$check" != "null" ]
  [ "$last_hook" -lt "$check" ]
  [ "$check" -lt "$fmt" ]
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

@test "staged config is installed after Build.sol stages it and before anything reads it" {
  local build install forge_build verify
  build="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.run) | select(.value.run | contains("forge script ./script/Build.sol")) | .key] | .[0]' "$workflow")"
  install="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.uses == "rainlanguage/rainix/.github/actions/install-staged-config@main") | .key] | .[0]' "$workflow")"
  forge_build="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.run) | select(.value.run | test("-c forge build$")) | .key] | .[0]' "$workflow")"
  verify="$(yq -r '[.jobs.copy-artifacts.steps | to_entries[] | select(.value.uses) | select(.value.uses | contains("codegen-declaration")) | select(.value.with.phase == "verify") | .key] | .[0]' "$workflow")"
  [ "$install" != "null" ]
  [ "$(yq -r ".jobs.copy-artifacts.steps[$install].if" "$workflow")" = "null" ]
  [ "$build" -lt "$install" ]
  [ "$install" -lt "$forge_build" ]
  [ "$install" -lt "$verify" ]
}

@test "derived artifacts regenerate from the declared input, never a file the repo happens to have" {
  local step
  step="$(yq -r '.jobs.copy-artifacts.steps[] | select(.name == "Regenerate derived artifacts")' "$workflow")"
  [ "$(echo "$step" | yq -r '.if')" = "inputs.regenerate-command != ''" ]
  # shellcheck disable=SC2016
  [ "$(echo "$step" | yq -r '.env.REGENERATE_COMMAND')" = '${{ inputs.regenerate-command }}' ]
  [ "$(yq -r '.on.workflow_call.inputs.regenerate-command.default' "$workflow")" = "" ]
  [ -z "$(grep -F 'script/build.sh' "$workflow" || true)" ]
  [ -z "$(echo "$runs" | grep -F 'inputs.' || true)" ]
}
