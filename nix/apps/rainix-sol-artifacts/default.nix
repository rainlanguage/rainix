{ writeShellApplication
, foundry-bin
}:
writeShellApplication {
  name = "rainix-sol-artifacts";
  meta.description = "Rainix Solidity build artifacts";
  runtimeInputs = [
    foundry-bin
  ];
  text = ''
    # Upload all function selectors to the registry.
    forge selectors up --all

    # Deploy all contracts to testnet.
    # Assumes the existence of a `Deploy.sol` script in the `script` directory.
    # Echos the deploy pubkey to stdout to make it easy to add gas to the account.
    echo 'deploy pubkey:';
    cast wallet address "''${DEPLOYMENT_KEY}";
    # Need to set --rpc-url explicitly due to an upstream bug.
    # https://github.com/foundry-rs/foundry/issues/6731

    attempts=;
    do_deploy() {
      forge script script/Deploy.sol:Deploy \
        -vvvvv \
        --slow \
        ''${DEPLOY_LEGACY:+--legacy} \
        ''${DEPLOY_BROADCAST:+--broadcast} \
        ''${DEPLOY_SKIP_SIMULATION:+--skip-simulation} \
        --rpc-url "''${ETH_RPC_URL}" \
        ''${DEPLOY_VERIFY:+--verify} \
        ''${DEPLOY_VERIFIER:+--verifier "''${DEPLOY_VERIFIER}"} \
        ''${DEPLOY_VERIFIER_URL:+--verifier-url "''${DEPLOY_VERIFIER_URL}"} \
        ''${ETHERSCAN_API_KEY:+--etherscan-api-key "''${ETHERSCAN_API_KEY}"} \
        ''${attempts:+--resume} \
        ;
    }

    until do_deploy; do
      attempts=$((''${attempts:-0} + 1));
      echo "Deploy failed, retrying in 5 seconds... (attempt ''${attempts})";
      sleep 5;
      if [[ ''${attempts} -gt 10 ]]; then
        echo "Deploy failed after 10 attempts, aborting.";
        exit 1;
      fi
    done
  '';
}
