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
# Output (always exits 0):
#   OK               every published version has its full constant suite
#   MISSING: <names> one or more expected constants are absent
#   SKIP: <reason>   the registry could not be reached (nothing verified)

set -euo pipefail

if [ "$#" -lt 3 ]; then
  printf 'Usage: check-published-deploy-constants <soldeer-package> <deploy-lib-path> <prefix> [<prefix>...]\n' >&2
  exit 1
fi

package="$1"
lib="$2"
shift 2

versions=$(
  curl -fsS "https://api.soldeer.xyz/api/v1/revision?project_name=${package}" 2>/dev/null \
    | grep -oE '"version":"[0-9][0-9.]*"' | cut -d'"' -f4 | sort -u
) || true

if [ -z "$versions" ]; then
  printf 'SKIP: could not fetch published soldeer versions'
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
