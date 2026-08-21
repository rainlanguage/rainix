# The append-only gate's SKIP predicate. It shipped globbing
# `src/generated/*/*.pointers.sol` — a filename no deploy repo has ever written,
# since frozen records are `src/generated/<tag>/<Name>.sol` — so the gate skipped
# in exactly the repos it exists for and was a no-op org-wide
# (rainlanguage/rainix#341). These cover which trees reach the check and which do
# not, against the record shape deploy repos actually carry.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/frozen-snapshots-append-only/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
}

teardown() {
  rm -rf "$work"
}

# The action script run against $work as the repo, with `git` and `nix` stubbed
# to echo their argv — so what reaches the binary is asserted without a network
# fetch or a build.
run_gate() {
  cd "$work" || return 1
  GITHUB_BASE_REF="${1:-}" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/frozen-snapshots-append-only" \
    ACTION_SCRIPT="$action_script" \
    bash -c '
      git() {
        # Report a non-shallow checkout so the unshallow branch stays out of the
        # way; everything else just records that it was called.
        if [ "$1 $2" = "rev-parse --git-dir" ]; then
          printf "%s\n" "$PWD/.git"
          return 0
        fi
        printf "git"
        printf " <%s>" "$@"
        printf "\n"
      }
      nix() {
        printf "nix"
        printf " <%s>" "$@"
        printf "\n"
      }
      export -f git nix
      bash -c "$ACTION_SCRIPT"
    '
}

@test "a deploy repo's real record shape reaches the check instead of skipping" {
  # THE regression: this is the tree rain.factory.deploy carries, and the
  # `*.pointers.sol` glob skipped it.
  mkdir -p "$work/src/generated/0_1_9" "$work/src/generated/candidate"
  touch "$work/src/generated/0_1_9/CloneFactory.sol"
  touch "$work/src/generated/candidate/CloneFactory.sol"

  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" != *"skip"* ]]
  [[ "$output" == *"<snapshots-append-only> <--base> <origin/main>"* ]]
}

@test "a repo with no generated record skips without fetching or checking" {
  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" == *"no generated record; skip"* ]]
  [[ "$output" != *"<--base>"* ]]
  [[ "$output" != *"<fetch>"* ]]
}

@test "a record holding only the rolling candidate still reaches the check" {
  # A deploy repo before its first release. A snapshot deleted from such a
  # branch is still a deletion the check must see.
  mkdir -p "$work/src/generated/candidate"
  touch "$work/src/generated/candidate/MetaBoard.sol"

  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" != *"skip"* ]]
  [[ "$output" == *"<snapshots-append-only>"* ]]
}

@test "the legacy pointers filename is still checked, not newly skipped" {
  mkdir -p "$work/src/generated/0_1_4"
  touch "$work/src/generated/0_1_4/CloneFactory.pointers.sol"

  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" == *"<snapshots-append-only>"* ]]
}

@test "the pull_request base ref is the ref the check is pointed at" {
  mkdir -p "$work/src/generated/0_1_9"
  touch "$work/src/generated/0_1_9/CloneFactory.sol"

  run run_gate release-2026

  [ "$status" -eq 0 ]
  [[ "$output" == *"<--base> <origin/release-2026>"* ]]
  [[ "$output" == *"git <fetch> <--no-tags> <origin> <release-2026>"* ]]
}

@test "a src/generated FILE rather than a directory is not mistaken for a record" {
  # `-d` and not `-e`: a stray file named src/generated is not a record tree,
  # and fetching the base to diff it would fail for nothing.
  mkdir -p "$work/src"
  touch "$work/src/generated"

  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" == *"no generated record; skip"* ]]
}
