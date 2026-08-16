# Single quoting is load-bearing throughout this file: it carries GitHub Actions
# expressions (`${{ … }}`) that must stay unexpanded to be compared against the
# shipped YAML, and pipeline bodies whose `$n` belongs to the shell under test
# rather than to bats. SC2016 asks whether that is a mistake; here it is the
# point.
# shellcheck disable=SC2016

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-copy-artifacts.yaml"
  action="$repo_root/.github/actions/codegen-fixed-point/action.yml"

  # The step is a `uses:`, so what ships is two halves: the pipeline the
  # workflow hands over, and the composite script that runs it. Both are read
  # from the files rather than restated, so a change to either is a change to
  # what these tests exercise.
  pipeline="$(yq -r '.jobs["copy-artifacts"].steps[]
    | select(.uses // "" | test("codegen-fixed-point"))
    | .with.run' "$workflow")"
  passes_expr="$(yq -r '.jobs["copy-artifacts"].steps[]
    | select(.uses // "" | test("codegen-fixed-point"))
    | .with["max-passes"]' "$workflow")"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  # The bound a consumer gets when they pass no `with:` at all.
  default_passes="$(yq -r '.["on"].workflow_call.inputs["max-codegen-passes"].default' "$workflow")"

  work="$(mktemp -d)"
  consumer="$work/repo"
  export RAINIX_TEST_SHA=0000000000000000000000000000000000000000
  export RAINIX_TEST_LOG="$work/commands"
  export RAINIX_TEST_CODEGEN="$work/codegen.sh"
  : >"$RAINIX_TEST_LOG"
  : >"$work/passes"

  # `nix` and `forge` are the only two binaries this reaches for. Stubbing them
  # as files on PATH — not shell functions — is what lets the real
  # `rainix-static` reach them: it runs the pipeline in a bash it spawns itself.
  mkdir -p "$work/bin"
  cat >"$work/bin/nix" <<'STUB'
#!/usr/bin/env bash
# Anything but the two entry points this is written against is an error, never
# a silent pass: a renamed devshell, a dropped `--`, or an unpinned ref must not
# still look like a working step.
case "$1" in
  run)
    # The composite resolves the binary from its OWN checkout. A pinned
    # `github:` ref here could only ever name a commit predating the subcommand.
    case "$2" in
      path:*'#rainix-static') ;;
      *) echo "unexpected nix run flake ref: $2" >&2; exit 90 ;;
    esac
    [ "$3" = "--" ] || { echo "nix run needs -- before the subcommand" >&2; exit 90; }
    shift 3
    exec rainix-static "$@"
    ;;
  develop)
    [ "$2" = "github:rainlanguage/rainix/$RAINIX_TEST_SHA#sol-shell" ] ||
      { echo "unexpected nix develop flake ref: $2" >&2; exit 90; }
    [ "$3" = "-c" ] || { echo "nix develop needs -c before the command" >&2; exit 90; }
    shift 3
    exec "$@"
    ;;
  *) echo "unexpected nix subcommand: $1" >&2; exit 90 ;;
esac
STUB
  cat >"$work/bin/forge" <<'STUB'
#!/usr/bin/env bash
printf 'forge %s\n' "$*" >>"$RAINIX_TEST_LOG"
# Only the codegen script writes anything; build/copy/fmt are recorded so a
# test can see the whole pipeline repeat, not just its first command.
if [ "$1 $2" = "script ./script/Build.sol" ]; then
  exec "$RAINIX_TEST_CODEGEN"
