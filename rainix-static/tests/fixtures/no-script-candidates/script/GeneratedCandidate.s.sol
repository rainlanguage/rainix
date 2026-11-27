// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {DEPLOYED_ADDRESS} from "../src/generated/candidate/Vault.sol";
import {DEPLOYED_ADDRESS as FROZEN_ADDRESS} from "../src/generated/0_1_0/Vault.sol";

contract GeneratedCandidate {
    function run() external pure returns (address, address) {
        address frozen = FROZEN_ADDRESS;
        return (frozen, DEPLOYED_ADDRESS);
    }
}
