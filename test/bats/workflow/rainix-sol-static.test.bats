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
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
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
