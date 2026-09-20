# The half of rainix-copy-artifacts' currency check that a diff cannot make.
#
# That job re-runs the consumer's codegen hooks and then `git diff --exit-code`,
# which is blind to a generator that has STOPPED emitting a file: the committed
# copy is already correct, so nothing is rewritten, nothing differs, and the job
# is green over a dead emitter (rainlanguage/rain.factory.deploy#35). The
# control/mutant pair from that issue is reproduced below against the real
# binary — including the part that makes it invisible, that `git diff` stays
# clean in BOTH halves.
#
# Two layers, as elsewhere in test/bats/action: the composite's wiring with
# `nix` stubbed (what reaches the binary), then the binary itself, which is on
# PATH in every shell.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/codegen-witness/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
  state="$work/witness.json"
}

teardown() {
  rm -rf "$work"
}

# The action script with `nix` stubbed to echo its argv, so what reaches the
# binary is asserted without building it.
run_witness_action() {
  RAINIX_CODEGEN_WITNESS_PHASE="$1" \
    RAINIX_CODEGEN_WITNESS_STATE="$2" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/codegen-witness" \
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

@test "the phase reaches the binary as the subcommand's phase argument" {
  run run_witness_action mark /tmp/w.json

  [ "$status" -eq 0 ]
  [[ "$output" == *"<codegen-witness> <mark> <--state> </tmp/w.json>" ]]
}

@test "both phases are wired through the same action, not two spellings" {
  run run_witness_action verify /tmp/w.json

  [ "$status" -eq 0 ]
  [[ "$output" == *"<codegen-witness> <verify> <--state> </tmp/w.json>" ]]
}

@test "the state path is a runner temp file, never a path in the working tree" {
  # It must not land in the tree: the job runs `git diff --exit-code` right
  # after this check, and a stray state file there would fail it.
  local state_expr
  state_expr="$(yq -r '.runs.steps[0].env.RAINIX_CODEGEN_WITNESS_STATE' "$action")"
  # The single quotes are the point: `${{ runner.temp }}` is a GitHub Actions
  # expression that must reach the YAML verbatim, so it is matched as a literal
  # prefix and must not be expanded by the shell running this test.
  # shellcheck disable=SC2016
  [[ "$state_expr" == '${{ runner.temp }}/'* ]]
}

# The binary itself, as CI invokes it.

# A consumer repo as copy-artifacts finds it: a codegen hook, a committed
# generated file, and a manifest declaring it. Files are staged rather than
# committed because `git ls-files` reads the index, and because `git diff` then
# compares the worktree against exactly the "committed" bytes.
mk_consumer() {
  git -C "$work" init -q
  mkdir -p "$work/script" "$work/src/generated" "$work/src/lib"
  printf 'contract Build {}\n' >"$work/script/Build.sol"
  printf 'GENERATED SNAPSHOT\n' >"$work/src/generated/Thing.sol"
  printf 'GENERATED AGGREGATE\n' >"$work/src/lib/LibReleasedSuites.sol"
  cat >"$work/script/codegen-manifest.txt" <<'EOF'
# declared generated files
src/generated/Thing.sol
src/lib/LibReleasedSuites.sol
EOF
  git -C "$work" add -A
}

# Age every file so that a real write during the window is unambiguously a
# later mtime, whatever the filesystem's timestamp resolution. In CI the same
# gap comes for free: checkout runs minutes before the codegen hooks do.
age_tree() {
  find "$work" -path "$work/.git" -prune -o -type f -exec touch -m -t 202001010000 {} +
}

# A live generator: rewrites each file it is given with the SAME bytes it
# already holds, which is what regeneration does on a current tree.
regenerate() {
  local f
  for f in "$@"; do
    local content
    content="$(cat "$work/$f")"
    printf '%s\n' "$content" >"$work/$f"
  done
}

# THE CONTROL, from the issue: a live generator rewrites the file. Byte
# identical, so `git diff` sees nothing — and the witness sees the write.
@test "a live generator's identical rewrite is witnessed, with git diff clean" {
  mk_consumer
  age_tree

  run rainix-static codegen-witness mark --root "$work" --state "$state"
  [ "$status" -eq 0 ]

  regenerate src/generated/Thing.sol src/lib/LibReleasedSuites.sol

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — 2 declared generated files"* ]]

  # The old check's whole signal, for contrast with the next test.
  run git -C "$work" diff --exit-code
  [ "$status" -eq 0 ]
}

# THE MUTANT, from the issue: the one call emitting the aggregate is removed.
# The generator still exits 0, the committed file is untouched and correct,
# `git diff` is still clean — and this is the only thing that notices.
@test "a generator that stopped emitting a file fails, though git diff is clean" {
  mk_consumer
  age_tree

  run rainix-static codegen-witness mark --root "$work" --state "$state"
  [ "$status" -eq 0 ]

  # Every emitter but the aggregate's.
  regenerate src/generated/Thing.sol

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"src/lib/LibReleasedSuites.sol"* ]]
  [[ "$output" == *"emitter is dead"* ]]
  # The file that IS still emitted must not be blamed.
  [[ "$output" != *"declares src/generated/Thing.sol generated, but no codegen hook"* ]]

  # The reason the defect was invisible: the check it sits beside is happy.
  run git -C "$work" diff --exit-code
  [ "$status" -eq 0 ]
}

