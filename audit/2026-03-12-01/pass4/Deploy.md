# Pass 4: Code Quality — Deploy.sol

## Evidence of thorough reading

### Source: test/fixture/script/Deploy.sol (13 lines)
- Contract: Deploy is Script (line 7)
- Function: `setUp()` — line 8 (empty)
- Function: `run()` — line 10

## Findings

### A03-1 [LOW] Pragma version mismatch with Counter.sol

**File:** test/fixture/script/Deploy.sol:3

`Deploy.sol` uses `pragma solidity ^0.8.13` while `Counter.sol` uses `^0.8.25`. Same inconsistency as noted in Counter.t.sol.
