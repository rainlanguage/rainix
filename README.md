# Rainix

Nix flake providing development environments and build tasks for the
[Rain Protocol](https://rainprotocol.xyz) ecosystem.

Rainix is shared infrastructure consumed by other Rain repos — the actual
project code lives in downstream consumers.

## Usage

Add Rainix as a flake input:

```nix
{
  inputs.rainix.url = "github:rainlanguage/rainix";
}
```

### Dev Shells

Requires Nix with flakes enabled.

```sh
nix develop        # default shell (Solidity + Rust + Node + subgraph tools)
nix develop .#tauri-shell  # Tauri desktop app development
```

The default shell auto-sources `.env` if present and runs
`npm ci --ignore-scripts` if `package.json` exists.

### Build Tasks

All tasks are Nix packages run via `nix run`. From a consuming repo:

#### Solidity

- `nix run ..#rainix-sol-prelude` — forge install + build
- `nix run ..#rainix-sol-test` — forge test
- `nix run ..#rainix-sol-static` — slither + forge fmt check
- `nix run ..#rainix-sol-legal` — REUSE/DCL-1.0 license compliance
- `nix run ..#rainix-sol-artifacts` — deploy to testnet

#### Rust

- `nix run ..#rainix-rs-prelude` — environment prep (currently no-op)
- `nix run ..#rainix-rs-test` — cargo test
- `nix run ..#rainix-rs-static` — cargo fmt + clippy
- `nix run ..#rainix-rs-artifacts` — cargo build --release

### Reusable Outputs

Downstream flakes can compose their own tasks and shells using:

- `pkgs` — nixpkgs with all overlays applied
- `rust-toolchain` — pinned Rust toolchain
- `rust-build-inputs`, `sol-build-inputs`, `node-build-inputs` — dependency
  lists
- `mkTask` — create Nix derivations wrapping shell scripts with dependencies on
  PATH
- `network-list` — supported network names

## Pinned Versions

- Rust: 1.94.0
- Solidity: solc 0.8.25
- Foundry: via foundry.nix
- Graph CLI: 0.69.2
- Goldsky CLI: 8.6.6

## License

DecentraLicense 1.0 — enforced via `reuse lint`.
