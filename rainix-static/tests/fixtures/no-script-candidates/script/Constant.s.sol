// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {LibNamingRollout} from "../src/LibNamingRollout.sol";

contract Constant {
    function run() external pure returns (address, address) {
        return (LibNamingRollout.TIMELOCK, LibNamingRollout.FACET_IMPLEMENTATION);
    }
}
