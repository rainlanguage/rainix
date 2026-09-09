#!/usr/bin/env bash

# Verifies that every reusable workflow in .github/workflows/rainix-*.yaml
# pins its RAINIX_SHA to the same commit SHA, and that no bare unpinned
# github:rainlanguage/rainix# references exist. A missed SHA during a bump
# silently leaves one reusable on a stale toolchain; this script catches the
# drift before it reaches CI.
#
# Usage (from the repo root):
#   source lib/check-rainix-flake-pin.sh
#   check_rainix_flake_pin_consistent   # exits non-zero on any mismatch

# Extract the RAINIX_SHA value from a single workflow file.
# Outputs the trimmed sha string if found; empty string otherwise.
# Usage: _extract_sha <yaml_file>
_extract_sha() {
    local file="$1"
    # `|| true` keeps a no-match grep from aborting callers running under
    # `set -euo pipefail` (the mkTask default); output is empty in that case.
    grep -E '^\s+RAINIX_SHA:' "$file" 2>/dev/null | head -1 | sed 's/.*RAINIX_SHA:\s*//' | tr -d '[:space:]' || true
}

# Count bare unpinned github:rainlanguage/rainix# refs (the 429-prone form)
# across all rainix-*.yaml workflow files.
# Usage: _count_unpinned_refs <workflows_dir>
_count_unpinned_refs() {
    local dir="$1"
    # Zero matches is the healthy case: `wc -l` still prints 0, and `|| true`
    # keeps grep's no-match status from tripping `set -euo pipefail` callers.
    grep -rE 'github:rainlanguage/rainix#' "$dir"/rainix-*.yaml 2>/dev/null | wc -l | tr -d '[:space:]' || true
}

# Assert that all rainix-*.yaml reusable workflow files define the same
# RAINIX_SHA value, that it is non-empty, and that no bare unpinned
# github:rainlanguage/rainix# refs are present.
#
# Arguments:
#   $1  Path to the .github/workflows directory (default: .github/workflows)
#
# Exits non-zero and prints a diagnostic on any violation.
check_rainix_flake_pin_consistent() {
    local workflows_dir="${1:-.github/workflows}"
    local first_sha=""
    local first_file=""
    local mismatches=0
    local matched=0

    for f in "${workflows_dir}"/rainix-*.yaml; do
        # An unmatched glob stays literal, so existence-guard each entry.
        # (compgen is not available in the minimal non-interactive bash that
        # mkTask scripts run under, so the glob itself is the existence check.)
        if [ ! -e "$f" ]; then
            continue
        fi
        matched=1
        # Aggregator workflows (only `uses:` other local reusables) and
        # workflows with no nix flake refs have nothing to pin; RAINIX_SHA is
        # only required where the rainix flake is actually referenced.
        if ! grep -q 'github:rainlanguage/rainix' "$f"; then
            continue
        fi
        local sha
        sha="$(_extract_sha "$f")"
        if [ -z "$sha" ]; then
            echo "check-rainix-flake-pin: RAINIX_SHA not found in ${f}" >&2
            mismatches=$((mismatches + 1))
            continue
        fi
        if [ -z "$first_sha" ]; then
            first_sha="$sha"
            first_file="$f"
        elif [ "$sha" != "$first_sha" ]; then
            echo "check-rainix-flake-pin: SHA mismatch: ${f} has '${sha}' but ${first_file} has '${first_sha}'" >&2
            mismatches=$((mismatches + 1))
        fi
    done

    if [ "$matched" -eq 0 ]; then
        echo "check-rainix-flake-pin: no rainix-*.yaml files found under ${workflows_dir}" >&2
        return 1
    fi

    local unpinned
    unpinned="$(_count_unpinned_refs "$workflows_dir")"
    if [ "$unpinned" -gt 0 ]; then
        echo "check-rainix-flake-pin: ${unpinned} bare unpinned 'github:rainlanguage/rainix#' ref(s) found — replace with pinned SHA form" >&2
        mismatches=$((mismatches + 1))
    fi

    if [ "$mismatches" -gt 0 ]; then
        return 1
    fi
    return 0
}
