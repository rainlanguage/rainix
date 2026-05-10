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
nix develop                # default shell (Solidity + Rust + Node + subgraph tools)
nix develop .#sol-shell    # slim Solidity-only shell — no rust, node, chromium, subgraph
nix develop .#tauri-shell  # Tauri desktop app development
```

The default shell auto-sources `.env` if present and runs
`npm ci --ignore-scripts` if `package.json` exists. `sol-shell` skips both.

### Build Tasks

All tasks are Nix packages run via `nix run`. From a consuming repo:

#### Solidity

- `nix run ..#rainix-sol-test` — forge test
- `nix run ..#rainix-sol-static` — slither + forge fmt check
- `nix run ..#rainix-sol-legal` — REUSE/DCL-1.0 license compliance
- `nix run ..#rainix-sol-artifacts` — deploy to testnet

#### Rust

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

### Reusable Workflows

#### Publish to Soldeer

`.github/workflows/publish-soldeer.yaml` is a `workflow_call` reusable that
pushes a Solidity package to soldeer.xyz on every `v*` tag. Consumer repos need
a five-line wrapper:

```yaml
name: Publish to Soldeer
on:
  push:
    tags: ["v*"]
jobs:
  publish:
    uses: rainlanguage/rainix/.github/workflows/publish-soldeer.yaml@main
    secrets: inherit
```

The package name defaults to the repo name with `.` replaced by `-`
(`rain.solmem` → `rain-solmem`), since soldeer rejects `.` in package names.
Override via the `package_name` input only if the registry name diverges.
`SOLDEER_API_TOKEN` must be set in the consumer repo (or org) secrets, and the
project must already exist on soldeer.xyz — the registry rejects pushes to
nonexistent projects.

#### rainix-sol-static

`.github/workflows/rainix-sol-static.yaml` runs `rainix-sol-static` (slither) on
Linux. Wrapper in the consumer repo:

```yaml
name: rainix-sol-static
on: [push]
jobs:
  static:
    uses: rainlanguage/rainix/.github/workflows/rainix-sol-static.yaml@main
```

Runs `forge soldeer install` automatically when a `soldeer.lock` is present.

#### rainix-sol-legal

`.github/workflows/rainix-sol-legal.yaml` runs `rainix-sol-legal` (`reuse lint`)
on Linux. Same wrapper shape as the static one:

```yaml
name: rainix-sol-legal
on: [push]
jobs:
  legal:
    uses: rainlanguage/rainix/.github/workflows/rainix-sol-legal.yaml@main
```

#### rainix-sol-test

`.github/workflows/rainix-sol-test.yaml` runs `rainix-sol-test` (`forge test`)
on Linux. Wrapper:

```yaml
name: rainix-sol-test
on: [push]
jobs:
  test:
    uses: rainlanguage/rainix/.github/workflows/rainix-sol-test.yaml@main
    secrets: inherit
```

`secrets: inherit` is required because the reusable wires the standard fork RPC
env vars (`ARBITRUM_RPC_URL`, `BASE_RPC_URL`, `BASE_SEPOLIA_RPC_URL`,
`FLARE_RPC_URL`, `POLYGON_RPC_URL`, `CI_DEPLOY_SEPOLIA_RPC_URL`) plus
`ETHERSCAN_API_KEY` and `DEPLOYMENT_KEY` from the consumer org's secrets/vars.
Repos that do no fork tests can ignore — empty values are harmless.

#### rainix-sol (composite)

`.github/workflows/rainix-sol.yaml` fans out static, legal, and test in parallel
— each on its own runner. Single wrapper for sol-only repos that want all three:

```yaml
name: rainix
on: [push]
jobs:
  rainix:
    uses: rainlanguage/rainix/.github/workflows/rainix-sol.yaml@main
    secrets: inherit
```

Consumers needing only one of the three should call the individual reusable
directly rather than this composite.

#### rainix-build-pointers

`.github/workflows/rainix-build-pointers.yaml` regenerates
`./script/BuildPointers.sol` artifacts, runs `forge fmt`, then asserts
`git diff --exit-code` — failing the PR if a maintainer changed
pointer-affecting source without committing the updated
`src/generated/*.pointers.sol` files.

```yaml
name: build-pointers
on: [push]
jobs:
  build-pointers:
    uses: rainlanguage/rainix/.github/workflows/rainix-build-pointers.yaml@main
```

Always runs through rainix's `sol-shell` (slim), regardless of the consumer's
default devShell.

#### rainix-rs-static

`.github/workflows/rainix-rs-static.yaml` runs `rainix-rs-static` (cargo fmt
check + clippy with `-D clippy::all`) on Linux. Wrapper:

```yaml
name: rainix-rs-static
on: [push]
jobs:
  rs-static:
    uses: rainlanguage/rainix/.github/workflows/rainix-rs-static.yaml@main
```

Always runs through rainix's `rust-shell` (rust toolchain only — no
chromium/sol/node), regardless of the consumer's default devShell.

#### rainix-rs-test

`.github/workflows/rainix-rs-test.yaml` runs `cargo test` on Linux and macOS.
Wrapper:

```yaml
name: rainix-rs-test
on: [push]
jobs:
  rs-test:
    uses: rainlanguage/rainix/.github/workflows/rainix-rs-test.yaml@main
```

Same shape as rs-static — runs through `rust-shell`. Consumers whose rust crate
compiles standalone (no live forge artifacts at compile time) can drop their
bespoke rs-test matrix in favour of this.

#### rainix-rs-wasm

`.github/workflows/rainix-rs-wasm.yaml` cross-compiles the workspace to
`wasm32-unknown-unknown` (release, library targets only). For consumers that
ship rust crates downstream as WASM (e.g. via wasm-bindgen for JS/TS), this
catches WASM-incompatible dependencies before they reach the JS build. Wrapper:

```yaml
name: rainix-rs-wasm
on: [push]
jobs:
  rs-wasm:
    uses: rainlanguage/rainix/.github/workflows/rainix-rs-wasm.yaml@main
```

`rust-shell`'s toolchain already includes the `wasm32-unknown-unknown` target,
so no extra setup is required.

#### rainix-rs (composite)

`.github/workflows/rainix-rs.yaml` fans out static, test, and wasm in parallel —
each on its own runner. Single wrapper for rust-shipping repos that want all
three:

```yaml
name: rainix-rs
on: [push]
jobs:
  rainix-rs:
    uses: rainlanguage/rainix/.github/workflows/rainix-rs.yaml@main
```

Consumers needing only one of the three should call the individual reusable
directly rather than this composite.

## Pinned Versions

- Rust: 1.94.0
- Solidity: solc 0.8.25
- Foundry: via foundry.nix
- Graph CLI: 0.69.2
- Goldsky CLI: 13.3.4

## License

DecentraLicense 1.0 — enforced via `reuse lint`.
