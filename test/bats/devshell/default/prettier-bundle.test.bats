setup() {
  pcc="$BATS_TEST_DIRNAME/../../../../.pre-commit-config.yaml"
  prettier_entry="$(yq '.repos[0].hooks[] | select(.name == "prettier") | .entry' "$pcc")"
  no_consumer_prettier_entry="$(yq '.repos[0].hooks[] | select(.name == "no-consumer-prettier") | .entry' "$pcc")"
  fixtures="$BATS_TEST_DIRNAME/../../../fixture/prettier-bundle"
}

@test "rainix-bundled prettier formats Svelte files with TS enums" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/svelte-with-enum/Lock.svelte" "$tmpdir/"
  cd "$tmpdir"

  bash -c "$prettier_entry Lock.svelte"

  actual="$(cat Lock.svelte)"
  rm -rf "$tmpdir"

  # Enum body intact (the original #117 bug stripped it) AND the script was
  # reformatted to canonical double quotes (proves the svelte plugin ran;
  # without it, prettier --ignore-unknown skips .svelte and the source's
  # single quotes survive).
  echo "$actual" | grep -q 'enum ButtonStatus {'
  echo "$actual" | grep -q 'READY = "LOCK"'
}

@test "rainix-bundled prettier reformats to rainix canon: 2-space indent and double quotes" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/canon-style/sample.ts" "$tmpdir/"
  cd "$tmpdir"

  bash -c "$prettier_entry sample.ts"

  actual="$(cat sample.ts)"
  rm -rf "$tmpdir"

  echo "$actual" | grep -qE '^  console\.log\("hello"\);$'
}

@test "no-consumer-prettier blocks package.json that declares prettier" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/blocked-prettier-dep/package.json" "$tmpdir/"
  cd "$tmpdir"

  run bash -c "$no_consumer_prettier_entry"
  rm -rf "$tmpdir"

  [ "$status" -ne 0 ]
  echo "$output" | grep -q "must not declare prettier"
}

@test "no-consumer-prettier blocks package.json that declares prettier-plugin-*" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/blocked-plugin-dep/package.json" "$tmpdir/"
  cd "$tmpdir"

  run bash -c "$no_consumer_prettier_entry"
  rm -rf "$tmpdir"

  [ "$status" -ne 0 ]
  echo "$output" | grep -q "prettier-plugin-svelte"
}

@test "no-consumer-prettier blocks prettier in optionalDependencies" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/blocked-optional-dep/package.json" "$tmpdir/"
  cd "$tmpdir"

  run bash -c "$no_consumer_prettier_entry"
  rm -rf "$tmpdir"

  [ "$status" -ne 0 ]
  echo "$output" | grep -q "prettier-plugin-tailwindcss"
}

@test "no-consumer-prettier blocks top-level \"prettier\" key in package.json" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/blocked-prettier-key/package.json" "$tmpdir/"
  cd "$tmpdir"

  run bash -c "$no_consumer_prettier_entry"
  rm -rf "$tmpdir"

  [ "$status" -ne 0 ]
  echo "$output" | grep -q 'top-level "prettier" key'
}

@test "no-consumer-prettier blocks the presence of any consumer .prettierrc" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/blocked-prettierrc/.prettierrc.json" "$tmpdir/"
  cd "$tmpdir"

  run bash -c "$no_consumer_prettier_entry"
  rm -rf "$tmpdir"

  [ "$status" -ne 0 ]
  echo "$output" | grep -q ".prettierrc.json is present"
}

@test "no-consumer-prettier blocks .prettierrc.ts (TypeScript variant)" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/blocked-prettierrc-ts/.prettierrc.ts" "$tmpdir/"
  cd "$tmpdir"

  run bash -c "$no_consumer_prettier_entry"
  rm -rf "$tmpdir"

  [ "$status" -ne 0 ]
  echo "$output" | grep -q ".prettierrc.ts is present"
}

@test "no-consumer-prettier passes when package.json is clean and there is no .prettierrc" {
  tmpdir="$(mktemp -d)"
  cp "$fixtures/clean/package.json" "$tmpdir/"
  cd "$tmpdir"

  run bash -c "$no_consumer_prettier_entry"
  rm -rf "$tmpdir"

  [ "$status" -eq 0 ]
}

@test "no-consumer-prettier hook is configured with always_run = true" {
  always_run="$(yq '.repos[0].hooks[] | select(.name == "no-consumer-prettier") | .always_run' "$pcc")"
  [ "$always_run" = "true" ]
}
