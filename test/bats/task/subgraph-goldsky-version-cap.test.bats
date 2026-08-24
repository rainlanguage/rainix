setup() {
  # shellcheck disable=SC1091
  source lib/subgraph.sh
}

@test "subgraph_goldsky_parse_versions extracts versions for a subgraph name" {
  list_text="$(
    cat <<'EOF'
┌──────────────┬──────────────────────────┐
│ Name         │ Version                  │
├──────────────┼──────────────────────────┤
│ raindex-base │ 0xabc-aaa1111            │
│ raindex-base │ 0xabc-bbb2222            │
│ raindex-eth  │ 0xdef-ccc3333            │
└──────────────┴──────────────────────────┘
raindex-base/0xabc-aaa1111
raindex-base/0xabc-bbb2222
EOF
  )"
  run bash -c "source lib/subgraph.sh; printf '%s\n' \"$list_text\" | subgraph_goldsky_parse_versions raindex-base"
  [ "$status" -eq 0 ]
  [[ "$output" == *"0xabc-aaa1111"* ]]
  [[ "$output" == *"0xabc-bbb2222"* ]]
  [[ "$output" != *"0xdef-ccc3333"* ]]
}

@test "subgraph_goldsky_versions_to_delete keeps preferred version and newest extras" {
  fixture_rows="$(
    cat <<'EOF'
old|100
keepme|50
mid|200
newest|300
EOF
  )"
  run bash -c "source lib/subgraph.sh; printf '%s\n' \"$fixture_rows\" | subgraph_goldsky_versions_to_delete 2 keepme"
  [ "$status" -eq 0 ]
  # keepme must be retained; only excess beyond max=2 are deleted.
  [[ "$output" != *"keepme"* ]]
  # Two deletes expected from the four inputs.
  count="$(printf '%s\n' "$output" | awk 'NF' | wc -l | tr -d ' ')"
  [ "$count" -eq 2 ]
}

@test "subgraph_goldsky_versions_to_delete with max 2 and no keep deletes oldest two of four" {
  fixture_rows="$(
    cat <<'EOF'
a|10
b|20
c|30
d|40
EOF
  )"
  run bash -c "source lib/subgraph.sh; printf '%s\n' \"$fixture_rows\" | subgraph_goldsky_versions_to_delete 2"
  [ "$status" -eq 0 ]
  [[ "$output" == *"a"* ]]
  [[ "$output" == *"b"* ]]
  [[ "$output" != *"c"* ]]
  [[ "$output" != *"d"* ]]
}

@test "subgraph_goldsky_migration_overlap_fail when newer is older than window" {
  now="$(date -u +%s)"
  newer=$((now - 90000)) # >24h
  older=$((now - 200000))
  run subgraph_goldsky_migration_overlap_fail "$newer" "$older" 24 0
  [ "$status" -eq 0 ]
}

@test "subgraph_goldsky_migration_overlap_fail is false for fresh newer version" {
  now="$(date -u +%s)"
  newer=$((now - 60))
  older=$((now - 200000))
  run subgraph_goldsky_migration_overlap_fail "$newer" "$older" 24 0
  [ "$status" -eq 1 ]
}

@test "subgraph_goldsky_migration_overlap_fail without timestamps fails closed unless keep" {
  run subgraph_goldsky_migration_overlap_fail 0 0 24 0
  [ "$status" -eq 0 ]
  run subgraph_goldsky_migration_overlap_fail 0 0 24 1
  [ "$status" -eq 1 ]
}
