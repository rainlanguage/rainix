# Pass 2: Test Coverage — Counter.sol

## Evidence of thorough reading

### Source: test/fixture/src/Counter.sol (15 lines)

- Contract: Counter (line 5)
- State variable: `number` (uint256, public) — line 6
- Function: `setNumber(uint256 newNumber)` — line 8
- Function: `increment()` — line 12

### Test: test/fixture/test/Counter.t.sol (25 lines)

- Contract: CounterTest is Test (line 8)
- Function: `setUp()` — line 11: creates Counter, sets number to 0
- Function: `test_Increment()` — line 16: increments once, asserts == 1
- Function: `testFuzz_SetNumber(uint256 x)` — line 21: fuzz sets number, asserts

## Findings

### A01-1 [LOW] No test for increment overflow behavior

**File:** test/fixture/test/Counter.t.sol

`test_Increment` only tests incrementing from 0 to 1. There is no test verifying
that `increment()` reverts when `number` is `type(uint256).max`. While Solidity
0.8+ guarantees overflow reverts, an explicit test documents this expected
behavior and would catch regressions if the contract were modified to use
`unchecked`.

### A01-2 [LOW] No test for consecutive increments

**File:** test/fixture/test/Counter.t.sol

`test_Increment` calls `increment()` once from 0. There is no test verifying
multiple consecutive increments (e.g., increment from 0 to N) to exercise
accumulation behavior.
