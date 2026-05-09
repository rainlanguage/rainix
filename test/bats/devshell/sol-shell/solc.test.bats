@test "solc-0.8.25 should be available on PATH" {
  run solc-0.8.25 --version
  [ "$status" -eq 0 ]
  [[ "$output" == *"0.8.25"* ]]
}
