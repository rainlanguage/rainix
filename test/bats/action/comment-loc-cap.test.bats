setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/comment-loc-cap/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
}

teardown() {
  rm -rf "$work"
}

# The action script with `nix` stubbed to echo its argv, so what reaches the
# binary is asserted without building it.
run_cap_action() {
  RAINIX_COMMENT_LOC_PATHS="$1" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/comment-loc-cap" \
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

@test "the paths input reaches the binary as one argument" {
  run run_cap_action 'src test'

  [ "$status" -eq 0 ]
  [[ "$output" == *"<comment-loc-cap> <--paths> <src test>" ]]
}

@test "the action defaults its paths input to src test" {
  [ "$(yq -r '.inputs.paths.default' "$action")" = "src test" ]
}

# The binary itself, as CI invokes it: rainix-static is on PATH in every shell.

fixture_repo() {
  git -C "$work" init -q
  mkdir -p "$work/src" "$work/test"
  printf '// a\n// b\n// c\n// d\n// e\n// f\n// g\nx;\ny;\n' >"$work/src/Over.sol"
  printf '// a\nx;\n' >"$work/src/Ok.sol"
  printf '# a\n# b\n' >"$work/test/notes.md"
  git -C "$work" add src test
}

@test "an aggregate over the cap exits 1 with totals and every file listed" {
  fixture_repo

  run rainix-static comment-loc-cap --root "$work"

  [ "$status" -eq 1 ]
  [[ "$output" == *"8 comment lines against a cap of 6 (twice 3 code lines) across 2 files"* ]]
  [[ "$output" == *"7       2  src/Over.sol"* ]]
  [[ "$output" == *"1       1  src/Ok.sol"* ]]
  [[ "$output" != *"notes.md"* ]]
}

@test "a tree whose comment lines are at or under twice its code lines in aggregate exits 0" {
  fixture_repo
  git -C "$work" rm -qf src/Over.sol

  run rainix-static comment-loc-cap --root "$work"

  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — 1 files"* ]]
}

@test "an untracked offender is not counted" {
  fixture_repo
  git -C "$work" rm -q --cached src/Over.sol

  run rainix-static comment-loc-cap --root "$work"

  [ "$status" -eq 0 ]
}

@test "a path set selecting no counted file exits 1 rather than passing" {
  fixture_repo

  run rainix-static comment-loc-cap --root "$work" --paths test

  [ "$status" -eq 1 ]
  [[ "$output" == *"no tracked source file under test"* ]]
}
