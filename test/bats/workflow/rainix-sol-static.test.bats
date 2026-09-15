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
