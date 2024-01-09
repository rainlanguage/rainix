#!/usr/bin/env bash
set -euxo pipefail

# It's assumed that even rust builds likely need build artifacts from the
# solidity contracts.
# Shallow install is much faster for repos with several nested instances of
# foundry in the dependency tree.
forge install --shallow
forge build