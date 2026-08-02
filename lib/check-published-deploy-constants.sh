#!/usr/bin/env bash
# SPDX-License-Identifier: LicenseRef-DCL-1.0
# SPDX-FileCopyrightText: Copyright (c) 2020 Rain Open Source Software Ltd
#
# Checks that every version published to the soldeer registry for a given
# package has a full suite of pinned deploy constants in the specified Solidity
# file. For each published version and each constant prefix, asserts that both
# a DEPLOYED_ADDRESS_<ver> and DEPLOYED_CODEHASH_<ver> constant exist (where
# <ver> is the version string with dots replaced by underscores).
#
# Usage:
#   check-published-deploy-constants <soldeer-package> <deploy-lib-path> <prefix> [<prefix> ...]
#
# Arguments:
#   soldeer-package  soldeer package name to query (e.g. "raindex")
#   deploy-lib-path  path to the Solidity file holding the constants
#   prefix...        one or more constant name prefixes
#
# Output for a well-formed invocation (always exits 0, so registry state never
# reds a consumer pipeline):
#   OK               every published version has its full constant suite
#   MISSING: <names> one or more expected constants are absent
#   SKIP: <reason>   nothing was verified; the reason distinguishes an
#                    unreachable registry from a reachable registry whose
#                    response yielded no parseable versions
#
# A malformed invocation (fewer than 3 arguments) is a caller bug, not a
# registry state: usage goes to stderr and the exit status is 1, so a miswired
# CI step fails loud instead of silently skipping forever.

set -euo pipefail

if [ "$#" -lt 3 ]; then
  printf 'Usage: check-published-deploy-constants <soldeer-package> <deploy-lib-path> <prefix> [<prefix>...]\n' >&2
  exit 1
fi

package="$1"
lib="$2"
shift 2

if ! response=$(curl -fsS "https://api.soldeer.xyz/api/v1/revision?project_name=${package}" 2>/dev/null); then
  printf 'SKIP: could not fetch published soldeer versions'
  exit 0
fi

# grep exits non-zero on zero matches; that is the legitimate
# "registry reachable but no versions parsed" case (e.g. a package with no
# published releases yet), kept distinct from the connectivity SKIP above so
# parser/API drift is never masked as an unreachable registry.
versions=$(
  printf '%s' "$response" \
    | grep -oE '"version":"[0-9][0-9.]*"' | cut -d'"' -f4 | sort -u
) || true

if [ -z "$versions" ]; then
  printf 'SKIP: no versions parsed from registry response'
  exit 0
fi

missing=""
for v in $versions; do
  suffix=$(printf '%s' "$v" | tr . _)
  for p in "$@"; do
    for kind in ADDRESS CODEHASH; do
      name="${p}_${kind}_${suffix}"
      grep -qE "constant ${name} =" "$lib" || missing="${missing} ${name}"
    done
  done
done

if [ -n "$missing" ]; then
  printf 'MISSING:%s' "$missing"
else
  printf 'OK'
fi
