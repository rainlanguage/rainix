#!/usr/bin/env bash

# Rain pragma convention (rainix#250):
# - Files whose top-level declarations are only library, interface, or
#   abstract contract (plus any file-scope errors/structs/constants/types)
#   MUST use `pragma solidity ^X.Y.Z` so downstream soldeer consumers can
#   compile published source against any compatible compiler.
# - Files declaring any concrete (non-abstract) contract MUST use
#   `pragma solidity =X.Y.Z` to pin the compiler version exactly.

# Strip /* */ block comments and // line comments from a .sol file.
# Shared with the comment-stripping in sol-single-contract.sh.
_sol_pc_strip_comments() {
  local file="$1"
  sed -E ':a;/\/\*/{/\*\//!{N;ba}};s,/\*[^*]*\*+([^/*][^*]*\*+)*/,,g' "$file" \
    | sed -E 's,//.*$,,'
}

# Count top-level concrete (non-abstract) contract declarations in a .sol file
# after stripping comments. In forge-fmt formatted source, top-level
# declarations sit at column zero, so `contract ` at the start of a line
# (without an `abstract ` prefix on the same line) identifies concrete
# contracts.
_sol_count_concrete_contracts() {
  local file="$1"
  { _sol_pc_strip_comments "$file" \
      | grep -cE '^contract[[:space:]]+[A-Za-z_]'; } || true
}

# Determine the pragma operator the Rain convention requires for a .sol file.
# Prints "=" if any concrete contract is declared; "^" for lib/abstract/
# interface files or files with only file-scope declarations.
# Returns 1 (and prints nothing) for files without a `pragma solidity` line
# so callers can skip non-Solidity files that end up in the file list.
sol_expected_pragma_operator() {
  local file="$1"
  if ! grep -qE '^pragma solidity ' "$file" 2>/dev/null; then
    return 1
  fi
  local n
  n="$(_sol_count_concrete_contracts "$file")"
  if [ "$n" -gt 0 ]; then
    printf '='
  else
    printf '^'
  fi
}

# Extract the actual pragma operator from a .sol file.
# Prints "^", "=", or the raw operator text if an unsupported form is used
# (e.g. ">=", ">", no operator for bare version numbers).
# Returns 1 (and prints nothing) if no pragma line is found.
sol_actual_pragma_operator() {
  local file="$1"
  local line
  line="$(grep -m1 -E '^pragma solidity ' "$file" 2>/dev/null || true)"
  if [ -z "$line" ]; then
    return 1
  fi
  # Extract everything between "pragma solidity " and the first digit.
  # e.g. "pragma solidity ^0.8.25;" → "^"
  #      "pragma solidity =0.8.25;" → "="
  #      "pragma solidity >=0.8.0 <0.9.0;" → ">="
  #      "pragma solidity 0.8.25;" → "" (bare version, no operator)
  printf '%s' "$line" \
    | sed -E 's/^pragma solidity ([^0-9 	]*)[0-9].*/\1/' \
    | tr -d '[:space:]'
}

# Check each given .sol file against the Rain pragma convention.
# Returns non-zero and prints a diagnostic for every violating file.
sol_pragma_convention_check() {
  local file expected actual failed=0
  for file in "$@"; do
    expected="$(sol_expected_pragma_operator "$file")" || continue
    if ! actual="$(sol_actual_pragma_operator "$file")"; then
      printf 'ERROR: %s: no pragma solidity line.\n' "$file" >&2
      failed=1
      continue
    fi
    case "$actual" in
      '^' | '=') ;;
      *)
        printf 'ERROR: %s: pragma operator "%s" is not ^ or = — Rain convention requires ^ for lib/abstract files and = for concrete contract files.\n' \
          "$file" "$actual" >&2
        failed=1
        continue
        ;;
    esac
    if [ "$actual" != "$expected" ]; then
      if [ "$expected" = '=' ]; then
        reason='concrete contract — must pin with ='
      else
        reason='lib/abstract/interface — must float with ^'
      fi
      printf 'ERROR: %s: pragma operator is "%s" but Rain convention requires "%s" (%s).\n' \
        "$file" "$actual" "$expected" "$reason" >&2
      failed=1
    fi
  done
  return "$failed"
}

# Enumerate all git-tracked .sol files, excluding auto-generated
# (src/generated/) and soldeer-vendored (dependencies/) directories, and run
# the pragma convention check over them.
sol_pragma_convention_check_tracked() {
  local -a files=()
  local file
  while IFS= read -r -d '' file; do
    case "$file" in
      src/generated/* | */src/generated/*) continue ;;
      dependencies/* | */dependencies/*) continue ;;
    esac
    files+=("$file")
  done < <(git ls-files -z '*.sol')

  if [ "${#files[@]}" -eq 0 ]; then
    return 0
  fi

  sol_pragma_convention_check "${files[@]}"
}
