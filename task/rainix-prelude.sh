#!/usr/bin/env bash
set -euxo pipefail

# It's assumed that even rust builds likely need build artifacts from the
# solidity contracts.
# We do NOT do a shallow clone here because nix flakes seem to not be compatible
# with shallow clones.
forge install
forge build