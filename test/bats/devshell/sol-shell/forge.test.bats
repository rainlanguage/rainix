@test "forge should be available on PATH" {
  run forge --version
  [ "$status" -eq 0 ]
}
