# The recheck's verdict is covered by the Rust unit tests; what is asserted here
# is what the action hands the binary and that the binary's status is the step's.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/verify-status-recheck/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
}

run_action() {
  local nix_status="$1"

  NIX_STATUS="$nix_status" \
    RUNNER_TEMP=/runner-temp \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/verify-status-recheck" \
    ACTION_SCRIPT="$action_script" \
    bash -c '
      nix() {
        printf "nix"
        printf " <%s>" "$@"
        printf "\n"
        return "$NIX_STATUS"
      }
      export -f nix
      bash -c "$ACTION_SCRIPT"
    '
}

@test "the recheck reads the deploy step's log and forge config" {
  run run_action 0

  [ "$status" -eq 0 ]
  [[ "$output" == *" <--> <verify-status-recheck> <--log> </runner-temp/rainix-forge-script.log> <--config> </runner-temp/rainix-forge-config.json>" ]]
}

@test "the recheck runs from the action's own checkout" {
  run run_action 0

  [[ "$output" == "nix <run> <path:$(cd "$repo_root" && pwd)#rainix-static>"* ]]
}

@test "a failed recheck fails the step" {
  run run_action 1

  [ "$status" -ne 0 ]
}
