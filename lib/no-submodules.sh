#!/usr/bin/env bash

# Git submodules are banned org-wide: dependencies come from soldeer / npm /
# nix flakes, never vendored submodule pointers (they break shallow clones,
# soldeer consumers, and reproducibility). Two detection legs:
#   1. a ROOT .gitmodules file (git only reads the repo root's; a vendored
#      dependencies/*/.gitmodules is inert content and is NOT flagged);
#   2. any committed gitlink entry (mode 160000) — catches a submodule whose
#      .gitmodules was deleted but whose pointer is still tracked.

# Check the repo at $1 (default current dir); print offenders and return 1,
# or return 0 when clean.
check_no_submodules() {
  local dir="${1:-.}"
  local fail=0
  if [ -f "$dir/.gitmodules" ]; then
    echo "Root .gitmodules found — submodules are banned (use soldeer/npm/nix):"
    echo "  $dir/.gitmodules"
    fail=1
  fi
  local gitlinks
  gitlinks=$(git -C "$dir" ls-files -s 2>/dev/null | awk '$1 == "160000" { print "  " $4 }')
  if [ -n "$gitlinks" ]; then
    echo "Committed gitlink entries (mode 160000) found — submodule pointers are banned:"
    echo "$gitlinks"
    fail=1
  fi
  if [ "$fail" -eq 0 ]; then
    echo "no-submodules: clean"
  fi
  return "$fail"
}
