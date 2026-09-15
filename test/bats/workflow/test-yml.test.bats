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
