// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {LibProdDeployV4} from "../src/LibProdDeployV4.sol";

// Multi-byte text, so a char offset and a byte offset land on different lines:
// ———————————————————————————————————————————————————————————————————————————
// ———————————————————————————————————————————————————————————————————————————
// ———————————————————————————————————————————————————————————————————————————
// ———————————————————————————————————————————————————————————————————————————
// ———————————————————————————————————————————————————————————————————————————
contract Direct {
    function run() external pure returns (address, address) {
        address frozen = LibProdDeployV4.TIMELOCK;
        return (frozen, LibProdDeployV4.TIMELOCK_CANDIDATE);
    }
}
