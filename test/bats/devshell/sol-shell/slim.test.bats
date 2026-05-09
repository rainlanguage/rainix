# Verify the sol-shell stays slim — these heavy default-shell binaries
# must NOT be pulled in via nix. System-installed copies on a CI runner
# (e.g. cargo/node pre-installed by GitHub Actions) are not the concern;
# what we are guarding against is sol-shell's nix closure quietly
# growing to include them.

assert_not_from_nix_store() {
  local bin="$1"
  local resolved
  resolved="$(command -v "$bin" 2>/dev/null || true)"
  if [ -z "$resolved" ]; then
    return 0
  fi
  case "$resolved" in
    /nix/store/*)
      echo "FAIL: $bin resolves to a nix store path: $resolved" >&2
      return 1
      ;;
  esac
  return 0
}

@test "chromium should NOT come from nix (sol-shell stays slim)" {
  assert_not_from_nix_store chromium
}

@test "cargo should NOT come from nix (sol-shell stays slim)" {
  assert_not_from_nix_store cargo
}

@test "node should NOT come from nix (sol-shell stays slim)" {
  assert_not_from_nix_store node
}

@test "graph (the-graph) should NOT come from nix (sol-shell stays slim)" {
  assert_not_from_nix_store graph
}

@test "goldsky should NOT come from nix (sol-shell stays slim)" {
  assert_not_from_nix_store goldsky
}

@test "age should NOT come from nix (sol-shell stays slim)" {
  assert_not_from_nix_store age
}
