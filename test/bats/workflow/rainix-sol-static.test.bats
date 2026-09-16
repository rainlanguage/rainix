# Nothing in this repo executes rainix-sol-static.yaml — it is `workflow_call`
# only, so its only runners are the consumer repos — which means a step
# silently dropped from it goes unnoticed here and ungated everywhere.
#
# The behaviour of each gate is covered in test/bats/devshell/sol-shell; what
# is asserted here is that the job still invokes them, and invokes them through
# the pinned shell rather than an unpinned `github:rainlanguage/rainix#…` HEAD
# ref (CLAUDE.md).

setup() {
  workflow="$BATS_TEST_DIRNAME/../../../.github/workflows/rainix-sol-static.yaml"
  runs="$(yq -r '.jobs.static.steps[] | select(.run) | .run' "$workflow")"
  uses="$(yq -r '.jobs.static.steps[] | select(.uses) | .uses' "$workflow")"
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
}

# audit/mutation-test-scans.json is hand-appended and read as evidence by the
# audit skill and by rain-org-health's scanner, neither of which validates it.
@test "rainix-sol-static gates the mutation-test ledger" {
  echo "$uses" | grep -q '^rainlanguage/rainix/.github/actions/mutation-ledger@main$'
}

@test "rainix-sol-static runs forge lint denying warnings" {
  echo "$runs" | grep -q '#sol-shell -c forge lint -D warnings$'
}

@test "rainix-sol-static runs the pre-commit hook bundle over all files" {
  echo "$runs" | grep -q '#sol-shell -c pre-commit run --all-files'
}

@test "every rainix-sol-static run step resolves rainix through the pinned sha" {
  [ -n "$sha" ]
  [ "$sha" != "null" ]
  local unpinned
  unpinned="$(echo "$runs" | grep 'github:rainlanguage/rainix' | grep -v 'env.RAINIX_SHA' || true)"
  if [ -n "$unpinned" ]; then
    echo "FAIL: unpinned rainix refs in rainix-sol-static.yaml:" >&2
    echo "$unpinned" >&2
    return 1
  fi
}

# The job runs `slither .`, which shells out to `forge clean` and then builds
# `--skip ./test/** ./script/**`, and `forge lint`, which writes AST-only
# artifacts with no bytecode. Neither leaves an `out/` a `forge test` can use,
# and `foundry-full-` is one namespace every sol workflow prefix-matches, so a
# save from here is restored by rainix-sol-test — which then skips compilation
# and reverts every `vm.getCode` against a test/ or script/ contract.
@test "rainix-sol-static writes no shared foundry build cache" {
  local cache_steps
  cache_steps="$(yq -r '.jobs.static.steps[] | select(.uses) | .uses' "$workflow" | grep 'actions/cache' || true)"
  if [ -n "$cache_steps" ]; then
    echo "FAIL: rainix-sol-static caches a build it only ever leaves partial:" >&2
    echo "$cache_steps" >&2
    return 1
  fi
}
