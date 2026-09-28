# Nothing in this repo executes rainix-copy-artifacts.yaml or
# rainix-tag-release.yaml — both are `workflow_call` only, so their only
# runners are the consumer repos, and a step dropped from either goes unnoticed
# here and ungated everywhere.
#
# One file for both, because the property spans them: a generated config is
# STAGED rather than written (foundry refuses to write the project root's own
# foundry.toml), so a regeneration that is not followed by the install changes
# no tracked file — the currency check and the publish guard then both pass on
# a tree nothing regenerated. The two paths regenerate with the same command in
# order to prove the same thing, so they have to install with the same one too.
#
# What the install DOES is covered by the Rust unit tests in
# rainix-static/src/staged_config.rs, and which trees reach it by
# test/bats/action/install-staged-config.test.bats.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  copy_artifacts="$repo_root/.github/workflows/rainix-copy-artifacts.yaml"
  tag_release="$repo_root/.github/workflows/rainix-tag-release.yaml"
  action='rainlanguage/rainix/.github/actions/install-staged-config@main'
}

# One line per step, so a step's position is a line number and a multi-line
# `run:` block cannot be mistaken for several steps.
steps_of() {
  yq -o=json -I=0 ".jobs[\"$2\"].steps[]" "$1"
}

# Line number of the first step matching $3, or empty.
index_of() {
  steps_of "$1" "$2" | grep -n -- "$3" | head -1 | cut -d: -f1
}

@test "rainix-copy-artifacts installs what the codegen staged" {
  run index_of "$copy_artifacts" copy-artifacts "$action"
  [ -n "$output" ]
}

@test "rainix-tag-release installs what the codegen staged" {
  run index_of "$tag_release" release "$action"
  [ -n "$output" ]
}

# Before the install the config is still the committed one; after the currency
# check it is too late to matter.
@test "the copy-artifacts install follows the codegen and precedes the currency check" {
  local codegen install assert
  codegen="$(index_of "$copy_artifacts" copy-artifacts 'forge script ./script/Build.sol')"
  install="$(index_of "$copy_artifacts" copy-artifacts "$action")"
  assert="$(index_of "$copy_artifacts" copy-artifacts 'git diff --exit-code')"
  [ -n "$codegen" ] && [ -n "$install" ] && [ -n "$assert" ]
  [ "$codegen" -lt "$install" ]
  [ "$install" -lt "$assert" ]
}

# The guard requires a clean tree. An uninstalled staged file leaves the tree
# clean by never touching it, which is the guard passing on a config that no
# regeneration produced.
@test "the tag-release install follows the codegen and precedes the publish guard" {
  local codegen install guard
  codegen="$(index_of "$tag_release" release 'forge script ./script/Build.sol')"
  install="$(index_of "$tag_release" release "$action")"
  guard="$(index_of "$tag_release" release 'release-guard')"
  [ -n "$codegen" ] && [ -n "$install" ] && [ -n "$guard" ]
  [ "$codegen" -lt "$install" ]
  [ "$install" -lt "$guard" ]
}

# The failure message is the only instruction a maintainer gets for reproducing
# the regeneration locally, and the install is now part of it.
@test "the stale-artifacts message names the install" {
  run yq -r '.jobs["copy-artifacts"].steps[] | select(.run) | .run' "$copy_artifacts"
  [[ "$output" == *"Committed artifacts are stale"* ]]
  [[ "$output" == *"rainix-static install-staged-config"* ]]
}
