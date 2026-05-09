@test "reuse should be available on PATH" {
  run reuse --version
  [ "$status" -eq 0 ]
}
