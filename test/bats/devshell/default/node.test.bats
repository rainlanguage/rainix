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
