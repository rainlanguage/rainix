// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {LibProdDeployV4} from "./LibProdDeployV4.sol";

library LibRollout {
    function facet() internal pure returns (address) {
        return LibProdDeployV4.FACET_CANDIDATE;
    }

    function facets() internal pure returns (address[] memory) {
        address[] memory list = new address[](1);
        list[0] = facet();
        return list;
    }

    function firstFacet() internal pure returns (address) {
        return facets()[0];
    }

    function plan() internal pure returns (address) {
        return firstFacet();
    }

    function timelock() internal pure returns (address) {
        return LibProdDeployV4.TIMELOCK;
    }
}
