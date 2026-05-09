# Closure-level slim assertion.
#
# Runtime-PATH checks (slim.test.bats) miss leaks where a heavy package
# is interpolated into a script string — the script does not appear on
# PATH, but its referenced store paths are still in sol-shell's nix
# closure and are downloaded/built every time CI evaluates the shell.
#
# This file inspects sol-shell's full closure (`nix-store -q
# --requisites`) and fails if any rust toolchain component shows up.
# It exists specifically to pin the rustfmt-conditional fix: the hook
# previously referenced ${rust-toolchain}/bin/cargo-fmt which dragged
# rustc, rust-src, rust-std-wasm32, rust-analyzer, clippy, and rustfmt
# into every consumer of sol-shell.
#
# Skips when `nix` is not on PATH (e.g. running the bats files under a
# stripped sandbox); CI invokes via nix and so always exercises this.

setup() {
  if ! command -v nix >/dev/null 2>&1; then
    skip "nix not on PATH"
  fi
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
  SYSTEM="$(nix eval --raw --impure --expr 'builtins.currentSystem')"
  # Realise the shell so we can query its runtime closure (the drv-level
  # closure is full of build-time bootstrap deps that never appear in the
  # realized environment, which produces noisy false positives).
  SOL_SHELL_OUT="$(nix build --no-link --print-out-paths "$REPO_ROOT#devShells.$SYSTEM.sol-shell")"
  CLOSURE="$(nix-store -q --requisites "$SOL_SHELL_OUT")"
}

@test "sol-shell closure has no rust toolchain" {
  # Match rust toolchain packages by their version-suffixed names so we
  # do not match arbitrary scripts that happen to contain the word
  # "rustfmt" (e.g. our rustfmt-conditional gating script).
  local hits
  hits="$(echo "$CLOSURE" | grep -E '/nix/store/[^/]*-(rust-default-[0-9]|rustc-[0-9]|rust-src-[0-9]|rust-std-[0-9]|rust-analyzer-[0-9]|rust-docs-[0-9]|rustfmt-[0-9]|clippy-[0-9])' || true)"
  if [ -n "$hits" ]; then
    echo "FAIL: sol-shell closure references rust toolchain components:" >&2
    echo "$hits" >&2
    return 1
  fi
}

@test "sol-shell closure has no node/npm" {
  local hits
  hits="$(echo "$CLOSURE" | grep -E '/nix/store/[^/]*-(nodejs-|nodejs_|npm-)' || true)"
  if [ -n "$hits" ]; then
    echo "FAIL: sol-shell closure references nodejs:" >&2
    echo "$hits" >&2
    return 1
  fi
}

@test "sol-shell closure has no chromium" {
  local hits
  hits="$(echo "$CLOSURE" | grep -E '/nix/store/[^/]*-chromium-' || true)"
  if [ -n "$hits" ]; then
    echo "FAIL: sol-shell closure references chromium:" >&2
    echo "$hits" >&2
    return 1
  fi
}
