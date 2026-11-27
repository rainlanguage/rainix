// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {DEPLOYED_ADDRESS} from "./generated/candidate/Vault.sol";

contract Vault {
    function deployedAddress() external pure returns (address) {
        return DEPLOYED_ADDRESS;
    }
}
