@test "cargo expand should be available on PATH" {
  run cargo expand --version
  [ "$status" -eq 0 ]
}
