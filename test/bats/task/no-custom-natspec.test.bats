setup() {
  # shellcheck disable=SC1091
  source lib/no-custom-natspec.sh
  TESTDIR="$(mktemp -d)"
}

teardown() {
  rm -rf "$TESTDIR"
}

@test "clean tree passes" {
  mkdir -p "$TESTDIR/src"
  echo 'contract A {}' > "$TESTDIR/src/A.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 0 ]
  [[ "$output" == *"no-custom-natspec: clean"* ]]
}

@test "custom error tag fails and names the file" {
  mkdir -p "$TESTDIR/src"
  printf '/// @custom:error Reverts with Foo.\ncontract A {}\n' > "$TESTDIR/src/A.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 1 ]
  [[ "$output" == *"src/A.sol"* ]]
  [[ "$output" == *"@custom:error"* ]]
}

@test "exact ERC-7201 storage-location form passes" {
  mkdir -p "$TESTDIR/src"
  printf '/// @custom:storage-location erc7201:rain.storage.Thing\ncontract A {}\n' > "$TESTDIR/src/A.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 0 ]
}

@test "near-miss ERC-7201 forms fail: double space, erc-7201, uppercase" {
  mkdir -p "$TESTDIR/src"
  printf '/// @custom:storage-location  erc7201:x\ncontract A {}\n' > "$TESTDIR/src/A.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 1 ]
  printf '/// @custom:storage-location erc-7201:x\ncontract B {}\n' > "$TESTDIR/src/A.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 1 ]
  printf '/// @custom:storage-location ERC7201:x\ncontract C {}\n' > "$TESTDIR/src/A.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 1 ]
}

@test "root-level vendor dirs are excluded" {
  mkdir -p "$TESTDIR/dependencies/dep" "$TESTDIR/lib/dep"
  printf '/// @custom:error vendored\ncontract V {}\n' > "$TESTDIR/dependencies/dep/V.sol"
  printf '/// @custom:error vendored\ncontract W {}\n' > "$TESTDIR/lib/dep/W.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 0 ]
}

@test "nested first-party lib dirs are NOT excluded (root-anchoring)" {
  mkdir -p "$TESTDIR/test/lib"
  printf '/// @custom:error first-party\ncontract T {}\n' > "$TESTDIR/test/lib/T.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 1 ]
  [[ "$output" == *"test/lib/T.sol"* ]]
}

@test "node_modules is excluded" {
  mkdir -p "$TESTDIR/node_modules/pkg"
  printf '/// @custom:error dep\ncontract N {}\n' > "$TESTDIR/node_modules/pkg/N.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 0 ]
}

@test "non-Solidity files are ignored" {
  printf '@custom:error in markdown\n' > "$TESTDIR/NOTES.md"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 0 ]
}

@test "allowed and banned tags in one file: fails listing only the banned line" {
  mkdir -p "$TESTDIR/src"
  printf '/// @custom:storage-location erc7201:rain.storage.Thing\n/// @custom:security ping me\ncontract A {}\n' > "$TESTDIR/src/A.sol"
  run check_no_custom_natspec "$TESTDIR"
  [ "$status" -eq 1 ]
  [[ "$output" == *"@custom:security"* ]]
  [[ "$output" != *"rain.storage.Thing"* ]]
}
