# Nothing in this repo executes rainix-rs-static.yaml — it is `workflow_call`
# only, so its only runners are the consumer repos, and a step silently dropped
# from it goes unnoticed here and ungated everywhere.

setup() {
  workflow="$BATS_TEST_DIRNAME/../../../.github/workflows/rainix-rs-static.yaml"
  uses="$(yq -r '.jobs.rs-static.steps[] | select(.uses) | .uses' "$workflow")"
}

# The ledger is a JSON record, not a Rust or a Solidity artifact. Gating it in
# the sol job alone would leave a rust-only repo that starts mutation-testing
# with no check at all.
@test "rainix-rs-static gates the mutation-test ledger" {
  echo "$uses" | grep -q '^rainlanguage/rainix/.github/actions/mutation-ledger@main$'
}
