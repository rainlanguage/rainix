# Pass 5: Correctness / Intent Verification — flake.nix

## Evidence of thorough reading

See Pass 1 flake.md for complete function/binding inventory.

## Findings

### A04-1 [MEDIUM] rainix-rs-prelude is a no-op but CI runs it before every task

**File:** flake.nix:253-259, .github/workflows/test.yml:51

`rainix-rs-prelude` is defined with an empty body (`set -euxo pipefail` and
nothing else). The CI workflow runs `nix run ../..#rainix-rs-prelude` before
every matrix task (line 51). This is harmless but costs CI time building and
running a no-op derivation. More importantly, if a downstream consumer expects
`rainix-rs-prelude` to prepare the Rust environment (by analogy with
`rainix-sol-prelude` which does `forge install && forge build`), they'll get no
preparation.

### A04-2 [LOW] CLAUDE.md documents rainix-rs-prelude as "currently no-op, placeholder for env prep" but flake has no TODO/comment

**File:** flake.nix:253-259

CLAUDE.md (line 32) notes this is a "placeholder for env prep" but the flake.nix
source has no corresponding comment or TODO indicating this is intentionally
incomplete. Future maintainers reading only the flake won't know whether this is
done or pending.
