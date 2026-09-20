setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-copy-artifacts.yaml"
  witness="$repo_root/rainix-static/src/codegen_witness.rs"
  names="$(yq -r '.jobs["copy-artifacts"].steps[] | .name // "«unnamed»"' "$workflow")"
  runs="$(yq -r '.jobs["copy-artifacts"].steps[] | select(.run) | .run' "$workflow")"
  mark_step="Mark tracked files before codegen"
  verify_step="Assert every declared generated file was written"
}

step_at() {
  echo "$names" | grep -nxF "$1" | cut -d: -f1
}

step_field() {
  yq -r ".jobs[\"copy-artifacts\"].steps[] | select(.name == \"$1\") | $2" "$workflow"
}

@test "the job invokes both codegen-witness phases" {
  [ "$(step_field "$mark_step" .uses)" = "rainlanguage/rainix/.github/actions/codegen-witness@main" ]
  [ "$(step_field "$mark_step" .with.phase)" = "mark" ]
  [ "$(step_field "$verify_step" .uses)" = "rainlanguage/rainix/.github/actions/codegen-witness@main" ]
  [ "$(step_field "$verify_step" .with.phase)" = "verify" ]
}

@test "both witness phases bracket every codegen hook step" {
  local mark verify
  mark="$(step_at "$mark_step")"
  verify="$(step_at "$verify_step")"
  [ -n "$mark" ]
  [ -n "$verify" ]
  [ "$mark" -lt "$verify" ]

  local hooked
  hooked="$(yq -r '.jobs["copy-artifacts"].steps | to_entries[]
    | select((.value.run // "") | test("\./script/(build-meta\.sh|Build\.sol|CopyArtifacts\.sol|build\.sh)"))
    | (.key + 1 | tostring) + " " + (.value.name // "«unnamed»")' "$workflow")"
  [ -n "$hooked" ]

  local pos name
  while read -r pos name; do
    if [ "$pos" -lt "$mark" ] || [ "$pos" -gt "$verify" ]; then
      echo "FAIL: codegen step '$name' (position $pos) is outside the witness window ($mark..$verify)" >&2
      return 1
    fi
  done <<<"$hooked"
}

@test "the witness closes before forge fmt runs" {
  local verify fmt
  verify="$(step_at "$verify_step")"
  fmt="$(echo "$names" | grep -n 'Format' | cut -d: -f1)"
  [ -n "$fmt" ]
  [ "$verify" -lt "$fmt" ]
}

@test "the committed-artifacts diff is still asserted after the witness" {
  local verify diff
  verify="$(step_at "$verify_step")"
  diff="$(step_at "Assert committed artifacts match freshly built")"
  [ -n "$diff" ]
  [ "$verify" -lt "$diff" ]
  echo "$runs" | grep -q 'git diff --exit-code'
}

@test "neither witness step is conditional" {
  local step guard
  for step in "$mark_step" "$verify_step"; do
    guard="$(step_field "$step" '.["if"] // "none"')"
    if [ "$guard" != "none" ]; then
      echo "FAIL: '$step' is guarded by: $guard" >&2
      return 1
    fi
  done
}

@test "the binary's hook list is exactly the hooks the workflow runs" {
  local declared invoked
  declared="$(sed -n '/pub(crate) const HOOKS/,/^];/p' "$witness" |
    grep -o '"script/[^"]*"' | tr -d '"' | sort)"
  invoked="$(echo "$runs" | grep -oE '\./script/[A-Za-z0-9_-]+\.(sol|sh)' |
    sed 's|^\./||' | sort -u)"
  [ -n "$declared" ]
  if [ "$declared" != "$invoked" ]; then
    echo "FAIL: codegen_witness.rs HOOKS and the hooks the workflow invokes disagree" >&2
    diff <(echo "$declared") <(echo "$invoked") >&2 || true
    return 1
  fi
}

@test "every rainix-copy-artifacts run step resolves rainix through the pinned sha" {
  local sha unpinned
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
  [ -n "$sha" ]
  [ "$sha" != "null" ]
  unpinned="$(echo "$runs" | grep 'github:rainlanguage/rainix' | grep -v 'env.RAINIX_SHA' || true)"
  if [ -n "$unpinned" ]; then
    echo "FAIL: unpinned rainix refs in rainix-copy-artifacts.yaml:" >&2
    echo "$unpinned" >&2
    return 1
  fi
}
