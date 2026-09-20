# Nothing in this repo executes rainix-copy-artifacts.yaml — it is
# `workflow_call` only, so its only runners are the consumer repos, which means
# a step silently dropped from it goes unnoticed here and ungated everywhere.
#
# What the codegen witness DOES is covered in test/bats/action/codegen-witness
# and in rainix-static/src/codegen_witness.rs. What is asserted here is the part
# that only the workflow can get wrong: that both phases are still invoked, and
# that they still bracket every codegen hook — a witness taken on the wrong side
# of a step measures nothing and reports green, which is the exact failure mode
# it was added to remove (rainlanguage/rain.factory.deploy#35).

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-copy-artifacts.yaml"
  witness="$repo_root/rainix-static/src/codegen_witness.rs"
  names="$(yq -r '.jobs["copy-artifacts"].steps[] | .name // "«unnamed»"' "$workflow")"
  runs="$(yq -r '.jobs["copy-artifacts"].steps[] | select(.run) | .run' "$workflow")"
  mark_step="Mark tracked files before codegen"
  verify_step="Assert every declared generated file was written"
}

# 1-based position of a step in the job, by name.
step_at() {
  echo "$names" | grep -nxF "$1" | cut -d: -f1
}

# The `uses`/`with.phase` of a step, by name.
step_field() {
  yq -r ".jobs[\"copy-artifacts\"].steps[] | select(.name == \"$1\") | $2" "$workflow"
}

@test "the job invokes both codegen-witness phases" {
  [ "$(step_field "$mark_step" .uses)" = "rainlanguage/rainix/.github/actions/codegen-witness@main" ]
  [ "$(step_field "$mark_step" .with.phase)" = "mark" ]
  [ "$(step_field "$verify_step" .uses)" = "rainlanguage/rainix/.github/actions/codegen-witness@main" ]
  [ "$(step_field "$verify_step" .with.phase)" = "verify" ]
}

# THE ordering invariant. A hook that runs before `mark` or after `verify` is
# outside the window, so whatever it writes looks unwritten — or, worse, a hook
# added later beside `forge fmt` would make a dead emitter look alive.
@test "both witness phases bracket every codegen hook step" {
  local mark verify
  mark="$(step_at "$mark_step")"
  verify="$(step_at "$verify_step")"
  [ -n "$mark" ]
  [ -n "$verify" ]
  [ "$mark" -lt "$verify" ]

  # Every step that INVOKES a consumer codegen hook (`./script/<hook>`), by
  # position. The invocation form is what matters: the final assertion step
  # names the same paths in its error text without running any of them.
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

# `forge fmt` rewrites sources of its own, so a witness taken after it cannot
# tell a file a generator wrote from one the formatter touched.
@test "the witness closes before forge fmt runs" {
  local verify fmt
  verify="$(step_at "$verify_step")"
  fmt="$(echo "$names" | grep -n 'Format' | cut -d: -f1)"
  [ -n "$fmt" ]
  [ "$verify" -lt "$fmt" ]
}

# The diff is the other half of the check and must not be traded away for this
# one: content currency and emitter liveness are different claims.
@test "the committed-artifacts diff is still asserted after the witness" {
  local verify diff
  verify="$(step_at "$verify_step")"
  diff="$(step_at "Assert committed artifacts match freshly built")"
  [ -n "$diff" ]
  [ "$verify" -lt "$diff" ]
  echo "$runs" | grep -q 'git diff --exit-code'
}

# A `hashFiles` guard would let a repo delete its codegen hooks and take the
# check that was watching them along with it. Whether there is anything to
# witness is the binary's decision, in one tested place.
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

# The binary decides "does this repo run codegen?" from its own HOOKS list. If
# the workflow gains or renames a hook and that list does not follow, a repo
# with codegen is told it has none and is never asked for a manifest.
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
