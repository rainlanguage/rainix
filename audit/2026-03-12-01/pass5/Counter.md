# Pass 5: Correctness / Intent Verification — Counter.sol

## Evidence of thorough reading

### Source: test/fixture/src/Counter.sol (15 lines)

- Contract: Counter (line 5)
- State variable: `number` (uint256, public) — line 6
- Function: `setNumber(uint256 newNumber)` — line 8: sets number to newNumber
- Function: `increment()` — line 12: increments number by 1

## Findings

No findings. `setNumber` sets and `increment` increments — behavior matches
names.