fi
STUB
  chmod +x "$work/bin/nix" "$work/bin/forge"
  PATH="$work/bin:$PATH"

  # A consumer checkout with every optional hook present, so the default
  # scenario exercises the whole pipeline.
  export GIT_CONFIG_NOSYSTEM=1
  export HOME="$work"
  mkdir -p "$consumer/src/generated" "$consumer/script"
  printf 'out/\ncache/\ndependencies/\n' >"$consumer/.gitignore"
  printf 'pass 0\n' >"$consumer/src/generated/A.sol"
  printf '// codegen\n' >"$consumer/script/Build.sol"
  printf '// copy\n' >"$consumer/script/CopyArtifacts.sol"
  for hook in build-meta.sh build.sh; do
    printf '#!/usr/bin/env bash\nprintf "hook %s\\n" >>"$RAINIX_TEST_LOG"\n' "$hook" \
      >"$consumer/script/$hook"
    chmod +x "$consumer/script/$hook"
  done
  git -C "$consumer" init -q -b main .
  git -C "$consumer" config user.email rainix@example.com
  git -C "$consumer" config user.name rainix
  git -C "$consumer" add --all
  git -C "$consumer" commit -qm 'committed artifacts'
}

teardown() {
  rm -rf "$work"
}

# What a pass regenerates. `$n` is the pass number, counted OUTSIDE the checkout
# so counting cannot itself look like a tree that keeps moving.
codegen() {
  cat >"$RAINIX_TEST_CODEGEN" <<EOF
#!/usr/bin/env bash
set -eu
printf x >>"$work/passes"
n=\$(wc -c <"$work/passes" | tr -d ' ')
$1
EOF
  chmod +x "$RAINIX_TEST_CODEGEN"
}

passes_run() {
  wc -c <"$work/passes" | tr -d ' '
}

log_count() {
  grep -cFx -- "$1" "$RAINIX_TEST_LOG" || true
}

# The step as GitHub runs it: the workflow's expressions expanded, the resulting
# pipeline and bound handed to the composite through the same env keys the
# composite declares, and the composite's own script executed.
run_step() {
  local expanded max_passes
  expanded="$(printf '%s\n' "$pipeline" |
    sed -e "s|\${{ env.RAINIX_SHA }}|$RAINIX_TEST_SHA|g")"
  max_passes="$(printf '%s' "${1:-$default_passes}")"
  # A renamed input or env key would otherwise leave an expression in the text
  # and fail as a bash syntax error that looks nothing like its cause.
  case "$expanded$passes_expr" in
  *'${{ env.'*)
    echo "unexpanded env expression left in the pipeline: $expanded"
    return 90
    ;;
  esac
  # The bound the workflow forwards must be its own input, not a literal.
  [ "$passes_expr" = '${{ inputs.max-codegen-passes }}' ] || {
    echo "the step does not forward the workflow input as the bound: $passes_expr"
    return 90
  }
  (
    cd "$consumer" &&
      env GITHUB_ACTION_PATH="$repo_root/.github/actions/codegen-fixed-point" \
        RAINIX_CODEGEN_RUN="$expanded" \
        RAINIX_CODEGEN_MAX_PASSES="$max_passes" \
        ACTION_SCRIPT="$action_script" \
        bash -c 'bash -c "$ACTION_SCRIPT"'
  )
}

@test "a repo already at its fixed point passes, having run the pipeline once" {
  codegen "printf 'pass 0\n' > src/generated/A.sol"

  run run_step

  [ "$status" -eq 0 ]
  [ "$(passes_run)" -eq 1 ]
  [[ "$output" == *"fixed point reached after 1 pass"* ]]
}

@test "generation that settles on a later pass passes, leaving the settled tree" {
  codegen 'if [ "$n" -lt 3 ]; then printf "pass %s\n" "$n" > src/generated/A.sol; fi'

  run run_step

  [ "$status" -eq 0 ]
  [ "$(passes_run)" -eq 3 ]
  [ "$(cat "$consumer/src/generated/A.sol")" = "pass 2" ]
}

@test "every command of the pipeline is inside the loop, not just the codegen" {
  codegen 'if [ "$n" -lt 3 ]; then printf "pass %s\n" "$n" > src/generated/A.sol; fi'

  run run_step

  [ "$status" -eq 0 ]
  [ "$(log_count 'hook build-meta.sh')" -eq 3 ]
  [ "$(log_count 'forge script ./script/Build.sol')" -eq 3 ]
  [ "$(log_count 'forge build')" -eq 3 ]
  [ "$(log_count 'forge script ./script/CopyArtifacts.sol --ffi')" -eq 3 ]
  [ "$(log_count 'hook build.sh')" -eq 3 ]
  [ "$(log_count 'forge fmt')" -eq 3 ]
}

