@test "anvil should be available on PATH" {
  run anvil --version
  [ "$status" -eq 0 ]
}
