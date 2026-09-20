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
  local state_expr
  state_expr="$(yq -r '.runs.steps[0].env.RAINIX_CODEGEN_WITNESS_STATE' "$action")"
  # `${{ runner.temp }}` is a GitHub Actions expression: single-quoted so the shell cannot expand it.
  # shellcheck disable=SC2016
  [[ "$state_expr" == '${{ runner.temp }}/'* ]]
}

# Staged, not committed: git ls-files reads the index.
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

# Filesystem mtime resolution: a write inside the window must be unambiguously later.
age_tree() {
  find "$work" -path "$work/.git" -prune -o -type f -exec touch -m -t 202001010000 {} +
}

regenerate() {
  local f
  for f in "$@"; do
    local content
    content="$(cat "$work/$f")"
    printf '%s\n' "$content" >"$work/$f"
  done
}

@test "a live generator's identical rewrite is witnessed, with git diff clean" {
  mk_consumer
  age_tree

  run rainix-static codegen-witness mark --root "$work" --state "$state"
  [ "$status" -eq 0 ]

  regenerate src/generated/Thing.sol src/lib/LibReleasedSuites.sol

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — 2 declared generated files"* ]]

  run git -C "$work" diff --exit-code
  [ "$status" -eq 0 ]
}

@test "a generator that stopped emitting a file fails, though git diff is clean" {
  mk_consumer
  age_tree

  run rainix-static codegen-witness mark --root "$work" --state "$state"
  [ "$status" -eq 0 ]

  regenerate src/generated/Thing.sol

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"src/lib/LibReleasedSuites.sol"* ]]
  [[ "$output" == *"emitter is dead"* ]]
  [[ "$output" != *"declares src/generated/Thing.sol generated, but no codegen hook"* ]]

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
  [[ "$output" == *"src/generated/Thing.sol"* ]]
  [[ "$output" == *"src/lib/LibReleasedSuites.sol"* ]]
}

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

@test "the manifest path defaults to script/codegen-manifest.txt" {
  mk_consumer
  age_tree

  rainix-static codegen-witness mark --root "$work" --state "$state"
  regenerate src/generated/Thing.sol

  run rainix-static codegen-witness verify --root "$work" --state "$state"
  [ "$status" -eq 1 ]
  [[ "$output" == *"script/codegen-manifest.txt declares"* ]]
}
