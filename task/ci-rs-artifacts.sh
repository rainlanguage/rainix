#!/usr/bin/env bash
set -euxo pipefail

# It's assumed that the rust build likely need build artifacts from the solidity
# contracts.
# Shallow install is much faster for repos with several nested instances of
# foundry in the dependency tree.
forge install --shallow
forge build

cargo build --release