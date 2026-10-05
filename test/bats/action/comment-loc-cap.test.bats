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
  RAINIX_COMMENT_LOC_BUCKETS="$1" \
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

@test "each line of the buckets input is one --bucket argument" {
  run run_cap_action 'src .github
test'

  [ "$status" -eq 0 ]
  [[ "$output" == *"<comment-loc-cap> <--bucket> <src .github> <--bucket> <test>" ]]
}

@test "blank lines in the buckets input are not buckets" {
  run run_cap_action '
src

   '

  [ "$status" -eq 0 ]
  [[ "$output" == *"<comment-loc-cap> <--bucket> <src>" ]]
}

# The defaults live in the binary, in one place. A copy here is how the old
# `paths` default drifted to `src test .github` while this file still asserted
# `src test` — and nothing noticed, because the bats suite swallowed it.
@test "an unset buckets input passes no bucket, leaving the defaults to the binary" {
  run run_cap_action ''

  [ "$status" -eq 0 ]
  [[ "$output" == *"<comment-loc-cap>" ]]
  [[ "$output" != *"--bucket"* ]]
}

@test "the action does not carry its own copy of the default buckets" {
  [ "$(yq -r '.inputs.buckets.default' "$action")" = "" ]
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

@test "a bucket over the cap exits 1 with totals and every file listed" {
  fixture_repo

  run rainix-static comment-loc-cap --root "$work"

  [ "$status" -eq 1 ]
  [[ "$output" == *"8 comment lines against a cap of 6 (twice 3 code lines) across 2 files"* ]]
  [[ "$output" == *"7       2  src/Over.sol"* ]]
  [[ "$output" == *"1       1  src/Ok.sol"* ]]
  [[ "$output" != *"notes.md"* ]]
}

@test "a bucket whose comment lines are at or under twice its code lines exits 0" {
  fixture_repo
  git -C "$work" rm -qf src/Over.sol

  run rainix-static comment-loc-cap --root "$work"

  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — 1 comment against a cap of 2 (twice 1 code lines) across 1 files"* ]]
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

@test "a named bucket selecting no counted file exits 1 rather than passing" {
  fixture_repo

  run rainix-static comment-loc-cap --root "$work" --bucket src --bucket test

  [ "$status" -eq 1 ]
  [[ "$output" == *"no tracked source file under test"* ]]
}

@test "every bucket selecting no counted file exits 1" {
  fixture_repo
  git -C "$work" rm -q --cached src/Over.sol src/Ok.sol

  run rainix-static comment-loc-cap --root "$work"

  [ "$status" -eq 1 ]
  [[ "$output" == *"no tracked source file under any bucket"* ]]
}

@test "a default bucket this repo has no files for is skipped rather than failing" {
  fixture_repo
  git -C "$work" rm -qf src/Over.sol

  run rainix-static comment-loc-cap --root "$work"

  [ "$status" -eq 0 ]
  [[ "$output" == *"test: no tracked source file — skipped"* ]]
}

# The whole point: by default `test`'s code cannot fund `src`'s prose.
@test "src is over its own cap though the repo aggregate is under" {
  fixture_repo
  for _ in $(seq 20); do printf 'x;\n' >>"$work/test/Heavy.sol"; done
  git -C "$work" add test/Heavy.sol

  run rainix-static comment-loc-cap --root "$work" --paths 'src test'
  [ "$status" -eq 0 ]

  run rainix-static comment-loc-cap --root "$work"

  [ "$status" -eq 1 ]
  [[ "$output" == *"src .github: 8 comment lines against a cap of 6"* ]]
  [[ "$output" == *"test: clean — 0 comment against a cap of 40 (twice 20 code lines) across 1 files"* ]]
}
