# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with
code in this repository.

## What is Rainix

Rainix is a Nix flake that provides development environments and build tasks for
the Rain Protocol ecosystem. It's a shared infrastructure flake consumed by
other Rain repos — the actual project code lives in downstream consumers. The
`test/fixture/` directory contains example contracts/crates used for CI
validation of the flake itself.

## Development Environment

Requires Nix with flakes enabled. Enter the dev shell before working:

```
nix develop          # default shell (Solidity + Rust + Node + subgraph tools)
nix develop .#sol-shell   # slim Solidity-only shell
nix develop .#rust-shell  # slim Rust-only shell
```

The shell auto-sources `.env` if present and runs `npm ci --ignore-scripts` if
`package.json` exists.

## Build Tasks

All tasks are Nix packages run via `nix run`. From a consuming repo use `..#`
(e.g., `nix run ..#rainix-sol-test`); from `test/fixture/` use `../..#` (e.g.,
`nix run ../..#rainix-sol-test`). Examples below use the consuming-repo prefix:

### Solidity

- `nix run ..#rainix-sol-prelude` — `forge install && forge build`
- `nix run ..#rainix-sol-test` — `forge test -vvv`
- `nix run ..#rainix-sol-static` — `slither . && forge fmt --check`
- `nix run ..#rainix-sol-single-contract` — fail if any tracked `.sol` declares
  more than one top-level `contract`/`abstract contract` (one-contract-per-file
  convention; `library`/`interface` not counted)
- `nix run ..#rainix-sol-legal` — `reuse lint` (REUSE/DCL-1.0 license
  compliance)
- `nix run ..#rainix-sol-artifacts` — deploy to testnet via `script/Deploy.sol`

### Rust

- `nix run ..#rainix-rs-prelude` — (currently no-op, placeholder for env prep)
- `nix run ..#rainix-rs-test` — `cargo test`
- `nix run ..#rainix-rs-static` —
  `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D clippy::all`

### Subgraph (dev shell only, not `nix run` targets)

- `subgraph-build` — forge build + npm ci + graph codegen/build
- `subgraph-test` — `docker compose up` in `./subgraph`
- `subgraph-deploy` — requires `GOLDSKY_TOKEN` and `GOLDSKY_NAME_AND_VERSION`

## Pinned Versions

- Rust: 1.94.0 (with `wasm32-unknown-unknown` target)
- Solidity: solc 0.8.25
- Foundry: via foundry.nix
- Graph CLI: 0.69.2
- Goldsky CLI: 8.6.6
- wasm-bindgen-cli: 0.2.122 (pinned via `buildWasmBindgenCli` in flake.nix; must
  match the `wasm-bindgen` crate version downstream lockfiles resolve to, or
  `wasm-bindgen` over the wasm file fails)

## Architecture

The flake exports:

- **`packages`**: All `rainix-*` task derivations.
- **`devShells`**: `default` (full toolchain), `sol-shell` (Solidity only),
  `rust-shell` (Rust only).
- **Reusable outputs**: `pkgs`, `rust-toolchain`, `rust-build-inputs`,
  `sol-build-inputs`, `node-build-inputs`, `mkTask` — consumed by downstream
  Rain flakes to compose their own tasks/shells

`mkTask` is the core abstraction: it creates self-contained Nix derivations
wrapping shell scripts with their dependencies on `PATH`.

## CI

Defined in `.github/workflows/`:

- **test.yml** — runs sol and rs tasks against `test/fixture/` on Ubuntu + macOS
  (ARM only; sol tasks Ubuntu-only)
- **check-shell.yml** — verifies dev shell tools are available
- **pr-assessment.yaml** — PR size assessment

### Flake-ref pinning in reusable workflows

The reusable workflows (`.github/workflows/rainix-*.yaml`) invoke the dev shells
via `nix develop github:rainlanguage/rainix/<sha>#<devshell>` — pinned to an
explicit commit **sha**, never the bare `github:rainlanguage/rainix#…` (HEAD)
form. Unpinned, nix resolves HEAD through `api.github.com/.../commits/HEAD`,
which GitHub **burst-rate-limits (429)** under CI load (the error body comes back
gzipped and nix mis-parses it as JSON) — this was the dominant org-wide CI flake.
A full sha makes nix skip that API call and fetch the tarball directly.
Authenticating the call does NOT help (it's a secondary limit, not missing auth);
pinning is the fix.

**Every flake ref across every reusable shares ONE sha.** To bump the toolchain,
find-replace the old sha with the new across `.github/workflows/*.yaml`, then
sanity-check with `nix flake show github:rainlanguage/rainix/<new-sha>` that the
referenced devshells (`sol-shell`, `rust-shell`, `rust-node-shell`,
`subgraph-shell`) still resolve at it. Never add a bare unpinned
`github:rainlanguage/rainix#…` ref — it reintroduces the 429. (Single-sourcing
this repeated sha so a bump is one line is tracked in #248.)

## Code Style

- Rust: `cargo fmt` and `cargo clippy` with all warnings denied
  (`-D clippy::all`)
- Solidity: `forge fmt` and `slither` static analysis
- License: DecentraLicense 1.0, enforced via `reuse lint`
