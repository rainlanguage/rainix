# Pass 4: Code Quality — Counter.sol

## Evidence of thorough reading

### Source: test/fixture/src/Counter.sol (15 lines)
- Contract: Counter (line 5)
- State variable: `number` (uint256, public) — line 6
- Function: `setNumber(uint256 newNumber)` — line 8
- Function: `increment()` — line 12

## Findings

### A01-1 [LOW] Pragma version mismatch between source and test

**File:** test/fixture/src/Counter.sol:3 vs test/fixture/test/Counter.t.sol:3

`Counter.sol` uses `pragma solidity ^0.8.25` while `Counter.t.sol` uses `pragma solidity ^0.8.13`. The source contract requires a newer compiler than the test file. While Foundry resolves this by using the highest required version, the inconsistency suggests the test pragma was not updated when the source pragma was bumped.
