@test "chromium should be available on PATH" {
  run chromium --version
  [ "$status" -eq 0 ]
}