@test "generation that never settles fails as non-convergence, not as staleness" {
  codegen 'printf "pass %s\n" "$((n % 2))" > src/generated/A.sol'

  run run_step

  [ "$status" -eq 1 ]
  [ "$(passes_run)" -eq "$default_passes" ]
  [[ "$output" == *"::error::"* ]]
  [[ "$output" == *"did not reach a fixed point in $default_passes passes"* ]]
  [[ "$output" == *"cycle that does not settle"* ]]
}

@test "the workflow input is the bound, not a value baked into the binary" {
  codegen 'if [ "$n" -lt 3 ]; then printf "pass %s\n" "$n" > src/generated/A.sol; fi'

  run run_step 2

  [ "$status" -eq 1 ]
  [ "$(passes_run)" -eq 2 ]
  [[ "$output" == *"did not reach a fixed point in 2 passes"* ]]
}

@test "the default bound is 5" {
  [ "$default_passes" -eq 5 ]
}

@test "a pipeline command that fails stops the loop and is not retried" {
  codegen "printf 'regenerated\n' > src/generated/A.sol; exit 3"

  run run_step

  [ "$status" -eq 1 ]
  [ "$(passes_run)" -eq 1 ]
  [[ "$output" == *"regeneration command failed"* ]]
}

@test "the repo index is left untouched for the currency check that follows" {
  codegen "printf 'regenerated\n' > src/generated/A.sol"

  run run_step

  [ "$status" -eq 0 ]
  # Unstaged, " M path", is what the currency check reads. Staged, "M  path",
  # is a tree the check would report as clean while it is anything but.
  [ "$(git -C "$consumer" status --porcelain)" = " M src/generated/A.sol" ]
}

@test "gitignored build output does not read as a tree that never settles" {
  codegen 'mkdir -p out cache dependencies
    printf "%s" "$n" > out/A.json
    printf "%s" "$n" > cache/x
    printf "%s" "$n" > dependencies/dep.sol'

  run run_step

  [ "$status" -eq 0 ]
  [ "$(passes_run)" -eq 1 ]
}

@test "optional consumer hooks that are absent are skipped, not invoked" {
  git -C "$consumer" rm -q script/build-meta.sh script/build.sh script/CopyArtifacts.sol
  git -C "$consumer" commit -qm 'pointer-only consumer'
  codegen "printf 'pass 0\n' > src/generated/A.sol"

  run run_step

  [ "$status" -eq 0 ]
  [ "$(log_count 'hook build-meta.sh')" -eq 0 ]
  [ "$(log_count 'hook build.sh')" -eq 0 ]
  [ "$(log_count 'forge script ./script/CopyArtifacts.sol --ffi')" -eq 0 ]
  [ "$(log_count 'forge build')" -eq 1 ]
}

@test "every devshell the pipeline enters is pinned to the workflow's sha" {
  [[ "$pipeline" != *'github:rainlanguage/rainix#'* ]]
  [[ "$pipeline" != *'github:rainlanguage/rainix/main'* ]]
  # One per command the pipeline wraps in a devshell: build-meta.sh, Build.sol,
  # build, copy, fmt. build.sh is deliberately NOT wrapped — it picks its own
  # shell per command — and the looping binary comes from the composite instead.
  [ "$(grep -cF 'github:rainlanguage/rainix/${{ env.RAINIX_SHA }}' <<<"$pipeline")" -eq 5 ]
  # Comments stripped: the composite explains the pinned ref it does NOT use,
  # and prose naming the rejected form must not read as the form being used.
  [[ "$(grep -v '^[[:space:]]*#' <<<"$action_script")" != *'github:'* ]]
}
