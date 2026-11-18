@test "node is nodejs_22 at or past the 21 October 2026 security release" {
  run node --version
  [ "$status" -eq 0 ]
  [[ "$output" == v22.* ]]
  printf '%s\n' v22.23.4 "$output" | sort -V -C
}

@test "npm runs and is bound to the packaged node, not PATH" {
  run npm --version
  [ "$status" -eq 0 ]
  run head -1 "$(readlink -f "$(command -v npm)")"
  [ "$output" = "#!$(readlink -f "$(command -v node)")" ]
}

# graph-cli's tarball bundles Node 20 and its wrapper prefers that over PATH.
@test "graph runs on the shell's node, not its bundled Node 20" {
  run graph --version
  [ "$status" -eq 0 ]
  [[ "$output" == *"node-$(node --version)"* ]]
}
