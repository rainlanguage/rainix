@test "gh should be available on PATH" {
  run gh --version
  [ "$status" -eq 0 ]
}
