#!/usr/bin/env bash
set -euxo pipefail

# Upload all function selectors to the registry.
forge selectors up --all

# Deploy all contracts to testnet.
# Assumes the existence of a `Deploy.sol` script in the `script` directory.
# Echos the deploy pubkey to stdout to make it easy to add gas to the account.
echo 'deploy pubkey:'
cast wallet address "${DEPLOYMENT_KEY}";
forge script script/Deploy.sol:Deploy \
    --legacy \
    --verify \
    --broadcast \
;