# Pass 2: Test Coverage — flake.nix

## Evidence of thorough reading

### Source: flake.nix (414 lines)
- See Pass 1 flake.md for complete function/binding inventory

### Test files:
- `.github/workflows/test.yml` — runs rainix-sol-*, rainix-rs-* tasks against test/fixture/
- `.github/workflows/check-shell.yml` — verifies dev shell tool availability
- `test/bats/devshell/tauri/shellhook.test.bats` — validates tauri shell environment

## Findings

### A02-1 [LOW] No CI coverage for subgraph tasks

**File:** .github/workflows/

`subgraph-build`, `subgraph-test`, and `subgraph-deploy` are defined in the flake but have no CI workflow exercising them. While subgraph-test requires Docker and subgraph-deploy requires tokens, subgraph-build could be validated in CI (it just runs forge build + npm ci + graph codegen/build).

### A02-2 [LOW] Default dev shell not tested in check-shell.yml

**File:** .github/workflows/check-shell.yml

`check-shell.yml` tests specific tool availability (`cargo release`, `flamegraph`, `graph`, `goldsky`) and the tauri shell hook, but does not test basic dev shell entry (`nix develop --command true`) or verify that the default shell's `shellHook` (which runs `npm ci`) succeeds. If a nixpkgs update breaks the shell derivation, there's no direct CI signal.
