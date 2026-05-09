# Verify the sol-shell stays slim — these heavy default-shell binaries
# must NOT be present, otherwise the closure has bloated.

@test "chromium should NOT be on PATH (default-only)" {
  run command -v chromium
  [ "$status" -ne 0 ]
}

@test "cargo should NOT be on PATH (default-only)" {
  run command -v cargo
  [ "$status" -ne 0 ]
}

@test "node should NOT be on PATH (default-only)" {
  run command -v node
  [ "$status" -ne 0 ]
}

@test "graph (the-graph) should NOT be on PATH (default-only)" {
  run command -v graph
  [ "$status" -ne 0 ]
}

@test "goldsky should NOT be on PATH (default-only)" {
  run command -v goldsky
  [ "$status" -ne 0 ]
}

@test "age should NOT be on PATH (default-only)" {
  run command -v age
  [ "$status" -ne 0 ]
}
