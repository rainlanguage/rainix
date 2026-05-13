# Closure-level slim assertion for rust-shell.
#
# Mirror of sol-shell/closure.test.bats. Runtime-PATH checks miss leaks
# where a heavy package is interpolated into a script string — the
# script does not appear on PATH, but its referenced store paths are
# still in rust-shell's nix closure and are downloaded/built every
# time CI evaluates the shell.
#
# This file inspects rust-shell's full closure (`nix-store -q
# --requisites`) and fails if slither, solc, node/npm, or chromium
# show up. rust-shell ships foundry-bin so that alloy's Anvil::new()
# spawn works in cargo tests, but does not pull in the rest of the
# Solidity static-analysis toolchain.
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
  RUST_SHELL_OUT="$(nix build --no-link --print-out-paths "$REPO_ROOT#devShells.$SYSTEM.rust-shell")"
  CLOSURE="$(nix-store -q --requisites "$RUST_SHELL_OUT")"
}

@test "rust-shell closure has no slither or solc" {
  local hits
  hits="$(echo "$CLOSURE" | grep -E '/nix/store/[^/]*-(solc-static-|slither-analyzer-)' || true)"
  if [ -n "$hits" ]; then
    echo "FAIL: rust-shell closure references sol static-analysis components:" >&2
    echo "$hits" >&2
    return 1
  fi
}

@test "rust-shell closure has no node/npm" {
  local hits
  hits="$(echo "$CLOSURE" | grep -E '/nix/store/[^/]*-(nodejs-|nodejs_|npm-)' || true)"
  if [ -n "$hits" ]; then
    echo "FAIL: rust-shell closure references nodejs:" >&2
    echo "$hits" >&2
    return 1
  fi
}

@test "rust-shell closure has no chromium" {
  local hits
  hits="$(echo "$CLOSURE" | grep -E '/nix/store/[^/]*-chromium-' || true)"
  if [ -n "$hits" ]; then
    echo "FAIL: rust-shell closure references chromium:" >&2
    echo "$hits" >&2
    return 1
  fi
}
