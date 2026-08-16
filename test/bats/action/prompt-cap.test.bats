setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/prompt-cap/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
}

teardown() {
  rm -rf "$work"
}

# The action script with `nix` stubbed to echo its argv, so what reaches the
# binary is asserted without building it.
run_cap_action() {
  RAINIX_PROMPT_PATHS="$1" \
    RAINIX_PROMPT_CAP="$2" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/prompt-cap" \
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

@test "the inputs reach the binary as one argument each" {
  run run_cap_action '*prompt*.txt' 60000

  [ "$status" -eq 0 ]
  [[ "$output" == *"<prompt-cap> <--paths> <*prompt*.txt> <--cap> <60000>" ]]
}

@test "a glob is passed through unexpanded rather than matched by the shell" {
  cd "$work"
  touch a-prompt.txt b-prompt.txt

  run run_cap_action '*prompt*.txt' 60000

  [ "$status" -eq 0 ]
  [[ "$output" == *"<--paths> <*prompt*.txt>"* ]]
  [[ "$output" != *"a-prompt.txt"* ]]
}

@test "a multi-line paths input stays one argument" {
  run run_cap_action 'a.txt
prompts/**' 10

  [ "$status" -eq 0 ]
  [[ "$output" == *"<--paths> <a.txt"$'\n'"prompts/**>"* ]]
}

# The binary itself, as CI invokes it: rainix-static is on PATH in every shell.

@test "prompt files within cap exit 0" {
  printf 'xxxxx' >"$work/a-prompt.txt"

  run rainix-static prompt-cap --root "$work" --paths '*prompt*' --cap 10

  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — 5 bytes"* ]]
}

@test "prompt files over cap exit 1, largest first, with the total and overage" {
  printf 'read notes.md\n' >"$work/a-prompt.txt"
  head -c 100 /dev/zero | tr '\0' 'n' >"$work/notes.md"

  run rainix-static prompt-cap --root "$work" --paths '*prompt*' --cap 10

  [ "$status" -eq 1 ]
  [[ "$output" == *"114 bytes"* ]]
  [[ "$output" == *"104 over"* ]]
  [[ "$output" == *"notes.md (referenced by a-prompt.txt)"* ]]
}

@test "a glob matching nothing exits 1 rather than passing" {
  run rainix-static prompt-cap --root "$work" --paths 'nope/*.txt' --cap 10

  [ "$status" -eq 1 ]
  [[ "$output" == *"no file matched"* ]]
}

@test "a missing cap exits 1 rather than defaulting to one" {
  printf 'x' >"$work/a-prompt.txt"

  run rainix-static prompt-cap --root "$work" --paths '*prompt*'

  [ "$status" -eq 1 ]
  [[ "$output" == *"--cap"* ]]
}
