@test "node is the 21 October 2026 patched runtime" {
  run node --version
  [ "$status" -eq 0 ]
  [ "$output" = "v22.23.4" ]
}
