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

# Convert a Goldsky "Created:" timestamp to unix epoch seconds.
# Accepts US locale (7/16/2026, 9:51:30 PM) and ISO-ish strings.
# Prints 0 and returns 1 on failure.
# Usage: subgraph_goldsky_created_to_epoch <created_text>
subgraph_goldsky_created_to_epoch() {
  local raw="$1"
  local s mon day year hour min sec ampm iso

  s="$(
    printf '%s' "$raw" |
      sed -E 's/^[Cc]reated([ _][Aa]t)?:[[:space:]]*//; s/,//g; s/^[[:space:]]+//; s/[[:space:]]+$//'
  )"
  [[ -z "$s" ]] && {
    echo 0
    return 1
  }

  # US locale: M/D/YYYY H:MM:SS AM/PM (comma already stripped).
  if [[ "$s" =~ ^([0-9]{1,2})/([0-9]{1,2})/([0-9]{4})[[:space:]]+([0-9]{1,2}):([0-9]{2}):([0-9]{2})[[:space:]]+([AaPp][Mm])$ ]]; then
    mon=$(printf '%02d' "$((10#${BASH_REMATCH[1]}))")
    day=$(printf '%02d' "$((10#${BASH_REMATCH[2]}))")
    year="${BASH_REMATCH[3]}"
    hour=$((10#${BASH_REMATCH[4]}))
    min="${BASH_REMATCH[5]}"
    sec="${BASH_REMATCH[6]}"
    ampm="$(printf '%s' "${BASH_REMATCH[7]}" | tr 'apm' 'APM')"
    if [[ "$ampm" == "PM" && "$hour" -ne 12 ]]; then
      hour=$((hour + 12))
    elif [[ "$ampm" == "AM" && "$hour" -eq 12 ]]; then
      hour=0
    fi
    iso="$(printf '%s-%s-%s %02d:%s:%s' "$year" "$mon" "$day" "$hour" "$min" "$sec")"
    if date -u -d "$iso" +%s 2>/dev/null; then
      return 0
    fi
    if date -u -j -f "%Y-%m-%d %H:%M:%S" "$iso" +%s 2>/dev/null; then
      return 0
    fi
    echo 0
    return 1
  fi

  # ISO / RFC3339-ish: 2026-07-16T21:51:30Z or with space.
  if [[ "$s" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2} ]]; then
    if date -u -d "$s" +%s 2>/dev/null; then
      return 0
    fi
    # BSD: strip trailing Z / fractional seconds.
    s="${s%%.*}"
    s="${s%Z}"
    s="${s/T/ }"
    if date -u -j -f "%Y-%m-%d %H:%M:%S" "$s" +%s 2>/dev/null; then
      return 0
    fi
  fi

  echo 0
  return 1
}

# Parse version names for a subgraph from goldsky list text on stdin.
# Skips GraphQL URL lines so trailing /gn is not absorbed into phantom versions.
# Usage: goldsky_list_text | subgraph_goldsky_parse_versions <subgraph_name>
subgraph_goldsky_parse_versions() {
  local subgraph_name="$1"
  sed 's/\x1b\[[0-9;]*m//g' |
    grep -viE 'https?://' |
    grep -oE "${subgraph_name}/[A-Za-z0-9._-]+" |
    sed "s|^${subgraph_name}/||" |
    awk 'NF' |
    sort -u
}

# Parse version|epoch rows from a name-only goldsky subgraph list on stdin.
# Pairs each non-URL "name/version" sighting with the following Created: line.
# Usage: goldsky_list_text | subgraph_goldsky_parse_version_rows <subgraph_name>
subgraph_goldsky_parse_version_rows() {
  local subgraph_name="$1"
  local line cleaned version created epoch current=""
  local -A seen=()

  while IFS= read -r line || [[ -n "$line" ]]; do
    cleaned="$(printf '%s' "$line" | sed 's/\x1b\[[0-9;]*m//g')"

    if printf '%s' "$cleaned" | grep -qiE 'https?://'; then
      continue
    fi

    version="$(
      printf '%s' "$cleaned" |
        grep -oE "${subgraph_name}/[A-Za-z0-9._-]+" |
        head -n1 |
        sed "s|^${subgraph_name}/||"
    )"
    if [[ -n "$version" ]]; then
      current="$version"
      continue
    fi

    if [[ -n "$current" ]] && printf '%s' "$cleaned" | grep -qiE 'Created([ _]at)?[[:space:]]*:'; then
      created="$(
        printf '%s' "$cleaned" |
          sed -E 's/^.*[Cc]reated([ _][Aa]t)?[[:space:]]*:[[:space:]]*//'
      )"
      epoch="$(subgraph_goldsky_created_to_epoch "$created")"
      # First Created for a version wins (listing order).
      if [[ -z "${seen[$current]:-}" ]]; then
        seen[$current]=1
        printf '%s|%s\n' "$current" "$epoch"
      fi
      current=""
    fi
  done
}

