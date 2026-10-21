setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/relative-src-imports/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
}

teardown() {
  rm -rf "$work"
}

run_action() {
  GITHUB_ACTION_PATH="$repo_root/.github/actions/relative-src-imports" \
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

# $1 is the body of the one tracked src/A.sol.
mk_consumer() {
  git -C "$work" init -q
  mkdir -p "$work/src"
  printf '%s\n' "$1" >"$work/src/A.sol"
  git -C "$work" add -A
}

@test "the action runs the check over the checkout root" {
  run run_action

  [ "$status" -eq 0 ]
  [[ "$output" == *"#rainix-static> <--> <relative-src-imports> <.>" ]]
}

@test "a clean repo exits 0 and says so" {
  mk_consumer 'import "./B.sol";'
  run rainix-static relative-src-imports "$work"

  [ "$status" -eq 0 ]
  [ "$output" = "relative-src-imports: clean" ]
}

@test "a repo-rooted import exits 1 and names the file and path" {
  mk_consumer 'import "src/B.sol";'
  run rainix-static relative-src-imports "$work"

  [ "$status" -eq 1 ]
  [ "${lines[1]}" = '  src/A.sol: "src/B.sol"' ]
}

@test "outside a git repo the check fails as an error, not a pass" {
  mkdir -p "$work/src"
  run rainix-static relative-src-imports "$work"

  [ "$status" -eq 1 ]
  [[ "$output" == "::error::relative-src-imports: git ls-files failed: "* ]]
}

@test "the directory defaults to the working directory" {
  mk_consumer 'import "src/B.sol";'
  cd "$work"
  run rainix-static relative-src-imports

  [ "$status" -eq 1 ]
}
