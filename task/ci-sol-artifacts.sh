#!/usr/bin/env bash
set -euxo pipefail

# Shallow install is much faster for repos with several nested instances of
# foundry in the dependency tree.
forge install --shallow

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
    --rpc-url "${CI_DEPLOY_RPC_URL}" \
    --etherscan-api-key "${EXPLORER_VERIFICATION_KEY}" \
;