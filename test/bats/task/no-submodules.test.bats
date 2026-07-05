setup() {
  # shellcheck disable=SC1091
  source lib/no-submodules.sh
  TESTDIR="$(mktemp -d)"
  git -C "$TESTDIR" init -q
}

teardown() {
  rm -rf "$TESTDIR"
}

@test "clean repo passes" {
  touch "$TESTDIR/README.md"
  run check_no_submodules "$TESTDIR"
  [ "$status" -eq 0 ]
  [[ "$output" == *"no-submodules: clean"* ]]
}

@test "root .gitmodules fails and is named" {
  printf '[submodule "lib/forge-std"]\n\tpath = lib/forge-std\n' > "$TESTDIR/.gitmodules"
  run check_no_submodules "$TESTDIR"
  [ "$status" -eq 1 ]
  [[ "$output" == *".gitmodules"* ]]
}

@test "committed gitlink without .gitmodules fails" {
  # Stage a bare gitlink entry (mode 160000) directly — a submodule pointer
  # whose .gitmodules was deleted but whose tree entry survives.
  git -C "$TESTDIR" update-index --add --cacheinfo 160000,0000000000000000000000000000000000000001,lib/ghost
  run check_no_submodules "$TESTDIR"
  [ "$status" -eq 1 ]
  [[ "$output" == *"lib/ghost"* ]]
  [[ "$output" == *"160000"* ]]
}

@test "vendored non-root .gitmodules is inert and passes" {
  mkdir -p "$TESTDIR/dependencies/dep"
  printf '[submodule "x"]\n' > "$TESTDIR/dependencies/dep/.gitmodules"
  run check_no_submodules "$TESTDIR"
  [ "$status" -eq 0 ]
}

@test "non-git directory with .gitmodules still fails on the file leg" {
  rm -rf "$TESTDIR/.git"
  printf '[submodule "x"]\n' > "$TESTDIR/.gitmodules"
  run check_no_submodules "$TESTDIR"
  [ "$status" -eq 1 ]
}
