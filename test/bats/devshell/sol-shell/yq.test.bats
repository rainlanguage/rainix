@test "yq should be available on PATH" {
  run yq --version
  [ "$status" -eq 0 ]
}
