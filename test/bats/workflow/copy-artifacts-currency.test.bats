setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-copy-artifacts.yaml"
  assert_script="$(yq -r '.jobs["copy-artifacts"].steps[] | select(.name == "Assert committed artifacts match freshly built") | .run' "$workflow")"

  consumer="$(mktemp -d)"
  # A consumer checkout is the only git state the step reads, so the fixture
  # owns its whole git environment: no ambient config, no ambient excludes.
  export GIT_CONFIG_NOSYSTEM=1
  export HOME="$consumer"
  cd "$consumer" || return 1
  git init -q -b main .
  git config user.email rainix@example.com
  git config user.name rainix
  printf 'out/\ncache/\ndependencies/\n' >.gitignore
  mkdir -p src/generated
  printf 'library CodeGennable {}\n' >src/generated/CodeGennable.sol
  git add --all
  git commit -qm 'committed artifacts'
}

teardown() {
  cd / || return 0
  rm -rf "$consumer"
}

run_assert() {
  bash -c "$assert_script"
}

@test "a checkout whose regeneration changed nothing passes" {
  run run_assert

  [ "$status" -eq 0 ]
}

@test "a committed artifact whose content drifted fails" {
  printf 'library CodeGennable { uint256 constant X = 1; }\n' >src/generated/CodeGennable.sol

  run run_assert

  [ "$status" -eq 1 ]
  [[ "$output" == *"CodeGennable.sol"* ]]
  [[ "$output" == *"::error::"* ]]
}

@test "an artifact regenerated under a new name, leaving the old one committed, fails" {
  printf 'library CodeGennableRenamed {}\n' >src/generated/CodeGennableRenamed.sol

  run run_assert

  [ "$status" -eq 1 ]
  [[ "$output" == *"CodeGennableRenamed.sol"* ]]
  [[ "$output" == *"::error::"* ]]
}

@test "a committed artifact that regeneration no longer emits fails" {
  rm src/generated/CodeGennable.sol

  run run_assert

  [ "$status" -eq 1 ]
  [[ "$output" == *"CodeGennable.sol"* ]]
  [[ "$output" == *"::error::"* ]]
}

@test "build output the consumer gitignores passes" {
  mkdir -p out cache dependencies
  printf '{}\n' >out/CodeGennable.json
  printf '{}\n' >cache/solidity-files-cache.json
  printf 'library Dep {}\n' >dependencies/Dep.sol

  run run_assert

  [ "$status" -eq 0 ]
}
