setup() {
  TESTDIR="$(mktemp -d)"
  # Stub curl on PATH so no test touches the network. STUB_CURL_FAIL simulates
  # an unreachable registry; otherwise the stub prints the file named by
  # STUB_CURL_PAYLOAD as the registry response.
  mkdir -p "$TESTDIR/bin"
  cat > "$TESTDIR/bin/curl" << 'EOF'
#!/usr/bin/env bash
if [ -n "${STUB_CURL_FAIL:-}" ]; then
  exit 22
fi
cat "$STUB_CURL_PAYLOAD"
EOF
  chmod +x "$TESTDIR/bin/curl"
  PATH="$TESTDIR/bin:$PATH"
  SCRIPT="./lib/check-published-deploy-constants.sh"
}

teardown() {
  rm -rf "$TESTDIR"
}

_write_payload() {
  STUB_CURL_PAYLOAD="$TESTDIR/payload.json"
  export STUB_CURL_PAYLOAD
  cat > "$STUB_CURL_PAYLOAD"
}

# The flake task execs the script's nix store copy directly, and the store
# canonicalizes the git file mode (644 -> 0444, 755 -> 0555), so the tracked
# file must carry the exec bit or the task dies with EACCES.
@test "script is executable" {
  [ -x "$SCRIPT" ]
}

@test "malformed invocation prints usage to stderr and exits 1" {
  run "$SCRIPT" only-two-args "$TESTDIR/lib.sol"
  [ "$status" -eq 1 ]
  [[ "$output" == *"Usage: check-published-deploy-constants"* ]]
}

@test "unreachable registry prints connectivity SKIP and exits 0" {
  export STUB_CURL_FAIL=1
  run "$SCRIPT" somepkg "$TESTDIR/lib.sol" DEPLOYED
  [ "$status" -eq 0 ]
  [ "$output" = "SKIP: could not fetch published soldeer versions" ]
}

@test "reachable registry with no parseable versions prints a distinct SKIP and exits 0" {
  _write_payload << 'EOF'
{"data":[],"status":"success"}
EOF
  run "$SCRIPT" somepkg "$TESTDIR/lib.sol" DEPLOYED
  [ "$status" -eq 0 ]
  [ "$output" = "SKIP: no versions parsed from registry response" ]
}

@test "prints OK when every published version has its full constant suite" {
  _write_payload << 'EOF'
{"data":[{"version":"1.0.0"},{"version":"1.2.3"}]}
EOF
  cat > "$TESTDIR/lib.sol" << 'EOF'
address constant DEPLOYED_ADDRESS_1_0_0 = address(1);
bytes32 constant DEPLOYED_CODEHASH_1_0_0 = bytes32(0);
address constant DEPLOYED_ADDRESS_1_2_3 = address(2);
bytes32 constant DEPLOYED_CODEHASH_1_2_3 = bytes32(0);
EOF
  run "$SCRIPT" somepkg "$TESTDIR/lib.sol" DEPLOYED
  [ "$status" -eq 0 ]
  [ "$output" = "OK" ]
}

@test "prints MISSING with each absent constant name across prefixes and exits 0" {
  _write_payload << 'EOF'
{"data":[{"version":"1.0.0"}]}
EOF
  cat > "$TESTDIR/lib.sol" << 'EOF'
address constant OBV2_ADDRESS_1_0_0 = address(1);
bytes32 constant OBV2_CODEHASH_1_0_0 = bytes32(0);
address constant OBV3_ADDRESS_1_0_0 = address(2);
EOF
  run "$SCRIPT" somepkg "$TESTDIR/lib.sol" OBV2 OBV3
  [ "$status" -eq 0 ]
  [ "$output" = "MISSING: OBV3_CODEHASH_1_0_0" ]
}
