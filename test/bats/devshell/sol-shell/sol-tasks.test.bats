@test "rainix-sol-test should be available on PATH" {
  run command -v rainix-sol-test
  [ "$status" -eq 0 ]
}

@test "rainix-sol-static should be available on PATH" {
  run command -v rainix-sol-static
  [ "$status" -eq 0 ]
}

@test "rainix-sol-legal should be available on PATH" {
  run command -v rainix-sol-legal
  [ "$status" -eq 0 ]
}

@test "rainix-sol-artifacts should be available on PATH" {
  run command -v rainix-sol-artifacts
  [ "$status" -eq 0 ]
}
