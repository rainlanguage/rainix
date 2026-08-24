#!/usr/bin/env bash

# Extract the contract address for a network from networks.json.
# Usage: subgraph_network_address <networks_json_path> <network>
subgraph_network_address() {
  local networks_json="$1"
  local network="$2"
  jq -r --arg net "$network" '.[$net] | to_entries[0].value.address' "$networks_json"
}

# Derive a deterministic deploy version from contract address and git commit.
# Usage: subgraph_deploy_version <address> <commit>
subgraph_deploy_version() {
  local address="$1"
  local commit="$2"
  echo "${address}-${commit}"
}

# List all networks defined in networks.json.
# Usage: subgraph_networks <networks_json_path>
subgraph_networks() {
  local networks_json="$1"
  jq -r 'keys[]' "$networks_json"
}

# Parse version names for a subgraph from goldsky list text on stdin.
# Usage: goldsky_list_text | subgraph_goldsky_parse_versions <subgraph_name>
subgraph_goldsky_parse_versions() {
  local subgraph_name="$1"
  sed 's/\x1b\[[0-9;]*m//g' |
    grep -oE "${subgraph_name}/[^[:space:]│|]+" |
    sed "s|^${subgraph_name}/||" |
    sed 's/[^A-Za-z0-9._-]//g' |
    awk 'NF' |
    sort -u
}

# Decide which versions to delete given keep/max.
# Input lines: version|epoch (epoch may be 0).
# Output: versions to delete, one per line (excess beyond max after keep preference).
# Usage: printf 'v|e\n...' | subgraph_goldsky_versions_to_delete <max> [keep_version]
subgraph_goldsky_versions_to_delete() {
  local max_versions="$1"
  local keep_version="${2:-}"
  awk -F'|' -v keep="$keep_version" '
    {
      version=$1
      epoch=$2+0
      priority=(keep != "" && version == keep) ? 2 : 0
      printf "%d %020d %s\n", priority, epoch, version
    }
  ' | sort -k1,1nr -k2,2nr -k3,3r |
    awk -v max="$max_versions" 'NR > max { print $3 }'
}

# Return 0 if a 2-version migration overlap should fail.
# Args: newer_epoch older_epoch migration_hours [has_keep]
# has_keep=1 means a fresh deploy keep was provided (unknown timestamps OK).
subgraph_goldsky_migration_overlap_fail() {
  local newer_epoch="$1"
  local older_epoch="$2"
  local migration_hours="$3"
  local has_keep="${4:-0}"
  local now_epoch limit_seconds
  now_epoch="$(date -u +%s)"
  limit_seconds=$((migration_hours * 3600))

  if [[ "$newer_epoch" -gt 0 ]]; then
    if [[ $((now_epoch - newer_epoch)) -gt $limit_seconds ]]; then
      return 0
    fi
    return 1
  fi

  if [[ "$older_epoch" -gt 0 ]]; then
    if [[ $((now_epoch - older_epoch)) -gt $limit_seconds ]]; then
      return 0
    fi
    return 1
  fi

  # No timestamps: fail closed on scheduled checks; allow during fresh deploy keep.
  if [[ "$has_keep" -eq 1 ]]; then
    return 1
  fi
  return 0
}

