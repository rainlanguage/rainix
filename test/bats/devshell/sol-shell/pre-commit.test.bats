setup() {
  if ! command -v pre-commit >/dev/null 2>&1; then
    skip "pre-commit not on PATH"
  fi
  CONFIG="$BATS_TEST_DIRNAME/../../../../.pre-commit-config.yaml"
  if [ ! -e "$CONFIG" ]; then
    skip "no generated .pre-commit-config.yaml; enter the devShell first"
  fi
  TESTDIR="$(mktemp -d)"
  git init -q "$TESTDIR"
  cp -L "$CONFIG" "$TESTDIR/.pre-commit-config.yaml"
}

teardown() {
  rm -rf "$TESTDIR"
}

write_dirty_markdown() {
  cat > "$TESTDIR/README.md" <<'EOF'
# Subject

This sentence is deliberately far longer than eighty columns so that the denofmt hook has a reflow to make when it runs over it.
EOF
  git -C "$TESTDIR" add README.md
}

@test "pre-commit run --all-files fails on a hook-dirty tracked file" {
  write_dirty_markdown
  cd "$TESTDIR"
  run pre-commit run --all-files --color never
  [ "$status" -ne 0 ]
  echo "$output" | grep -q '^denofmt.*Failed'
}

@test "pre-commit run --all-files passes once the hook has rewritten the file" {
  write_dirty_markdown
  cd "$TESTDIR"
  run pre-commit run --all-files --color never
  [ "$status" -ne 0 ]

  run pre-commit run --all-files --color never
  [ "$status" -eq 0 ]
}

@test "JSON is not gated in sol-shell because prettier-rainix no-ops there" {
  [ -z "${RAINIX_PRETTIER_BUNDLE_DIR:-}" ]
  printf '{\n    "a":1,\n    "b":  2}' > "$TESTDIR/config.json"
  git -C "$TESTDIR" add config.json
  cd "$TESTDIR"
  run pre-commit run --all-files --color never
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^prettier-rainix'
}
