@test "slither should be available on PATH" {
  run slither --version
  [ "$status" -eq 0 ]
}
