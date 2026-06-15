#!/usr/bin/env bash

# Bump a rainix-consuming repo to the latest rainix and re-lock Soldeer.
# Run from the consumer repo root. Makes local changes only (no commit/push) —
# review the diff and commit yourself.
#
# Soldeer dependency *version* bumps are intentionally left to the developer
# (edit foundry.toml's [dependencies], run `forge soldeer update`, fix the
# version-suffixed imports) — bumping blindly can break builds when a transitive
# dependency pins an older version.

set -euo pipefail

if [ ! -f flake.nix ]; then
  echo "update-rainix: no flake.nix in $(pwd); run from a consumer repo root" >&2
  exit 1
fi

# 1. bump the `rainix` flake input to the latest default branch.
nix flake lock --update-input rainix

# 2. for Solidity repos, re-lock Soldeer and sanity-build (tools from the
#    consumer's own dev shell).
if [ -f foundry.toml ]; then
  nix develop --command bash -euo pipefail -c '
    forge soldeer install
    forge build
  '
fi

echo "update-rainix: done. Review the changes with \`git diff\` and commit."
