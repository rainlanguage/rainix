# The install-staged-config composite's bash half: which trees reach the Rust
# install and which are skipped. What a staging directory must CONTAIN, and
# what installing it does, is covered by the Rust unit tests in
# rainix-static/src/staged_config.rs; what is asserted here is the one decision
# bash makes — because this action runs in every sol consumer, and a skip and a
# pass are the same green.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/install-staged-config/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
}

teardown() {
  rm -rf "$work"
}

# The action script run against $work as the repo, with `nix` stubbed to echo
# its argv — so what reaches the binary is asserted without a build.
run_step() {
  cd "$work" || return 1
  GITHUB_ACTION_PATH="$repo_root/.github/actions/install-staged-config" \
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

@test "a repo with a staging directory reaches the install" {
  mkdir -p "$work/.staged-config"
  printf 'generated\n' > "$work/.staged-config/foundry.toml"

  run run_step

  [ "$status" -eq 0 ]
  [[ "$output" != *"skip"* ]]
  [[ "$output" == *"<install-staged-config>"* ]]
}

# Every consumer that generates no config stages nothing, and must not pay a
# nix build to be told so.
@test "a repo with nothing staged skips without building the binary" {
  run run_step

  [ "$status" -eq 0 ]
  [[ "$output" == *"nothing staged; skip"* ]]
  [[ "$output" != *"<install-staged-config>"* ]]
}

# Bash decides only whether there is anything at the path. Every question about
# WHAT is there belongs to the check, which refuses these — a bash-side `-d`
# would answer them with a silent skip instead.
@test "a FILE at the staging path reaches the check rather than being skipped" {
  printf 'not a directory\n' > "$work/.staged-config"

  run run_step

  [ "$status" -eq 0 ]
  [[ "$output" != *"skip"* ]]
  [[ "$output" == *"<install-staged-config>"* ]]
}

@test "a dangling symlink at the staging path reaches the check" {
  ln -s "$work/never-written" "$work/.staged-config"

  run run_step

  [ "$status" -eq 0 ]
  [[ "$output" != *"skip"* ]]
  [[ "$output" == *"<install-staged-config>"* ]]
}

@test "the install runs from the composite's own checkout, not a pinned RAINIX_SHA" {
  mkdir -p "$work/.staged-config"

  run run_step

  [ "$status" -eq 0 ]
  # The script resolves the path itself, so compare against the same absolute
  # form rather than the `../../..` this file starts from.
  local resolved
  resolved="$(cd "$repo_root" && pwd)"
  [[ "$output" == *"<path:$resolved#rainix-static>"* ]]
}
