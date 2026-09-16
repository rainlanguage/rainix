# rainix's own CI eating the gate it hands every consumer. The claim is made in
# a comment beside the matrix entry and by nothing else: drop the task and the
# fixture consumer stops being linted, with rainix still shipping the gate to 38
# repos it no longer runs itself.
#
# The expected task is read out of rainix-sol-static rather than spelled out
# here, so the two cannot drift apart in either direction: change the gate's
# flags on one side and this goes red until the other side follows.

setup() {
  static="$BATS_TEST_DIRNAME/../../../.github/workflows/rainix-sol-static.yaml"
  ci="$BATS_TEST_DIRNAME/../../../.github/workflows/test.yml"
  # The gate as rainix-sol-static runs it, minus the `nix develop <pinned
  # shell> -c` prefix — which is exactly the task string test.yml's matrix
  # hands to its own `nix develop ../..`.
  gate="$(yq -r '.jobs.static.steps[] | select(.run) | .run' "$static" | grep -o 'forge lint.*')"
  tasks="$(yq -r '.jobs.rainix.strategy.matrix.include[].task' "$ci")"
}

@test "rainix runs the consumers' forge lint gate over its own fixture" {
  [ -n "$gate" ]
  echo "$tasks" | grep -qxF "$gate"
}

# The matrix shares ONE `foundry-full-` cache across every task, so the tasks
# that leave `out/` partial — `slither .` builds `--skip ./test/** ./script/**`,
# `forge lint` writes AST-only artifacts with no bytecode — must stay out of it
# or `forge test -vvv` restores their leftovers and skips compilation. The gate
# string is read from rainix-sol-static, as above, so the two cannot drift.
@test "test.yml keeps the partial-build tasks out of the shared foundry cache" {
  local guard
  guard="$(yq -r '.jobs.rainix.steps[] | select(.name == "Cache Foundry build") | .["if"]' "$ci")"
  [ -n "$guard" ]
  [ "$guard" != "null" ]
  [ -n "$gate" ]
  for partial in "$gate" 'slither .'; do
    if echo "$guard" | grep -qF -- "$partial"; then
      echo "FAIL: '$partial' leaves out/ partial but is in the cache allowlist:" >&2
      echo "$guard" >&2
      return 1
    fi
  done
}
