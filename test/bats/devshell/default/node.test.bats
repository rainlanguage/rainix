@test "node is the 21 October 2026 patched runtime" {
  run node --version
  [ "$status" -eq 0 ]
  [ "$output" = "v22.23.4" ]
}

@test "npm runs and is bound to the packaged node, not PATH" {
  run npm --version
  [ "$status" -eq 0 ]
  run head -1 "$(readlink -f "$(command -v npm)")"
  [[ "$output" == "#!/nix/store/"*"-nodejs-22.23.4/bin/node" ]]
}

# graph-cli's tarball bundles Node 20 and its wrapper prefers that over PATH.
@test "graph runs on the patched node, not its bundled Node 20" {
  run graph --version
  [ "$status" -eq 0 ]
  [[ "$output" == *"node-v22.23.4"* ]]
}
