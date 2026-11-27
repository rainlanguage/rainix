// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {LibProdDeployV4 as LibProdDeployCurrent} from "../src/LibProdDeployV4.sol";
import {GOVERNANCE_CANDIDATE as GOVERNANCE} from "../src/LibProdDeployV4.sol";

contract Alias {
    function timelock() external pure returns (address) {
        return LibProdDeployCurrent.TIMELOCK_CANDIDATE;
    }

    function governance() external pure returns (address) {
        return GOVERNANCE;
    }

    function frozen() external pure returns (address) {
        return LibProdDeployCurrent.TIMELOCK;
    }
}
