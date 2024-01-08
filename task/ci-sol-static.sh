#!/usr/bin/env bash
set -euxo pipefail

# Shallow install is much faster for repos with several nested instances of
# foundry in the dependency tree.
forge install --shallow

slither .
forge fmt --check