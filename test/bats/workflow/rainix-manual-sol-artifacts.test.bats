# Nothing in this repo executes rainix-manual-sol-artifacts.yaml — it is
# `workflow_call` only, so its only runners are the consumer repos.
#
# The recheck's verdict is covered by the Rust unit tests and
# test/bats/action/verify-status-recheck.test.bats; what is asserted here is the
# wiring they cannot see: that the deploy step leaves the files the action
# reads, and that a failed deploy reaches the recheck instead of passing.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-manual-sol-artifacts.yaml"
  action="$repo_root/.github/actions/verify-status-recheck/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  deploy="$(yq -r '.jobs.deploy.steps[] | select(.id == "deploy") | .run' "$workflow")"
  runs="$(yq -r '.jobs.deploy.steps[] | select(.run) | .run' "$workflow")"
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
}

# The deploy step as bash runs it, with `verify` set to $1 and every other
# expression stubbed, against a nix whose `forge config` fails and whose
# `forge script` exits $2. Each nix call is logged to $BATS_TEST_TMPDIR/calls.
run_deploy() {
  local script="$deploy"
  # shellcheck disable=SC2016
  script="${script//'${{ inputs.verify }}'/$1}"
  while [[ "$script" =~ \$\{\{[^}]*\}\} ]]; do
    script="${script/"${BASH_REMATCH[0]}"/x}"
  done
  SCRIPT_STATUS="$2" CALLS="$BATS_TEST_TMPDIR/calls" RUNNER_TEMP="$BATS_TEST_TMPDIR" \
    DEPLOY_SCRIPT="$script" bash -c '
      nix() {
        printf "%s\n" "$*" >>"$CALLS"
        case "$*" in
          *"forge config"*) return 1 ;;
          *"forge script"*) return "$SCRIPT_STATUS" ;;
        esac
      }
      export -f nix
      bash -c "$DEPLOY_SCRIPT"
    '
}

# The one value after `<flag> "` in the action script.
action_path() {
  echo "$action_script" | grep -o -- "$1 \"[^\"]*\"" | sed "s/^$1 \"//; s/\"$//"
}

@test "the action reads files under the runner temp dir, never the working tree" {
  local log config
  log="$(action_path --log)"
  config="$(action_path --config)"
  # The literals the action script carries, not expansions of them.
  # shellcheck disable=SC2016
  [[ "$log" == '$RUNNER_TEMP/'* ]]
  # shellcheck disable=SC2016
  [[ "$config" == '$RUNNER_TEMP/'* ]]
}

@test "forge script's output is teed into the log the action reads" {
  [[ "$deploy" == *"forge script "*"2>&1 | tee \"$(action_path --log)\""* ]]
}

@test "forge config is written to the file the action reads" {
  [[ "$deploy" == *"forge config --json > \"$(action_path --config)\""* ]]
}

# Without pipefail bash reports tee's status, so a failed broadcast would pass.
@test "the deploy step sets pipefail" {
  [[ "$deploy" == "set -euo pipefail"* ]]
  run run_deploy true 1
  [ "$status" -ne 0 ]
}

# Only the recheck reads the config dump, so it must never stop a deploy.
@test "a deploy without verify never runs forge config" {
  run run_deploy false 0
  [ "$status" -eq 0 ]
  run grep -c "forge config" "$BATS_TEST_TMPDIR/calls"
  [ "$output" = 0 ]
  grep -q "forge script" "$BATS_TEST_TMPDIR/calls"
}

@test "a failed forge config dump does not stop a verifying deploy" {
  run run_deploy true 0
  [ "$status" -eq 0 ]
  grep -q "forge config" "$BATS_TEST_TMPDIR/calls"
  grep -q "forge script" "$BATS_TEST_TMPDIR/calls"
}

@test "only a verifying deploy continues on error, and only into the recheck" {
  local deploy_index recheck_index
  # shellcheck disable=SC2016
  [ "$(yq -r '.jobs.deploy.steps[] | select(.id == "deploy") | ."continue-on-error"' "$workflow")" = '${{ inputs.verify }}' ]
  deploy_index="$(yq -r '.jobs.deploy.steps | to_entries[] | select(.value.id == "deploy") | .key' "$workflow")"
  recheck_index="$(yq -r '.jobs.deploy.steps | to_entries[] | select(.value.uses == "rainlanguage/rainix/.github/actions/verify-status-recheck@main") | .key' "$workflow")"
  [ "$recheck_index" -eq $((deploy_index + 1)) ]
  [ "$(yq -r ".jobs.deploy.steps[$recheck_index].if" "$workflow")" = "steps.deploy.outcome == 'failure'" ]
  [ "$(yq -r ".jobs.deploy.steps[$recheck_index].\"continue-on-error\"" "$workflow")" = "null" ]
  [ "$(yq -r '.jobs.deploy.steps | length' "$workflow")" -eq $((recheck_index + 1)) ]
}

@test "every rainix-manual-sol-artifacts run step resolves rainix through the pinned sha" {
  [ -n "$sha" ]
  [ "$sha" != "null" ]
  local unpinned
  unpinned="$(echo "$runs" | grep 'github:rainlanguage/rainix' | grep -v 'env.RAINIX_SHA' || true)"
  if [ -n "$unpinned" ]; then
    echo "FAIL: unpinned rainix refs in rainix-manual-sol-artifacts.yaml:" >&2
    echo "$unpinned" >&2
    return 1
  fi
}
