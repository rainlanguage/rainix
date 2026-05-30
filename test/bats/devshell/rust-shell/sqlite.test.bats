@test "sqlite3 should be available on PATH" {
  run sqlite3 --version
  [ "$status" -eq 0 ]
}
