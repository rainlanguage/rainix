# The mutation-ledger composite's bash half: which trees reach the Rust check,
# which are skipped, and whether ancestry is asked with a history that can
# answer it. What a ledger must CONTAIN is covered by the Rust unit tests in
# rainix-static/src/mutation_ledger.rs; what is asserted here is the
# orchestration around them, because a skip and a pass are the same green.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/mutation-ledger/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
}

teardown() {
  rm -rf "$work"
}

# The action script run against $work as the repo, with `git` and `nix` stubbed
# to echo their argv — so what reaches the binary is asserted without a network
# fetch or a build. $1 is what `git rev-parse --is-shallow-repository` answers.
run_gate() {
  cd "$work" || return 1
  SHALLOW="${1:-false}" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/mutation-ledger" \
    ACTION_SCRIPT="$action_script" \
    bash -c '
      git() {
        if [ "$1 $2" = "rev-parse --is-shallow-repository" ]; then
          printf "%s\n" "$SHALLOW"
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

@test "a repo with a ledger reaches the check" {
  mkdir -p "$work/audit"
  printf '[]\n' > "$work/audit/mutation-test-scans.json"

  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" != *"skip"* ]]
  [[ "$output" == *"<mutation-ledger>"* ]]
}

@test "a repo with no ledger skips without fetching or checking" {
  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" == *"no ledger; skip"* ]]
  [[ "$output" != *"<mutation-ledger>"* ]]
  [[ "$output" != *"<fetch>"* ]]
}

# Every repo carries an audit/ directory long before it carries a ledger, and a
# directory named like the file is not a record either.
@test "an audit directory alone is not a ledger" {
  mkdir -p "$work/audit/protofire"

  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" == *"no ledger; skip"* ]]
}

@test "a DIRECTORY at the ledger path is not mistaken for a record" {
  mkdir -p "$work/audit/mutation-test-scans.json"

  run run_gate

  [ "$status" -eq 0 ]
  [[ "$output" == *"no ledger; skip"* ]]
}

# THE regression the gate would otherwise ship with: the shared checkout is
# shallow, and ancestry past the graft boundary is a silent wrong answer.
@test "a shallow checkout is deepened, commits only, before ancestry is asked" {
  mkdir -p "$work/audit"
  printf '[]\n' > "$work/audit/mutation-test-scans.json"

  run run_gate true

  [ "$status" -eq 0 ]
  [[ "$output" == *"git <fetch> <--no-tags> <--filter=tree:0> <--unshallow> <origin>"* ]]
  [[ "$output" == *"<mutation-ledger>"* ]]
}

@test "a checkout that already has history is not refetched" {
  mkdir -p "$work/audit"
  printf '[]\n' > "$work/audit/mutation-test-scans.json"

  run run_gate false

  [ "$status" -eq 0 ]
  [[ "$output" != *"<fetch>"* ]]
  [[ "$output" == *"<mutation-ledger>"* ]]
}

# The fetch is ordered before the check on purpose — the Rust half refuses a
# shallow checkout rather than answering wrongly, so a deepen that ran after it
# would turn every ledger repo red.
@test "the deepen precedes the check" {
  mkdir -p "$work/audit"
  printf '[]\n' > "$work/audit/mutation-test-scans.json"

  run run_gate true

  [ "$status" -eq 0 ]
  local fetch_line check_line
  fetch_line="$(echo "$output" | grep -n '<fetch>' | head -1 | cut -d: -f1)"
  check_line="$(echo "$output" | grep -n '<mutation-ledger>' | head -1 | cut -d: -f1)"
  [ -n "$fetch_line" ]
  [ -n "$check_line" ]
  [ "$fetch_line" -lt "$check_line" ]
}

@test "the check runs from the composite's own checkout, not a pinned RAINIX_SHA" {
  mkdir -p "$work/audit"
  printf '[]\n' > "$work/audit/mutation-test-scans.json"

  run run_gate

  [ "$status" -eq 0 ]
  # The script resolves the path itself, so compare against the same absolute
  # form rather than the `../../..` this file starts from.
  local resolved
  resolved="$(cd "$repo_root" && pwd)"
  [[ "$output" == *"<path:$resolved#rainix-static>"* ]]
}
