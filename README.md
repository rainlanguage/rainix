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
nix develop .#sol-shell    # slim Solidity-only shell — no rust, node, subgraph
nix develop .#rust-shell   # slim Rust-only shell — no sol, node
```

The default shell auto-sources `.env` if present and runs
`npm ci --ignore-scripts` if `package.json` exists. `sol-shell` skips both.

### Updating a consumer to the latest rainix

`lib/update-rainix.sh` bumps a consuming repo to the latest rainix and re-locks
Soldeer. Run it from the repo root (it makes local changes only — review and
commit yourself):

```sh
/path/to/rainix/lib/update-rainix.sh
```

It bumps the `rainix` flake input to the latest default branch and — for
Solidity repos — re-locks Soldeer and runs a sanity `forge build`. Soldeer
dependency _version_ bumps are left to the developer (edit `foundry.toml`, run
`forge soldeer update`, fix the version-suffixed imports), since bumping blindly
can break builds when a transitive dependency pins an older version.

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

### Reusable Outputs

Downstream flakes can compose their own tasks and shells using:

- `pkgs` — nixpkgs with all overlays applied
- `rust-toolchain` — pinned Rust toolchain
- `rust-build-inputs`, `sol-build-inputs`, `node-build-inputs` — dependency
  lists
- `mkTask` — create Nix derivations wrapping shell scripts with dependencies on
  PATH

### Reusable Workflows

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
`ETHEREUM_RPC_URL`, `FLARE_RPC_URL`, `HYPEREVM_RPC_URL`, `POLYGON_RPC_URL`,
`CI_DEPLOY_SEPOLIA_RPC_URL`) plus `ETHERSCAN_API_KEY` and `DEPLOYMENT_KEY` from
the consumer org's secrets/vars. Repos that do no fork tests can ignore — empty
values are harmless.

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

#### rainix-copy-artifacts

`.github/workflows/rainix-copy-artifacts.yaml` regenerates committed generated
Solidity artifacts from source and asserts `git diff --exit-code` — failing the
PR if a maintainer changed source without committing the regenerated files. In a
single job it runs whichever of these the repo has:

- `./script/BuildPointers.sol` → `src/generated/*.pointers.sol`
- `forge build` + `./script/CopyArtifacts.sol --ffi` → committed ABI JSON

then `forge fmt` and the `git diff` assert.

```yaml
name: copy-artifacts
on: [push]
jobs:
  copy-artifacts:
    uses: rainlanguage/rainix/.github/workflows/rainix-copy-artifacts.yaml@main
    secrets: inherit
```

This replaces the former `rainix-build-pointers` reusable — a pointer-only repo
just omits `CopyArtifacts.sol` (the copy step is skipped via `hashFiles`).
Always runs through rainix's `sol-shell` (slim), regardless of the consumer's
default devShell. `secrets: inherit` carries `CACHIX_AUTH_TOKEN`.

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

Always runs through rainix's `rust-shell` (rust toolchain only — no sol/node),
regardless of the consumer's default devShell.

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

### Fork RPC endpoints

Each `<NETWORK>_RPC_URL` is chosen at job start by the `rpc-preflight` composite
action, not bound to a single configured URL. Foundry maps one `[rpc_endpoints]`
alias to exactly one URL and `--fork-retries` only retries that same URL, so a
dead upstream — plan quota exhausted, pruning node, host gone — cannot be
recovered inside `forge`. The preflight recovers it one layer up.

**Candidates are a merged pool, not a fallback chain.** For each network:

| source                            | holds                              | order |
| --------------------------------- | ---------------------------------- | ----- |
| secret `RPC_URL_<NETWORK>_FORK`   | keyed/paid URLs (masked)           | first |
| variable `RPC_URL_<NETWORK>_FORK` | public keyless URLs (visible)      | next  |
| hardcoded public archive defaults | measured keyless archive endpoints | last  |

Both the secret and the variable hold a **newline-separated list**; a single
bare URL is a one-element list, which is what they contain today. Every entry in
every source is a real candidate — the variable's URLs are tried even when the
secret is set. The order only expresses preference: the paid endpoint first, the
org's curated public list next, the hardcoded safety net when both are
exhausted. Keeping keyed URLs in the secret and keyless ones in the variable is
the point of merging: a public archive endpoint can back up a keyed one without
putting a non-secret into a secret (where masking makes logs unreadable for no
security benefit). `#` starts a comment, so a candidate can be parked with a
note.

**Health is archive-aware.** A candidate must report the right chain id, then
serve historical account state and a historical `eth_call` at the deepest block
any repo in the org pins for that network, three times consecutively. An
`eth_blockNumber` check would happily select a pruning node that then fails the
suite with `trying to fork from an older block with a non-archive node`; a
code-only check would select a host that answers no `eth_call` at all; and a
single sample would qualify a load balancer that round-robins over a mix of
archive and pruning backends. Ethereum and HyperEVM are latest-only in every
consumer, so they are not held to the archive bar, and neither are
deploy/broadcast paths.

**No candidate URL is ever printed.** Logs name the _source_ (`secret[0]`,
`variable[1]`, `default[0]`) and a typed reason, never a URL:

```
rpc-preflight: arbitrum: secret[0] rejected: quota exhausted / rate limited (rpc error -32001)
rpc-preflight: arbitrum: SELECTED variable[0] (chain 42161, archive at block 280000000, 3/3 samples)
```

Only networks the repo actually references are probed, and a network with no
candidates at all is left exactly as it is today.

## Pinned Versions

- Rust: 1.94.0
- Solidity: solc 0.8.25
- Foundry: via foundry.nix
- Graph CLI: 0.69.2
- Goldsky CLI: 13.3.4

## License

DecentraLicense 1.0 — enforced via `reuse lint`.
