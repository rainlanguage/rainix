# Pass 1: Security — Counter.sol

## Evidence of thorough reading

**File:** test/fixture/src/Counter.sol (15 lines)

### Contract: Counter (line 5)
- State variable: `number` (uint256, public) — line 6
- Function: `setNumber(uint256 newNumber)` — line 8
- Function: `increment()` — line 12

### Types/errors/constants
- None

## Findings

No security findings. This is a test fixture contract. `setNumber` has no access control, which is intentional for a CI validation fixture. Arithmetic overflow in `increment()` is handled by Solidity 0.8+ built-in checks.
