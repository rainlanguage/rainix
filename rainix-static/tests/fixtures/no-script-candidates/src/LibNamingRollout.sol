// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {LibProdDeployV4} from "./LibProdDeployV4.sol";

library LibNamingRollout {
    address internal constant FACET = LibProdDeployV4.FACET_CANDIDATE;
    address internal constant FACET_IMPLEMENTATION = FACET;
    address internal constant TIMELOCK = LibProdDeployV4.TIMELOCK;
}