# Parse distinct subgraph base names from `goldsky subgraph list --summary` stdin.
# Usage: goldsky_summary_text | subgraph_goldsky_parse_summary_names
subgraph_goldsky_parse_summary_names() {
  sed 's/\x1b\[[0-9;]*m//g' |
    grep -viE 'https?://' |
    grep -oE '[A-Za-z0-9._-]+/[A-Za-z0-9._-]+' |
    cut -d/ -f1 |
    awk 'NF' |
    sort -u
}

# Decide which versions to delete given keep/max.
# Input lines: version|epoch (epoch must be >0 for age ordering).
# Output: versions to delete, one per line (excess beyond max after keep preference).
# Usage: printf 'v|e\n...' | subgraph_goldsky_versions_to_delete <max> [keep_version]
subgraph_goldsky_versions_to_delete() {
  local max_versions="$1"
  local keep_version="${2:-}"
  local input
  input="$(cat)"

  if [[ -z "$input" ]]; then
    return 0
  fi

  # Fail closed: never fall back to string-sort deletes when ages are missing.
  if printf '%s\n' "$input" | awk -F'|' '$2+0 <= 0 { found=1 } END { exit found ? 0 : 1 }'; then
    echo "Refusing to choose delete targets: one or more Created timestamps are missing/unparseable." >&2
    return 1
  fi

  printf '%s\n' "$input" | awk -F'|' -v keep="$keep_version" '
    {
      version=$1
      epoch=$2+0
      priority=(keep != "" && version == keep) ? 2 : 0
      printf "%d %020d %s\n", priority, epoch, version
    }
  ' | sort -k1,1nr -k2,2nr -k3,3r |
    awk -v max="$max_versions" 'NR > max { print $3 }'
}

# Return 0 if a 2-version migration overlap should fail / be reclaimed.
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

