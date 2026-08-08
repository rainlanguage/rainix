setup() {
  # shellcheck disable=SC1091
  source lib/check-rainix-flake-pin.sh
  TESTDIR="$(mktemp -d)"
  mkdir -p "$TESTDIR"
}

teardown() {
  rm -rf "$TESTDIR"
}

# ── helper ──────────────────────────────────────────────────────────────────

_write_workflow() {
  local file="$1"
  local sha="$2"
  cat > "$file" << EOF
on: [workflow_call]
env:
  RAINIX_SHA: $sha
  OTHER_VAR: foo
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: nix develop github:rainlanguage/rainix/\${{ env.RAINIX_SHA }}#sol-shell -c forge test
EOF
}

# ── extract sha ─────────────────────────────────────────────────────────────

@test "_extract_sha returns the SHA from a workflow file" {
  _write_workflow "$TESTDIR/rainix-sol-test.yaml" "abc123def456"
  run _extract_sha "$TESTDIR/rainix-sol-test.yaml"
  [ "$status" -eq 0 ]
  [ "$output" = "abc123def456" ]
}

@test "_extract_sha returns empty string when RAINIX_SHA is absent" {
  cat > "$TESTDIR/rainix-no-sha.yaml" << 'EOF'
on: [workflow_call]
jobs:
  test:
    runs-on: ubuntu-latest
EOF
  run _extract_sha "$TESTDIR/rainix-no-sha.yaml"
  [ "$output" = "" ]
}

# ── count unpinned refs ──────────────────────────────────────────────────────

@test "_count_unpinned_refs returns 0 when no bare refs present" {
  _write_workflow "$TESTDIR/rainix-sol-test.yaml" "abc123"
  run _count_unpinned_refs "$TESTDIR"
  [ "$status" -eq 0 ]
  [ "$output" = "0" ]
}

@test "_count_unpinned_refs counts bare unpinned refs" {
  cat > "$TESTDIR/rainix-bad.yaml" << 'EOF'
on: [workflow_call]
env:
  RAINIX_SHA: abc123
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: nix develop github:rainlanguage/rainix#sol-shell -c forge test
EOF
  run _count_unpinned_refs "$TESTDIR"
  [ "$status" -eq 0 ]
  [ "$output" = "1" ]
}

# ── consistency check ────────────────────────────────────────────────────────

@test "check_rainix_flake_pin_consistent passes when all files have the same SHA" {
  local sha="307bf27fcc5a410994f5a6a6a96527a64625c3da"
  _write_workflow "$TESTDIR/rainix-sol-test.yaml" "$sha"
  _write_workflow "$TESTDIR/rainix-rs-test.yaml" "$sha"
  _write_workflow "$TESTDIR/rainix-copy-artifacts.yaml" "$sha"
  run check_rainix_flake_pin_consistent "$TESTDIR"
  [ "$status" -eq 0 ]
}

@test "check_rainix_flake_pin_consistent fails when SHAs differ between files" {
  _write_workflow "$TESTDIR/rainix-sol-test.yaml" "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  _write_workflow "$TESTDIR/rainix-rs-test.yaml" "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  run check_rainix_flake_pin_consistent "$TESTDIR"
  [ "$status" -ne 0 ]
  [[ "$output" == *"SHA mismatch"* ]]
}

@test "check_rainix_flake_pin_consistent fails when a file is missing RAINIX_SHA" {
  _write_workflow "$TESTDIR/rainix-sol-test.yaml" "abc123"
  cat > "$TESTDIR/rainix-no-sha.yaml" << 'EOF'
on: [workflow_call]
jobs:
  test:
    runs-on: ubuntu-latest
EOF
  run check_rainix_flake_pin_consistent "$TESTDIR"
  [ "$status" -ne 0 ]
  [[ "$output" == *"RAINIX_SHA not found"* ]]
}

@test "check_rainix_flake_pin_consistent fails on bare unpinned ref" {
  _write_workflow "$TESTDIR/rainix-sol-test.yaml" "abc123"
  cat > "$TESTDIR/rainix-bad.yaml" << 'EOF'
on: [workflow_call]
env:
  RAINIX_SHA: abc123
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: nix develop github:rainlanguage/rainix#sol-shell -c forge test
EOF
  run check_rainix_flake_pin_consistent "$TESTDIR"
  [ "$status" -ne 0 ]
  [[ "$output" == *"bare unpinned"* ]]
}

@test "check_rainix_flake_pin_consistent fails when directory has no rainix-*.yaml files" {
  run check_rainix_flake_pin_consistent "$TESTDIR"
  [ "$status" -ne 0 ]
  [[ "$output" == *"no rainix-*.yaml files found"* ]]
}

# ── mixed-file smoke test ────────────────────────────────────────────────────

@test "check_rainix_flake_pin_consistent passes with three files sharing a full 40-char sha" {
  local sha="307bf27fcc5a410994f5a6a6a96527a64625c3da"
  _write_workflow "$TESTDIR/rainix-sol-test.yaml" "$sha"
  _write_workflow "$TESTDIR/rainix-rs-static.yaml" "$sha"
  _write_workflow "$TESTDIR/rainix-subgraph-test.yaml" "$sha"
  run check_rainix_flake_pin_consistent "$TESTDIR"
  [ "$status" -eq 0 ]
}