# Enforce always-on Goldsky version budget for one subgraph name (RAI-1962).
#
# Env:
#   GOLDSKY_TOKEN            required for goldsky CLI
#   GOLDSKY_MAX_VERSIONS     default 2
#   GOLDSKY_MIGRATION_HOURS  default 24
#
# Usage:
#   subgraph_goldsky_enforce_version_cap <subgraph_name> [--keep <version>] [--check-only]
subgraph_goldsky_enforce_version_cap() {
  local subgraph_name="${1:?subgraph name required}"
  shift || true

  local keep_version=""
  local check_only=0
  local max_versions="${GOLDSKY_MAX_VERSIONS:-2}"
  local migration_hours="${GOLDSKY_MIGRATION_HOURS:-24}"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --keep)
        keep_version="${2:?}"
        shift 2
        ;;
      --check-only)
        check_only=1
        shift
        ;;
      --max)
        max_versions="${2:?}"
        shift 2
        ;;
      --migration-hours)
        migration_hours="${2:?}"
        shift 2
        ;;
      *)
        echo "Unknown option for subgraph_goldsky_enforce_version_cap: $1" >&2
        return 2
        ;;
    esac
  done

  if [[ -z "${GOLDSKY_TOKEN:-}" ]]; then
    echo "GOLDSKY_TOKEN is required for Goldsky version-cap enforcement." >&2
    return 1
  fi

  local goldsky_bin="${GOLDSKY_BIN:-goldsky}"

  _subgraph_goldsky_cmd() {
    "$goldsky_bin" --token "$GOLDSKY_TOKEN" --color=false "$@"
  }

  _subgraph_goldsky_list_version_rows() {
    local raw versions version detail created epoch
    raw="$(_subgraph_goldsky_cmd subgraph list "$subgraph_name" --filter deployments 2>&1 || true)"
    printf '%s\n' "$raw" >&2
    versions="$(printf '%s\n' "$raw" | subgraph_goldsky_parse_versions "$subgraph_name")"
    while IFS= read -r version; do
      [[ -z "$version" ]] && continue
      epoch=0
      detail="$(_subgraph_goldsky_cmd subgraph list "${subgraph_name}/${version}" --filter deployments 2>/dev/null || true)"
      created="$(
        printf '%s\n' "$detail" |
          sed 's/\x1b\[[0-9;]*m//g' |
          grep -oiE '(created([ _]at)?|created):[[:space:]]*[0-9T:Z.+-]+' |
          head -n1 |
          grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}[^[:space:]]*' || true
      )"
      if [[ -n "$created" ]]; then
        epoch="$(date -u -d "$created" +%s 2>/dev/null || echo 0)"
      fi
      printf '%s|%s\n' "$version" "$epoch"
    done <<<"$versions"
  }

  echo "==> Enforcing Goldsky version cap for ${subgraph_name} (max=${max_versions}, keep=${keep_version:-none}, check_only=${check_only})"

  local -a rows=()
  local row
  while IFS= read -r row; do
    [[ -n "$row" ]] && rows+=("$row")
  done < <(_subgraph_goldsky_list_version_rows)

  if [[ ${#rows[@]} -eq 0 ]]; then
    echo "No deployments found for ${subgraph_name}."
    return 0
  fi

  echo "Live versions (${#rows[@]}):"
  for row in "${rows[@]}"; do
    echo "  - ${subgraph_name}/${row%%|*}"
  done

  if [[ "$check_only" -eq 0 ]]; then
    local to_delete
    to_delete="$(printf '%s\n' "${rows[@]}" | subgraph_goldsky_versions_to_delete "$max_versions" "$keep_version")"
    if [[ -n "$to_delete" ]]; then
      while IFS= read -r version; do
        [[ -z "$version" ]] && continue
        echo "Deleting ${subgraph_name}/${version}"
        _subgraph_goldsky_cmd subgraph delete "${subgraph_name}/${version}" --force
      done <<<"$to_delete"

      rows=()
      while IFS= read -r row; do
        [[ -n "$row" ]] && rows+=("$row")
      done < <(_subgraph_goldsky_list_version_rows)

      echo "After cleanup (${#rows[@]}):"
      for row in "${rows[@]}"; do
        echo "  - ${subgraph_name}/${row%%|*}"
      done
    fi
  fi

  if [[ ${#rows[@]} -gt $max_versions ]]; then
    echo "::error title=Goldsky version cap exceeded::${subgraph_name} has ${#rows[@]} live versions; max allowed is ${max_versions}."
    return 1
  fi

  if [[ ${#rows[@]} -eq 2 ]]; then
    local sorted newer_epoch older_epoch has_keep=0
    sorted="$(printf '%s\n' "${rows[@]}" | awk -F'|' '{ printf "%020d %s|%s\n", $2+0, $1, $2 }' | sort -k1,1nr | awk '{ print $2 }')"
    newer_epoch="$(printf '%s\n' "$sorted" | sed -n '1p' | cut -d'|' -f2)"
    older_epoch="$(printf '%s\n' "$sorted" | sed -n '2p' | cut -d'|' -f2)"
    [[ -n "$keep_version" ]] && has_keep=1
    if subgraph_goldsky_migration_overlap_fail "$newer_epoch" "$older_epoch" "$migration_hours" "$has_keep"; then
      echo "::error title=Goldsky migration overlap >${migration_hours}h::${subgraph_name} still has 2 live versions past the ${migration_hours}h migration window."
      return 1
    fi
    echo "Two versions present for ${subgraph_name}; migration window OK."
  fi

  echo "Version cap OK for ${subgraph_name}."
}
