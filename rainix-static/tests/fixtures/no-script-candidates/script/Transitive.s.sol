// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {LibRollout} from "../src/LibRollout.sol";

contract Transitive {
    function run() external pure returns (address, address) {
        return (LibRollout.timelock(), LibRollout.plan());
    }
}