@test "a declared file that no longer exists at all is named as such" {
  mk_consumer
  rm "$work/src/lib/LibReleasedSuites.sol"
  age_tree

  rainix-static codegen-witness mark --root "$work" --state "$state"
  regenerate src/generated/Thing.sol

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"src/lib/LibReleasedSuites.sol"* ]]
  [[ "$output" == *"no such file exists"* ]]
}

# The workflow is consumed at @main by every Rain repo, most of which generate
# nothing: they must pass without being asked for anything.
@test "a repo with no codegen hook and no manifest passes" {
  git -C "$work" init -q
  mkdir -p "$work/src"
  printf 'contract A {}\n' >"$work/src/A.sol"
  git -C "$work" add -A
  age_tree

  rainix-static codegen-witness mark --root "$work" --state "$state"

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 0 ]
  [[ "$output" == *"no codegen hook and no"* ]]
}

@test "a repo that runs codegen but declares nothing fails, and is printed one" {
  mk_consumer
  rm "$work/script/codegen-manifest.txt"
  git -C "$work" rm -q --cached script/codegen-manifest.txt
  age_tree

  rainix-static codegen-witness mark --root "$work" --state "$state"
  regenerate src/generated/Thing.sol src/lib/LibReleasedSuites.sol

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"script/Build.sol"* ]]
  # The starting point it prints is what this run actually wrote.
  [[ "$output" == *"src/generated/Thing.sol"* ]]
  [[ "$output" == *"src/lib/LibReleasedSuites.sol"* ]]
}

# Deliberate asymmetry: an incidental write inside the window must never redden
# a job every Rain repo runs. It is reported, not enforced.
@test "a written but undeclared file is a note, not a failure" {
  mk_consumer
  printf 'lock\n' >"$work/soldeer.lock"
  git -C "$work" add soldeer.lock
  age_tree

  rainix-static codegen-witness mark --root "$work" --state "$state"
  regenerate src/generated/Thing.sol src/lib/LibReleasedSuites.sol soldeer.lock

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 0 ]
  [[ "$output" == *"note — written but not declared"* ]]
  [[ "$output" == *"soldeer.lock"* ]]
}

# A deletion is not a write. The diff catches a deleted generated file on its
# own; blaming the emitter for it would send the reader to the wrong place.
@test "a file the hooks deleted is not counted as written" {
  mk_consumer
  age_tree

  rainix-static codegen-witness mark --root "$work" --state "$state"
  regenerate src/generated/Thing.sol
  rm "$work/src/lib/LibReleasedSuites.sol"

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"no such file exists"* ]]
}

@test "verify without a prior mark fails naming mark, rather than passing" {
  mk_consumer

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"codegen-witness mark"* ]]
}

@test "an unknown phase fails rather than defaulting to one" {
  run rainix-static codegen-witness --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"mark"* ]]
  [[ "$output" == *"verify"* ]]
}

@test "a missing --state fails rather than inventing a path" {
  run rainix-static codegen-witness mark --root "$work"
  [ "$status" -eq 1 ]
  [[ "$output" == *"--state"* ]]
}

# The default the workflow relies on: neither the action nor the workflow passes
# --manifest, so the path the binary defaults to is the one consumers commit.
@test "the manifest path defaults to script/codegen-manifest.txt" {
  mk_consumer
  age_tree

  rainix-static codegen-witness mark --root "$work" --state "$state"
  regenerate src/generated/Thing.sol

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"script/codegen-manifest.txt declares"* ]]
}
