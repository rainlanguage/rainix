# Single quoting is load-bearing throughout this file: the injection test hands
# the action a pipeline whose `$(…)` must arrive unexpanded, which is the whole
# property under test. SC2016 asks whether that is a mistake; here it is the
# point.
# shellcheck disable=SC2016

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/codegen-fixed-point/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  workflow="$repo_root/.github/workflows/rainix-copy-artifacts.yaml"
}

# Runs the composite's own script with `nix` stubbed to echo its argv, so what
# is asserted is the command line the action builds rather than a reimplementation
# of it. The loop itself is covered by the Rust unit tests; what can only break
# here is the wiring between the two.
run_codegen_action() {
  local max_passes="$1"
  local pipeline="$2"

  RAINIX_CODEGEN_MAX_PASSES="$max_passes" \
    RAINIX_CODEGEN_RUN="$pipeline" \
    GITHUB_ACTION_PATH="$repo_root/.github/actions/codegen-fixed-point" \
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

@test "the subcommand and both flags reach the binary" {
  run run_codegen_action 5 'forge fmt'

  [ "$status" -eq 0 ]
  [[ "$output" == *"<codegen-fixed-point>"* ]]
  [[ "$output" == *"<--max-passes> <5>"* ]]
  [[ "$output" == *"<--run> <forge fmt>"* ]]
}

@test "the bound is the caller's, not a number baked into the action" {
  run run_codegen_action 9 'forge fmt'

  [ "$status" -eq 0 ]
  [[ "$output" == *"<--max-passes> <9>"* ]]
  [[ "$output" != *"<--max-passes> <5>"* ]]
}

# The binary hands the string to `bash -c` whole. Word-splitting it here would
# turn a multi-command pipeline into a subcommand plus a pile of stray argv.
@test "a multi-line pipeline arrives as exactly one argument" {
  run run_codegen_action 5 'set -eu
forge build
forge fmt'

  [ "$status" -eq 0 ]
  [[ "$output" == *"<--run> <set -eu
forge build
forge fmt>"* ]]
  # One `<...>` opens after --run and the line ends at its close: nothing
  # spilled into further arguments.
  [[ "$output" == *"forge fmt>" ]]
}

# The pipeline is caller input reaching a reusable workflow from arbitrary
# repos. The action must pass it along, never evaluate it.
@test "the action never executes the pipeline it is handed" {
  canary="$BATS_TEST_TMPDIR/canary"

  run run_codegen_action 5 "\$(touch '$canary') \`touch '$canary.tick'\`"

  [ "$status" -eq 0 ]
  [ ! -e "$canary" ]
  [ ! -e "$canary.tick" ]
  [[ "$output" == *'<--run> <$(touch'* ]]
}

# The action deliberately carries no default, so the bound has exactly one
# published default — the reusable workflow input the README documents. A
# default reappearing here is a second number to keep in step.
@test "the bound is defaulted in exactly one place" {
  run yq -e '.inputs["max-passes"] | has("default")' "$action"
  [ "$status" -ne 0 ]

  run yq -r '.inputs["max-passes"].required' "$action"
  [ "$output" = "true" ]

  run yq -r '.on.workflow_call.inputs["max-codegen-passes"].default' "$workflow"
  [ "$output" = "5" ]
}

# `github:…/$RAINIX_SHA` cannot name a commit that contains a subcommand the
# same PR adds, so the step would fail with "unknown subcommand" from the moment
# it merged until a follow-up bumped the pin. The path: ref resolves the binary
# from this composite's own checkout, which is what removes that window.
#
# Asserted against the argv the stub records rather than the script text, so it
# is the ref actually passed to nix that is pinned here — and the `../../..`
# arithmetic has to land on the repo root for it to hold.
@test "the binary is resolved from the action's own checkout, not a pinned flake" {
  run run_codegen_action 5 'forge fmt'

  [ "$status" -eq 0 ]
  [[ "$output" == "nix <run> <path:$(cd "$repo_root" && pwd)#rainix-static> <-->"* ]]
  [[ "$output" != *"github:"* ]]
}

@test "the reusable workflow calls the action rather than the binary directly" {
  run yq -r '.jobs.copy-artifacts.steps[] | select(.uses | test("codegen-fixed-point")) | .uses' "$workflow"

  [ "$status" -eq 0 ]
  [ "$output" = "rainlanguage/rainix/.github/actions/codegen-fixed-point@main" ]
}
