# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Rainix

Rainix is a Nix flake that provides development environments and build tasks for the Rain Protocol ecosystem. It's a shared infrastructure flake consumed by other Rain repos — the actual project code lives in downstream consumers. The `test/fixture/` directory contains example contracts/crates used for CI validation of the flake itself.

## Development Environment

Requires Nix with flakes enabled. Enter the dev shell before working:

```
nix develop        # default shell (Solidity + Rust + Node + subgraph tools)
nix develop .#tauri-shell  # Tauri desktop app development (macOS-specific quirks)
```

The shell auto-sources `.env` if present and runs `npm ci --ignore-scripts` if `package.json` exists.

## Build Tasks

All tasks are Nix packages run via `nix run`. From a consuming repo (or `test/fixture/`):

### Solidity
- `nix run ..#rainix-sol-prelude` — `forge install && forge build`
- `nix run ..#rainix-sol-test` — `forge test -vvv`
- `nix run ..#rainix-sol-static` — `slither . && forge fmt --check`
- `nix run ..#rainix-sol-legal` — `reuse lint` (REUSE/DCL-1.0 license compliance)
- `nix run ..#rainix-sol-artifacts` — deploy to testnet via `script/Deploy.sol`

### Rust
- `nix run ..#rainix-rs-prelude` — (currently no-op, placeholder for env prep)
- `nix run ..#rainix-rs-test` — `cargo test`
- `nix run ..#rainix-rs-static` — `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D clippy::all`
- `nix run ..#rainix-rs-artifacts` — `cargo build --release`

### Subgraph
- `subgraph-build` — forge build + npm ci + graph codegen/build
- `subgraph-test` — `docker compose up` in `./subgraph`
- `subgraph-deploy` — requires `GOLDSKY_TOKEN` and `GOLDSKY_NAME_AND_VERSION`

## Pinned Versions

- Rust: 1.94.0 (with `wasm32-unknown-unknown` target)
- Solidity: solc 0.8.19
- Foundry: via foundry.nix
- Graph CLI: 0.69.2
- Goldsky CLI: 8.6.6
- wasm-bindgen-cli: 0.2.100

## Architecture

The flake exports:
- **`packages`**: All `rainix-*` task derivations plus `tauri-release-env`
- **`devShells`**: `default` (full toolchain) and `tauri-shell` (Tauri + macOS workarounds)
- **Reusable outputs**: `pkgs`, `rust-toolchain`, `rust-build-inputs`, `sol-build-inputs`, `node-build-inputs`, `mkTask`, `network-list` — consumed by downstream Rain flakes to compose their own tasks/shells

`mkTask` is the core abstraction: it creates self-contained Nix derivations wrapping shell scripts with their dependencies on `PATH`.

## CI

Defined in `.github/workflows/`:
- **test.yml** — runs sol and rs tasks against `test/fixture/` on Ubuntu + macOS (Intel & ARM)
- **check-shell.yml** — verifies dev shell tools are available
- **pr-assessment.yaml** — PR size assessment

## Code Style

- Rust: `cargo fmt` and `cargo clippy` with all warnings denied (`-D clippy::all`)
- Solidity: `forge fmt` and `slither` static analysis
- License: DecentraLicense 1.0, enforced via `reuse lint`
