setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  action="$repo_root/.github/actions/install-staged-config/action.yml"
  action_script="$(yq -r '.runs.steps[0].run' "$action")"
  work="$(mktemp -d)"
}

teardown() {
  rm -rf "$work"
}

mk_marked_consumer() {
  printf '[profile.default]\n# rain-deploy:generated:rpc_endpoints:begin\nold = "x"\n# rain-deploy:generated:rpc_endpoints:end\n' >"$work/foundry.toml"
  printf 'OLD_RPC_URL=\n' >"$work/.env.example"
}

install() {
  run rainix-static install-staged-config --root "$work"
}

@test "the action runs the subcommand from its own checkout" {
  # shellcheck disable=SC2016
  run env GITHUB_ACTION_PATH="$repo_root/.github/actions/install-staged-config" ACTION_SCRIPT="$action_script" bash -c '
    nix() { printf "nix"; printf " <%s>" "$@"; printf "\n"; }
    export -f nix
    bash -c "$ACTION_SCRIPT"
  '

  [ "$status" -eq 0 ]
  [[ "$output" == *"#rainix-static> <--> <install-staged-config>" ]]
  [[ "$output" == "nix <run> <path:$(cd "$repo_root" && pwd)#rainix-static>"* ]]
}

@test "every staged file replaces the root file of the same name" {
  mk_marked_consumer
  mkdir "$work/.staged-config"
  printf 'NEW FOUNDRY\n' >"$work/.staged-config/foundry.toml"
  printf 'NEW_RPC_URL=\n' >"$work/.staged-config/.env.example"

  install

  [ "$status" -eq 0 ]
  [ "$(cat "$work/foundry.toml")" = "NEW FOUNDRY" ]
  [ "$(cat "$work/.env.example")" = "NEW_RPC_URL=" ]
  [[ "$output" == *"installed "*".env.example"* ]]
  [[ "$output" == *"installed "*"foundry.toml"* ]]
}

@test "the staging dir is removed once installed" {
  mk_marked_consumer
  mkdir "$work/.staged-config"
  printf "# rain-deploy:generated:env:begin\nNEW\n" >"$work/.staged-config/foundry.toml"

  install
  [ "$status" -eq 0 ]
  [ ! -e "$work/.staged-config" ]

  install
  [ "$status" -ne 0 ]
  [[ "$output" == *"does not exist"* ]]
  [ "$(tail -1 "$work/foundry.toml")" = "NEW" ]
}

@test "a missing staging dir fails when foundry.toml has generated blocks" {
  mk_marked_consumer
  cp "$work/foundry.toml" "$work/before.toml"

  install

  [ "$status" -ne 0 ]
  [[ "$output" == *"::error::install-staged-config: .staged-config/ does not exist"* ]]
  cmp "$work/foundry.toml" "$work/before.toml"
}

@test "an empty staging dir fails and installs nothing" {
  mk_marked_consumer
  cp "$work/foundry.toml" "$work/before.toml"
  mkdir -p "$work/.staged-config/nested"
  printf 'NOT STAGED\n' >"$work/.staged-config/nested/foundry.toml"

  install

  [ "$status" -ne 0 ]
  [[ "$output" == *"::error::install-staged-config: .staged-config/ holds no files"* ]]
  cmp "$work/foundry.toml" "$work/before.toml"
  [ -d "$work/.staged-config" ]
}

@test "an empty staging dir fails even without generated blocks" {
  printf '[profile.default]\n' >"$work/foundry.toml"
  mkdir "$work/.staged-config"

  install

  [ "$status" -ne 0 ]
  [[ "$output" == *"holds no files"* ]]
}

@test "a repo that stages nothing and has no generated blocks is a no-op" {
  printf '[profile.default]\n# mentions rain-deploy:generated: mid-line only\n' >"$work/foundry.toml"
  cp "$work/foundry.toml" "$work/before.toml"

  install

  [ "$status" -eq 0 ]
  [[ "$output" == *"nothing staged"* ]]
  cmp "$work/foundry.toml" "$work/before.toml"
}

@test "a repo with no foundry.toml and nothing staged is a no-op" {
  install

  [ "$status" -eq 0 ]
  [[ "$output" == *"nothing staged"* ]]
}

@test "a marker after other text on its line still requires staging" {
  printf '[profile.default]\nx = 1 # rain-deploy:generated:rpc_endpoints:begin\n' >"$work/foundry.toml"

  install

  [ "$status" -ne 0 ]
  [[ "$output" == *"does not exist"* ]]
}
