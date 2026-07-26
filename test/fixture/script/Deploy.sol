// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity ^0.8.25;

import {Script} from "forge-std-1.16.1/src/Script.sol";
import {Counter} from "../src/Counter.sol";

contract Deploy is Script {
    function setUp() public {}

    /// Reads the deployer key from `DEPLOYMENT_KEY`, the same convention as
    /// the consumer `Deploy.sol` scripts `rainix-sol-artifacts` runs, so the
    /// fixture exercises the task exactly as consumers do (broadcast included
    /// — a bare `vm.broadcast()` would hit foundry's default-sender refusal).
    function run() public {
        uint256 deployerPrivateKey = vm.envUint("DEPLOYMENT_KEY");
        vm.startBroadcast(deployerPrivateKey);
        new Counter();
        vm.stopBroadcast();
    }
}
