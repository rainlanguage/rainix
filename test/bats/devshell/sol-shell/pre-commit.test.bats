# The `pre-commit run --all-files` gate that rainix-sol-static runs over every
# sol consumer, exercised through the config git-hooks.nix actually generates.
#
# The config is copied out of this repo (dereferenced — it is a symlink into
# the nix store) into a throwaway git repo and left UNTRACKED there, which is
# how it exists in every consumer: written at devShell entry, gitignored, never
# committed. Untracked also keeps the config out of its own `--all-files` set,
# which walks `git ls-files`.
#
# The hook set is whatever sol-shell resolves, not a list restated here, so a
# hook added or dropped in flake.nix does not silently fall out of the gate.

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

# Prose well past the 80 columns denofmt wraps at, so the hook has something to
# rewrite. The expected bytes are never spelled out here: the hook binary
# pinned in the generated config decides them.
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

# The other direction, without hard-coding what "formatted" means: the failing
# run rewrites the file in place, so the second run of the same command over
# the same tree is the clean case.
@test "pre-commit run --all-files passes once the hook has rewritten the file" {
  write_dirty_markdown
  cd "$TESTDIR"
  run pre-commit run --all-files --color never
  [ "$status" -ne 0 ]

  run pre-commit run --all-files --color never
  [ "$status" -eq 0 ]
}

# Coverage limit of running the gate in sol-shell, pinned so the claim cannot
# rot: prettier-rainix resolves its binary through RAINIX_PRETTIER_BUNDLE_DIR,
# which only the default shell exports, so the hook no-ops here and JSON — the
# one file type denofmt excludes and prettier-rainix owns — is not gated. If
# sol-shell ever gains the bundle this test fails, which is the signal to
# restate the gate's coverage rather than to relax the test.
@test "JSON is not gated in sol-shell because prettier-rainix no-ops there" {
  [ -z "${RAINIX_PRETTIER_BUNDLE_DIR:-}" ]
  printf '{\n    "a":1,\n    "b":  2}' > "$TESTDIR/config.json"
  git -C "$TESTDIR" add config.json
  cd "$TESTDIR"
  run pre-commit run --all-files --color never
  [ "$status" -eq 0 ]
  echo "$output" | grep -q '^prettier-rainix'
}
