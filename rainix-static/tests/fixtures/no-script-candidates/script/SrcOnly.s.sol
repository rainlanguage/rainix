// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {Beacon} from "../src/Beacon.sol";
import {LibProdDeployV4} from "../src/LibProdDeployV4.sol";

contract SrcOnly {
    function run() external returns (Beacon, bytes memory, string memory, address) {
        Beacon beacon = new Beacon();
        return (beacon, type(Beacon).creationCode, type(Beacon).name, LibProdDeployV4.TIMELOCK);
    }
}
