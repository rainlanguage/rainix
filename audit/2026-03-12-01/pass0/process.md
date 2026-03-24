# Pass 0: Process Review

## Evidence of thorough reading

### CLAUDE.md

- Sections: What is Rainix, Development Environment, Build Tasks (Solidity,
  Rust, Subgraph), Pinned Versions, Architecture, CI, Code Style
- Line 7: project description
- Lines 14-15: dev shell commands
- Lines 18: shell auto-source behavior
- Lines 25-29: Solidity tasks
- Lines 32-35: Rust tasks
- Lines 38-40: Subgraph tasks
- Lines 44-49: Pinned versions
- Lines 53-58: Architecture description
- Lines 62-65: CI workflows
- Lines 69-71: Code style

## Findings

### A01-1 [LOW] Inaccurate CI platform description

**File:** CLAUDE.md:63

The CLAUDE.md states test.yml runs on "Ubuntu + macOS (Intel & ARM)". However,
test.yml only uses `ubuntu-latest` and `macos-latest` — it does not include
`macos-13` (Intel). The Intel + Apple Silicon matrix is only in
`check-shell.yml`. A future session relying on this could incorrectly assume
Rust tests run on Intel macOS.

### A01-2 [LOW] Task path prefix varies by working directory but documentation uses single prefix

**File:** CLAUDE.md:22-29

Tasks are documented as `nix run ..#rainix-sol-test` (single `..`), which is
correct from a direct consumer repo. But from `test/fixture/` (the only runnable
location in this repo), the actual invocation is `nix run ../..#rainix-sol-test`
(double `../..`), as confirmed in `.github/workflows/test.yml:50`. The note
"From a consuming repo (or `test/fixture/`)" doesn't clarify that the path
prefix differs.

### A01-3 [LOW] Subgraph tasks not runnable via `nix run`

**File:** CLAUDE.md:38-40

Subgraph tasks (`subgraph-build`, `subgraph-test`, `subgraph-deploy`) are listed
under the "Build Tasks" section that opens with "All tasks are Nix packages run
via `nix run`." However, these tasks are not exported in `packages` — they're
only available on `PATH` inside the dev shell. A future session could attempt
`nix run ..#subgraph-build` and fail.

### A01-4 [INFO] Required environment variables for artifact deployment not documented

**File:** CLAUDE.md:29

`rainix-sol-artifacts` requires `DEPLOYMENT_KEY`, `ETH_RPC_URL`, and optionally
`ETHERSCAN_API_KEY`, `DEPLOY_BROADCAST`, `DEPLOY_VERIFY`, `DEPLOY_VERIFIER`,
`DEPLOY_VERIFIER_URL`, `DEPLOY_LEGACY`. None are mentioned. Similarly,
subgraph-deploy requires `GOLDSKY_TOKEN` and `GOLDSKY_NAME_AND_VERSION` (this
one is documented). A future session attempting deployment would need to
discover these from the flake source.
