# Pass 1: Security — flake.nix

## Evidence of thorough reading

**File:** flake.nix (414 lines)

### Inputs (lines 4-12)

- nixpkgs, flake-utils, rust-overlay, foundry, solc, nixpkgs-old

### Let bindings (lines 17-163)

- `wasm-bindgen-overlay` (line 17): overlay pinning wasm-bindgen-cli to 0.2.100
- `overlays` (line 20): combined overlay list
- `pkgs` (line 21): nixpkgs with overlays
- `old-pkgs` (line 22): pinned old nixpkgs
- `rust-version` (line 24): "1.94.0"
- `rust-toolchain` (line 25): stable rust with wasm32 target and extensions
- `rust-build-inputs` (line 31): list of rust build dependencies
- `sol-build-inputs` (line 48): list of solidity build dependencies
- `node-build-inputs` (line 56): nodejs_22
- `network-list` (line 57): ["base" "flare"]
- `the-graph` (line 58): mkDerivation for graph CLI 0.69.2, fetched by URL+SHA
  per system
- `goldsky` (line 90): mkDerivation for goldsky CLI 8.6.6, fetched by URL+SHA
  per system
- `tauri-build-inputs` (line 123): tauri dependencies including old-pkgs
- `tauri-release-env` (line 141): buildEnv for tauri releases
- `mkTask` (line 151): helper to create wrapped script derivations
- `rainix-sol-prelude` (line 165): forge install + build
- `rainix-sol-static` (line 180): slither + forge fmt --check
- `rainix-sol-legal` (line 190): reuse lint
- `rainix-sol-test` (line 199): forge test -vvv
- `rainix-sol-artifacts` (line 208): deploy with retry loop (up to 10 attempts)
- `rainix-rs-prelude` (line 253): no-op
- `rainix-rs-static` (line 261): cargo fmt + clippy
- `rainix-rs-test` (line 271): cargo test
- `rainix-rs-artifacts` (line 280): cargo build --release
- `rainix-tasks` (line 289): aggregated task list
- `subgraph-build` (line 302): forge build + npm ci + graph codegen/build
- `subgraph-test` (line 315): docker compose up
- `subgraph-deploy` (line 323): subgraph-build + goldsky deploy
- `subgraph-tasks` (line 333): aggregated subgraph task list
- `source-dotenv` (line 335): sources .env if present
- `tauri-shellhook-test` (line 343): BATS test runner (Darwin-only)

### Outputs (lines 354-413)

- `packages` (line 364): all rainix-* tasks + tauri-release-env
- `devShells.default` (line 371): full dev shell
- `devShells.tauri-shell` (line 385): tauri-specific dev shell with macOS
  workarounds

### Types/constants

- `network-list`: ["base" "flare"]
- `rust-version`: "1.94.0"

## Findings

No security findings. External binaries (graph CLI, goldsky) are fetched with
SHA256 pins preventing MITM. Shell scripts use `set -euxo pipefail`. Environment
variable interpolation uses Nix `''${}` escaping. `.env` sourcing is guarded by
file existence check and `.env` is gitignored.
