// SPDX-License-Identifier: LicenseRef-DCL-1.0
// SPDX-FileCopyrightText: Copyright (c) 2020 thedavidmeister
pragma solidity ^0.8.25;

contract Counter {
    uint256 public number;

    event SetNumber(uint256 indexed newNumber);

    function setNumber(uint256 newNumber) public {
        number = newNumber;
        emit SetNumber(newNumber);
    }

    function increment() public {
        number++;
    }
}