# Older version from version|epoch rows (requires epochs > 0).
# Usage: printf 'v|e\n...' | subgraph_goldsky_older_version
subgraph_goldsky_older_version() {
  local input
  input="$(cat)"
  if printf '%s\n' "$input" | awk -F'|' '$2+0 <= 0 { found=1 } END { exit found ? 0 : 1 }'; then
    echo "Cannot pick older version without Created timestamps." >&2
    return 1
  fi
  printf '%s\n' "$input" |
    awk -F'|' '{ printf "%020d %s\n", $2+0, $1 }' |
    sort -k1,1n |
    head -n1 |
    awk '{ print $2 }'
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

  # Lists version|epoch from name-only listing. Fails closed on CLI errors.
  _subgraph_goldsky_list_version_rows() {
    local raw rc=0
    set +e
    raw="$(_subgraph_goldsky_cmd subgraph list "$subgraph_name" --filter deployments 2>&1)"
    rc=$?
    set -e
    printf '%s\n' "$raw" >&2

    if [[ $rc -ne 0 ]]; then
      echo "Goldsky subgraph list failed for ${subgraph_name} (exit ${rc})." >&2
      return 1
    fi
    if printf '%s\n' "$raw" | grep -qiE 'listing failed|not found|unauthorized|forbidden|invalid token|rate limit'; then
      # "not found" alone can mean zero deployments for a brand-new name; only
      # treat as hard failure when the CLI also signals an error-shaped message
      # that is not a clean empty listing. Prefer exit-code above; this catches
      # soft-failure text with exit 0.
      if printf '%s\n' "$raw" | grep -qiE 'listing failed|unauthorized|forbidden|invalid token|rate limit'; then
        echo "Goldsky subgraph list returned an error for ${subgraph_name}." >&2
        return 1
      fi
    fi

    printf '%s\n' "$raw" | subgraph_goldsky_parse_version_rows "$subgraph_name"
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
    echo "  - ${subgraph_name}/${row%%|*} (created_epoch=${row##*|})"
  done

  if [[ ${#rows[@]} -gt 1 ]]; then
    if printf '%s\n' "${rows[@]}" | awk -F'|' '$2+0 <= 0 { found=1 } END { exit found ? 0 : 1 }'; then
      echo "::error title=Goldsky Created timestamps missing::Could not parse Created dates for ${subgraph_name}; refusing unsafe age-based deletes." >&2
      return 1
    fi
  fi

  if [[ "$check_only" -eq 0 ]]; then
    local to_delete
    if ! to_delete="$(printf '%s\n' "${rows[@]}" | subgraph_goldsky_versions_to_delete "$max_versions" "$keep_version")"; then
      return 1
    fi
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
        echo "  - ${subgraph_name}/${row%%|*} (created_epoch=${row##*|})"
      done
    fi
  fi

  if [[ ${#rows[@]} -gt $max_versions ]]; then
    echo "::error title=Goldsky version cap exceeded::${subgraph_name} has ${#rows[@]} live versions; max allowed is ${max_versions}."
    return 1
  fi

  if [[ ${#rows[@]} -eq 2 ]]; then
    local sorted newer_epoch older_epoch older_version has_keep=0
    sorted="$(printf '%s\n' "${rows[@]}" | awk -F'|' '{ printf "%020d %s|%s\n", $2+0, $1, $2 }' | sort -k1,1nr | awk '{ print $2 }')"
    newer_epoch="$(printf '%s\n' "$sorted" | sed -n '1p' | cut -d'|' -f2)"
    older_epoch="$(printf '%s\n' "$sorted" | sed -n '2p' | cut -d'|' -f2)"
    [[ -n "$keep_version" ]] && has_keep=1
    if subgraph_goldsky_migration_overlap_fail "$newer_epoch" "$older_epoch" "$migration_hours" "$has_keep"; then
      if [[ "$check_only" -eq 1 ]]; then
        echo "::error title=Goldsky migration overlap >${migration_hours}h::${subgraph_name} still has 2 live versions past the ${migration_hours}h migration window."
        return 1
      fi

      # Reclaim: delete the older version so the 2nd slot stops billing.
      if ! older_version="$(printf '%s\n' "${rows[@]}" | subgraph_goldsky_older_version)"; then
        return 1
      fi
      if [[ -n "$keep_version" && "$older_version" == "$keep_version" ]]; then
        echo "::error title=Goldsky migration reclaim blocked::Older version is the keep target (${keep_version}); manual intervention required." >&2
        return 1
      fi
      echo "Migration window exceeded; reclaiming older version ${subgraph_name}/${older_version}"
      _subgraph_goldsky_cmd subgraph delete "${subgraph_name}/${older_version}" --force

      rows=()
      while IFS= read -r row; do
        [[ -n "$row" ]] && rows+=("$row")
      done < <(_subgraph_goldsky_list_version_rows)

      if [[ ${#rows[@]} -gt 1 ]]; then
        echo "::error title=Goldsky migration reclaim incomplete::${subgraph_name} still has ${#rows[@]} live versions after reclaim."
        return 1
      fi
    else
      echo "Two versions present for ${subgraph_name}; migration window OK."
    fi
  fi

  echo "Version cap OK for ${subgraph_name}."
}

# Fail if account-level Goldsky subgraph names are outside the allowlist.
# Usage: subgraph_goldsky_audit_orphans <allowlist_file>
# allowlist_file: one subgraph base name per line (e.g. raindex-base).
subgraph_goldsky_audit_orphans() {
  local allowlist_file="${1:?allowlist file required}"

  if [[ -z "${GOLDSKY_TOKEN:-}" ]]; then
    echo "GOLDSKY_TOKEN is required for Goldsky orphan audit." >&2
    return 1
  fi

  local goldsky_bin="${GOLDSKY_BIN:-goldsky}"
  local raw rc=0
  set +e
  raw="$("$goldsky_bin" --token "$GOLDSKY_TOKEN" --color=false subgraph list --summary --filter deployments 2>&1)"
  rc=$?
  set -e
  printf '%s\n' "$raw" >&2

  if [[ $rc -ne 0 ]]; then
    echo "Goldsky account summary list failed (exit ${rc})." >&2
    return 1
  fi
  if printf '%s\n' "$raw" | grep -qiE 'listing failed|unauthorized|forbidden|invalid token|rate limit'; then
    echo "Goldsky account summary list returned an error." >&2
    return 1
  fi

  local names orphans
  names="$(printf '%s\n' "$raw" | subgraph_goldsky_parse_summary_names)"
  if [[ -z "$names" ]]; then
    echo "No subgraph names found in account summary."
    return 0
  fi

  orphans="$(
    printf '%s\n' "$names" | while IFS= read -r name; do
      [[ -z "$name" ]] && continue
      if ! grep -Fxq "$name" "$allowlist_file"; then
        printf '%s\n' "$name"
      fi
    done
  )"

  if [[ -n "$orphans" ]]; then
    echo "::error title=Goldsky orphan subgraphs::Names outside networks.json allowlist (manual cleanup needed):"
    printf '%s\n' "$orphans" | sed 's/^/  - /'
    return 1
  fi

  echo "No orphan Goldsky subgraph names outside allowlist."
}
