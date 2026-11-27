// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {LibProdDeployV4} from "./LibProdDeployV4.sol";
import {LibRollout} from "./LibRollout.sol";

contract Beacon {
    address public constant OWNER = LibProdDeployV4.TIMELOCK_CANDIDATE;
    address public immutable implementation;

    constructor() {
        implementation = LibRollout.plan();
    }

    function owner() external pure returns (address) {
        return OWNER;
    }
}
