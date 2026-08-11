setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/rpc-preflight/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
}

run_preflight_action() {
  local archive="$1"

  RAINIX_RPC_ARCHIVE="$archive" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/rpc-preflight" \
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

@test "archive mode does not pass --no-archive" {
  run run_preflight_action true

  [ "$status" -eq 0 ]
  [[ "$output" == *" <--> <rpc-preflight>" ]]
  [[ "$output" != *"<--no-archive>"* ]]
}

@test "non-archive mode passes --no-archive" {
  run run_preflight_action false

  [ "$status" -eq 0 ]
  [[ "$output" == *" <--> <rpc-preflight> <--no-archive>" ]]
}
