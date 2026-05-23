@test "jq should be available on PATH" {
  run jq --version
  [ "$status" -eq 0 ]
}
