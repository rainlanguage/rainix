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

## Pinned Versions

- Rust: 1.94.0
- Solidity: solc 0.8.25
- Foundry: via foundry.nix
- Graph CLI: 0.69.2
- Goldsky CLI: 13.3.4

## License

DecentraLicense 1.0 — enforced via `reuse lint`.
