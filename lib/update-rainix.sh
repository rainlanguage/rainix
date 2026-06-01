#!/usr/bin/env bash

# Update the current rainix-consuming repo to the latest rainix and re-lock
# Soldeer dependencies. Run from the root of a consumer repo. Makes local
# changes only (no commit/push) — review the diff and commit yourself.
#
# Usage:
#   nix run github:rainlanguage/rainix#... is not required; just:
#   bash <(curl -fsSL https://raw.githubusercontent.com/rainlanguage/rainix/main/lib/update-rainix.sh)
#   # or, from a checkout of rainix:
#   /path/to/rainix/lib/update-rainix.sh
#
# What it does:
#   1. Points the `rainix` flake input at github:rainlanguage/rainix and runs
#      `nix flake lock --update-input rainix` (-> latest default branch).
#   2. Ensures the flake `outputs` argument set ends with `...` so the lint
#      hooks can drop an unused `self` without breaking evaluation (Nix always
#      passes `self`), then runs deadnix/statix/nixfmt over flake.nix.
#   3. If a foundry.toml is present, re-locks Soldeer (`forge soldeer install`)
#      and runs a sanity `forge build`.
#
# Soldeer dependency *version* bumps are intentionally left to the developer:
# edit the versions in foundry.toml's [dependencies], run `forge soldeer
# update`, then fix the version-suffixed import paths in src/test/script.
# Bumping blindly can break builds when a transitive dependency pins an older
# version (with recursive_deps = false).

set -euo pipefail

if [ ! -f flake.nix ]; then
  echo "update-rainix: no flake.nix in $(pwd); run from a consumer repo root" >&2
  exit 1
fi

# 1. canonical rainix url (rainprotocol -> rainlanguage, strip any pinned ref)
#    then bump the lock to the latest default branch.
sed -i -E 's#github:rain(protocol|language)/rainix(/[^"]*)?#github:rainlanguage/rainix#g' flake.nix
nix flake lock --update-input rainix

# 2. ensure the outputs argument set ends with `...` before linting.
python3 - flake.nix <<'PY'
import re, sys

path = sys.argv[1]
text = open(path).read()


def ensure_ellipsis(match):
    pre, open_brace, params, close_brace, colon = match.groups()
    if "..." not in params:
        params = params.rstrip() + ", ..."
    return pre + open_brace + params + close_brace + colon


# the first `{ ... }:` after `outputs =` is the argument set (no nested braces)
text = re.sub(
    r"(outputs\s*=\s*)(\{)([^{}]*?)(\})(\s*:)",
    ensure_ellipsis,
    text,
    count=1,
    flags=re.S,
)
open(path, "w").write(text)
PY

# 3. clean flake.nix for the latest rainix lint hooks, then (if Solidity) re-lock
#    Soldeer and sanity-build. Tools come from the consumer repo's dev shell.
nix develop --command bash -euo pipefail -c '
  deadnix --edit flake.nix >/dev/null 2>&1 || true
  statix fix  flake.nix    >/dev/null 2>&1 || true
  nixfmt      flake.nix    >/dev/null 2>&1 || true
  if [ -f foundry.toml ] && command -v forge >/dev/null 2>&1; then
    forge soldeer install
    forge build
  fi
'

echo "update-rainix: done. Review the changes with \`git diff\` and commit."
