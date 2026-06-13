#!/usr/bin/env bash

# Rain convention is one contract per .sol file (named after the file). This
# library enforces it mechanically so the convention does not drift via inline
# helper/mock contracts in .t.sol test files or accumulated source helpers.
#
# Scope (per rainlanguage/rainix#214):
# - Counts top-level `contract` and `abstract contract` declarations.
# - `library` and `interface` are NOT counted — they are allowed alongside (or
#   instead of) a single contract.
# - File-scope `error`/`struct`/`enum`/`constant`/`function`/`type` are not
#   contracts and are always allowed.

# Count top-level `contract` / `abstract contract` declarations in a single
# .sol file, ignoring matches inside `//` line comments and `/* */` block
# comments. Top-level declarations sit at column zero in `forge fmt`-formatted
# source (which rainix enforces via `forge fmt --check`), so the count anchors
# the keyword to the start of the line.
# Usage: sol_count_contracts <sol_file>
sol_count_contracts() {
  local sol_file="$1"
  # 1. Strip /* ... */ block comments (including multi-line ones).
  # 2. Strip // ... line comments.
  # 3. Count lines whose first non-space token is an optional `abstract `
  #    qualifier followed by `contract ` and an identifier character. The
  #    leading-whitespace anchor avoids matching `contract` mid-expression
  #    (e.g. a `contract`-suffixed identifier) while tolerating the stray
  #    indentation a leading inline block comment leaves behind.
  # `grep -c` exits non-zero when it finds zero matches; that is a valid
  # result here (a file with no contracts), so it must not propagate as a
  # failure. `{ ...; } || true` keeps the count without the no-match status.
  {
    sed -E ':a;/\/\*/{/\*\//!{N;ba}};s,/\*[^*]*\*+([^/*][^*]*\*+)*/,,g' "$sol_file" \
      | sed -E 's,//.*$,,' \
      | grep -cE '^[[:space:]]*(abstract[[:space:]]+)?contract[[:space:]]+[A-Za-z_]'
  } || true
}

# Check a list of .sol files. Prints each offending file with its contract
# count and returns non-zero if any file declares more than one contract.
# Usage: sol_single_contract_check <sol_file> [<sol_file> ...]
sol_single_contract_check() {
  local sol_file
  local count
  local failed=0
  for sol_file in "$@"; do
    count="$(sol_count_contracts "$sol_file")"
    if [ "$count" -gt 1 ]; then
      echo "ERROR: $sol_file declares $count contracts; Rain convention is one contract per file." >&2
      failed=1
    fi
  done
  return "$failed"
}

# Enumerate all git-tracked .sol files in the current repository and run the
# single-contract check over them. Returns non-zero if any file declares more
# than one contract.
# Usage: sol_single_contract_check_tracked
sol_single_contract_check_tracked() {
  # -z / read -d handles paths with spaces or unusual characters.
  local -a files=()
  local sol_file
  while IFS= read -r -d '' sol_file; do
    files+=("$sol_file")
  done < <(git ls-files -z '*.sol')

  if [ "${#files[@]}" -eq 0 ]; then
    return 0
  fi

  sol_single_contract_check "${files[@]}"
}
