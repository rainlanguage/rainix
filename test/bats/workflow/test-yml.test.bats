setup() {
  static="$BATS_TEST_DIRNAME/../../../.github/workflows/rainix-sol-static.yaml"
  ci="$BATS_TEST_DIRNAME/../../../.github/workflows/test.yml"
  gate="$(yq -r '.jobs.static.steps[] | select(.run) | .run' "$static" | grep -o 'forge lint.*')"
  tasks="$(yq -r '.jobs.rainix.strategy.matrix.include[].task' "$ci")"
}

@test "rainix runs the consumers' forge lint gate over its own fixture" {
  [ -n "$gate" ]
  echo "$tasks" | grep -qxF "$gate"
}
