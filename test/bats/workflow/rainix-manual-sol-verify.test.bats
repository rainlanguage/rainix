# rainix-manual-sol-verify.yaml is `workflow_call` only and submits to live
# explorers, so its verify step runs here with nix and sleep stubbed.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-manual-sol-verify.yaml"
  verify="$(yq -r '.jobs.verify.steps[] | select(.name == "Verify") | .run' "$workflow")"
  printf '[etherscan]\nzerog = { key = "x", chain = 16661, url = "https://x" }\n' >"$BATS_TEST_TMPDIR/foundry.toml"
}

# The verify step as bash runs it, with every expression stubbed, constructor
# args $1, and a nix that logs its arguments and exits $2.
run_verify() {
  local script="$verify"
  while [[ "$script" =~ \$\{\{[^}]*\}\} ]]; do
    script="${script/"${BASH_REMATCH[0]}"/x}"
  done
  (
    cd "$BATS_TEST_TMPDIR" || exit 1
    VERIFY_CONTRACT=src/A.sol:A VERIFY_ADDRESS=0x3A7387a484d87Aa8bBA45E98AAB401Ce4FBF03E2 \
      VERIFY_CONSTRUCTOR_ARGS="$1" NIX_STATUS="$2" CALLS="$BATS_TEST_TMPDIR/calls" VERIFY_SCRIPT="$script" bash -c '
        nix() { printf "%s\n" "$*" >>"$CALLS"; return "$NIX_STATUS"; }
        sleep() { :; }
        export -f nix sleep
        bash -c "$VERIFY_SCRIPT"
      '
  )
}

@test "constructor args reach the explorer submission" {
  run_verify 0x0000000000000000000000000000000000000000000000000000000000000001 0
  grep -q -- "forge verify-contract --chain 16661 0x3A7387a484d87Aa8bBA45E98AAB401Ce4FBF03E2 src/A.sol:A --constructor-args 0x0000000000000000000000000000000000000000000000000000000000000001 " "$BATS_TEST_TMPDIR/calls"
}

@test "constructor args reach the Sourcify retry" {
  run run_verify 0x01 1
  [ "$status" -ne 0 ]
  grep -q -- "--verifier sourcify .* --constructor-args 0x01 " "$BATS_TEST_TMPDIR/calls"
}

@test "no constructor args sends no --constructor-args" {
  run_verify '' 0
  grep -q -- "forge verify-contract" "$BATS_TEST_TMPDIR/calls"
  run grep -c -- "--constructor-args" "$BATS_TEST_TMPDIR/calls"
  [ "$output" = 0 ]
}
