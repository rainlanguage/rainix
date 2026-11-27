// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity =0.8.25;

import {Vault} from "../src/Vault.sol";

contract GeneratedContractType {
    function run() external returns (Vault, bytes memory, string memory) {
        Vault vault = new Vault();
        return (vault, type(Vault).creationCode, type(Vault).name);
    }
}
