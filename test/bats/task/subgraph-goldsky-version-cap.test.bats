setup() {
  # shellcheck disable=SC1091
  source lib/subgraph.sh
}

@test "subgraph_goldsky_parse_versions skips GraphQL URL /gn phantoms" {
  list_text="$(
    cat <<'EOF'
* raindex-base/0xb05D73E6abc-105c526
  https://api.goldsky.com/api/public/project_cmexample/subgraphs/raindex-base/0xb05D73E6abc-105c526/gn
* raindex-base/0xe522cB4adef-8e9477b
  https://api.goldsky.com/api/public/project_cmexample/subgraphs/raindex-base/0xe522cB4adef-8e9477b/gn
* raindex-eth/0xdef-ccc3333
EOF
  )"
  run bash -c "source lib/subgraph.sh; printf '%s\n' \"$list_text\" | subgraph_goldsky_parse_versions raindex-base"
  [ "$status" -eq 0 ]
  [[ "$output" == *"0xb05D73E6abc-105c526"* ]]
  [[ "$output" == *"0xe522cB4adef-8e9477b"* ]]
  [[ "$output" != *"0xb05D73E6abc-105c526gn"* ]]
  [[ "$output" != *"0xe522cB4adef-8e9477bgn"* ]]
  [[ "$output" != *"0xdef-ccc3333"* ]]
  count="$(printf '%s\n' "$output" | awk 'NF' | wc -l | tr -d ' ')"
  [ "$count" -eq 2 ]
}

@test "subgraph_goldsky_parse_version_rows pairs Created US dates without URL phantoms" {
  list_text="$(
    cat <<'EOF'
* raindex-base/0xb05D73E6abc-105c526
  Status: LIVE
  Created: 7/16/2026, 9:51:30 PM
  https://api.goldsky.com/api/public/project_cmexample/subgraphs/raindex-base/0xb05D73E6abc-105c526/gn

* raindex-base/0xe522cB4adef-8e9477b
  Status: LIVE
  Created: 8/20/2026, 1:02:03 AM
  https://api.goldsky.com/api/public/project_cmexample/subgraphs/raindex-base/0xe522cB4adef-8e9477b/gn
EOF
  )"
  run bash -c "source lib/subgraph.sh; printf '%s\n' \"$list_text\" | subgraph_goldsky_parse_version_rows raindex-base"
  [ "$status" -eq 0 ]
  [[ "$output" == *"0xb05D73E6abc-105c526|"* ]]
  [[ "$output" == *"0xe522cB4adef-8e9477b|"* ]]
  [[ "$output" != *"gn|"* ]]
  while IFS='|' read -r _ver epoch; do
    [[ -z "$_ver" ]] && continue
    [ "$epoch" -gt 0 ]
  done <<<"$output"
  count="$(printf '%s\n' "$output" | awk 'NF' | wc -l | tr -d ' ')"
  [ "$count" -eq 2 ]
}

@test "subgraph_goldsky_created_to_epoch parses US locale Created strings" {
  run subgraph_goldsky_created_to_epoch "7/16/2026, 9:51:30 PM"
  [ "$status" -eq 0 ]
  [ "$output" -gt 0 ]
}

@test "subgraph_goldsky_created_to_epoch parses ISO Created strings" {
  run subgraph_goldsky_created_to_epoch "2026-07-16T21:51:30Z"
  [ "$status" -eq 0 ]
  [ "$output" -gt 0 ]
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
  [[ "$output" != *"keepme"* ]]
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

@test "subgraph_goldsky_versions_to_delete fails closed when epochs are missing" {
  fixture_rows="$(
    cat <<'EOF'
a|0
b|20
c|30
EOF
  )"
  run bash -c "source lib/subgraph.sh; printf '%s\n' \"$fixture_rows\" | subgraph_goldsky_versions_to_delete 2"
  [ "$status" -ne 0 ]
}

@test "subgraph_goldsky_older_version picks lowest epoch" {
  fixture_rows="$(
    cat <<'EOF'
newer|300
older|100
mid|200
EOF
  )"
  run bash -c "source lib/subgraph.sh; printf '%s\n' \"$fixture_rows\" | subgraph_goldsky_older_version"
  [ "$status" -eq 0 ]
  [ "$output" = "older" ]
}

@test "subgraph_goldsky_parse_summary_names skips URL paths" {
  summary="$(
    cat <<'EOF'
* raindex-base/0xaaa
  https://api.goldsky.com/api/public/project_x/subgraphs/raindex-base/0xaaa/gn
* ob4-base/1.0.0
  https://api.goldsky.com/api/public/project_x/subgraphs/ob4-base/1.0.0/gn
* metadata-base/2
EOF
  )"
  run bash -c "source lib/subgraph.sh; printf '%s\n' \"$summary\" | subgraph_goldsky_parse_summary_names"
  [ "$status" -eq 0 ]
  [[ "$output" == *"raindex-base"* ]]
  [[ "$output" == *"ob4-base"* ]]
  [[ "$output" == *"metadata-base"* ]]
  [[ "$output" != *"api.goldsky.com"* ]]
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
