# Where generated Solidity sources live is ONE value, and every rainix mechanism
# that needs it reads that value from `rainix-static generated-dir` rather than
# spelling `src/generated` again (rainlanguage/rainix#313). Restating it is the
# hazard: the restatement keeps matching nothing after the canonical path moves,
# so the copy-artifacts currency guard, the frozen-snapshot check and the
# autopublish content gate all go quietly inert instead of red.
#
# Each mechanism here is therefore driven with a NON-default directory reported
# by the binary, and asserted to follow it — a hand-written `src/generated` fails
# these tests in both directions.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  # Deliberately neither `src/generated` nor a prefix/suffix of it.
  stub_dir="gen/out"
  work="$(mktemp -d)"
}

teardown() {
  rm -rf "$work"
}

# Run a workflow/action bash body with `nix` stubbed: the `generated-dir` call
# answers $STUB_GENERATED_DIR (or fails, when $STUB_GENERATED_DIR_FAILS is set),
# every other invocation echoes its argv so what would have run is assertable
# without building anything. `git` is stubbed the same way, so the scripts' fetch
# plumbing is inert in a bare temp dir.
run_stubbed() {
  ACTION_SCRIPT="$1" \
    STUB_GENERATED_DIR="$stub_dir" \
    STUB_GENERATED_DIR_FAILS="${STUB_GENERATED_DIR_FAILS:-}" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/frozen-snapshots-append-only" \
    bash -c '
      nix() {
        last=""
        for a in "$@"; do last="$a"; done
        if [ "$last" = "generated-dir" ]; then
          if [ -n "$STUB_GENERATED_DIR_FAILS" ]; then
            echo "nix stub: generated-dir unavailable" >&2
            return 1
          fi
          printf "%s\n" "$STUB_GENERATED_DIR"
          return 0
        fi
        printf "nix"
        printf " <%s>" "$@"
        printf "\n"
      }
      git() {
        printf "git"
        printf " <%s>" "$@"
        printf "\n"
      }
      export -f nix git
      bash -c "$ACTION_SCRIPT"
    '
}

frozen_action() {
  run_stubbed "$(yq -r '.runs.steps[0].run' \
    "$repo_root/.github/actions/frozen-snapshots-append-only/action.yml")"
}

copy_artifacts_regen() {
  workflow="$repo_root/.github/workflows/rainix-copy-artifacts.yaml"
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
  # GitHub resolves ${{ … }} before bash ever sees the body; left in, bash reads
  # it as a bad substitution and the step dies before reaching what is asserted.
  run_stubbed "$(yq -r '.jobs.copy-artifacts.steps[]
      | select(.name == "Regenerate generated sources")
      | .run' "$workflow" | sed "s|\${{ env.RAINIX_SHA }}|$sha|g")"
}

# The canonical value itself.

@test "generated-dir prints the one directory generated sources live in" {
  run rainix-static generated-dir

  [ "$status" -eq 0 ]
  [ "$output" = "src/generated" ]
}

# Mechanism 1: rainix-copy-artifacts' currency guard.

@test "copy-artifacts hard-fails on generated sources with no script/Build.sol" {
  cd "$work"
  mkdir -p "$stub_dir"

  run copy_artifacts_regen

  [ "$status" -eq 1 ]
  [[ "$output" == *"::error::"* ]]
  [[ "$output" == *"script/Build.sol"* ]]
}

@test "copy-artifacts does not guard a directory the binary does not name" {
  cd "$work"
  mkdir -p src/generated

  run copy_artifacts_regen

  [ "$status" -eq 0 ]
  [[ "$output" != *"::error::"* ]]
}

@test "copy-artifacts fails loudly when the canonical directory cannot be read" {
  cd "$work"
  mkdir -p "$stub_dir"
  STUB_GENERATED_DIR_FAILS=1

  run copy_artifacts_regen

  # Never exit 0 with the guard silently skipped because the lookup broke.
  [ "$status" -ne 0 ]
}

# Mechanism 2: the frozen-snapshots-append-only action's presence probe.

@test "frozen-snapshots checks per-tag snapshots under the directory the binary names" {
  cd "$work"
  mkdir -p "$stub_dir/0_1_4"
  touch "$stub_dir/0_1_4/CloneFactory.pointers.sol"

  run frozen_action

  [ "$status" -eq 0 ]
  [[ "$output" != *"no per-tag snapshots; skip"* ]]
  [[ "$output" == *"<snapshots-append-only>"* ]]
}

@test "frozen-snapshots skips snapshots outside the directory the binary names" {
  cd "$work"
  mkdir -p src/generated/0_1_4
  touch src/generated/0_1_4/CloneFactory.pointers.sol

  run frozen_action

  [ "$status" -eq 0 ]
  [[ "$output" == *"no per-tag snapshots; skip"* ]]
  [[ "$output" != *"<snapshots-append-only>"* ]]
}

@test "frozen-snapshots fails loudly when the canonical directory cannot be read" {
  cd "$work"
  mkdir -p "$stub_dir/0_1_4"
  touch "$stub_dir/0_1_4/CloneFactory.pointers.sol"
  STUB_GENERATED_DIR_FAILS=1

  run frozen_action

  [ "$status" -ne 0 ]
  [[ "$output" != *"no per-tag snapshots; skip"* ]]
}

# Mechanism 2b: the binary's own default root for that check is the same value,
# so the action never has to pass --root to keep the two in step.

@test "snapshots-append-only defaults its root to the canonical directory" {
  dir="$(rainix-static generated-dir)"
  cd "$work"
  git init -q -b main .
  git config user.email rainix@example.com
  git config user.name rainix
  mkdir -p "$dir/0_1_4"
  printf 'address constant A = address(1);\n' >"$dir/0_1_4/X.pointers.sol"
  git add -A
  git commit -qm base
  git checkout -qb branch
  printf 'address constant A = address(2);\n' >"$dir/0_1_4/X.pointers.sol"
  git commit -qam edit

  run rainix-static snapshots-append-only --base main

  [ "$status" -eq 1 ]
  [[ "$output" == *"modified frozen snapshot $dir/0_1_4/X.pointers.sol"* ]]
}
