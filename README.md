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
`BSC_RPC_URL`, `ETHEREUM_RPC_URL`, `FLARE_RPC_URL`, `HYPEREVM_RPC_URL`,
`POLYGON_RPC_URL`, `ROBINHOOD_RPC_URL`, `SEPOLIA_RPC_URL`,
`CI_DEPLOY_SEPOLIA_RPC_URL`) plus `ETHERSCAN_API_KEY` and `DEPLOYMENT_KEY` from
the consumer org's secrets/vars. Repos that do no fork tests can ignore — empty
values are harmless.

`CI_DEPLOY_SEPOLIA_RPC_URL` and the `ETH_RPC_URL` it is bound to are LEGACY and
scheduled for removal (rainlanguage/rainix#340). `ETH_RPC_URL` reads as though
it means Ethereum mainnet but resolves to the Sepolia-era deploy secret, so a
test trusting the name forks the wrong network. New code wants
`ETHEREUM_RPC_URL` or `SEPOLIA_RPC_URL` — whichever it actually means — both of
which the preflight health-checks before binding.

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

- `./script/Build.sol` → `src/generated/` (the rolling `candidate/` deploy pins,
  plus the alias and released-suites libs generated from the record). The same
  regeneration `rainix-tag-release` re-runs at publish time to prove the tagged
  commit's frozen snapshot is a fresh one.
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

#### rainix-autopublish

`.github/workflows/rainix-autopublish.yaml` is the LIBRARY repo release
workflow: it publishes on content change at merge, to Soldeer, crates.io, npm,
or any combination. Wrapper:

```yaml
name: Package Release
on:
  push:
    branches:
      - main
jobs:
  release:
    uses: rainlanguage/rainix/.github/workflows/rainix-autopublish.yaml@main
    with:
      soldeer-package: rain-lib-hash
    secrets: inherit
```

The caller owns the trigger; a push to the release branch is the convention.

A called workflow can only downgrade the caller's token, never elevate it, so
the grants this one needs have to reach it from the caller: `contents: write` to
push the `sol-v` tag and create the release, `actions: read` for the CI gate
that reads the commit's other workflow runs, and `id-token: write` for the npm
lane's OIDC publish. A caller that declares a `permissions:` block must list
every one it needs there. A caller that declares none — the example above —
inherits the repository's default `GITHUB_TOKEN` permissions, which covers it
only while those defaults are read-write; under read-only defaults the omission
fails at the tag push, the release, the gate, or npm auth, so such a repo has to
spell the block out.

What each input means is documented on the input itself; what the workflow does
with it is [Release lifecycle](#release-lifecycle) below.

#### rainix-tag-release

`.github/workflows/rainix-tag-release.yaml` is the DEPLOY repo release workflow:
it verifies and publishes the commit a `sol-v<x.y.z>` tag names. Wrapper:

```yaml
name: Package Release
on:
  push:
    tags:
      - sol-v*
jobs:
  release:
    uses: rainlanguage/rainix/.github/workflows/rainix-tag-release.yaml@main
    with:
      soldeer-package: rain-math-float-deploy
    secrets: inherit
```

The caller's `tags:` filter decides which tags release; the workflow only parses
the version out of the ref. `secrets: inherit` carries `SOLDEER_API_TOKEN`,
which this workflow declares required, and the fork RPC secrets the verification
step needs. See [Release lifecycle](#release-lifecycle).

### Release lifecycle

A repo that publishes is strictly one of two kinds, and the kind fixes the
workflow, the trigger, and where the version comes from:

|                        | library repo                             | deploy repo               |
| ---------------------- | ---------------------------------------- | ------------------------- |
| workflow               | `rainix-autopublish`                     | `rainix-tag-release`      |
| trigger                | push to the release branch               | `sol-v<x.y.z>` tag push   |
| publishes              | only if packaged content changed         | always — the tag is it    |
| version from           | the Soldeer registry, raised by `next-v` | the tag                   |
| foundry.toml `version` | absent by design                         | the last released version |
| deploy pins            | none — it pins no address                | frozen `src/generated/`   |

A library publishes an abstract surface (interfaces, libs) and pins no deployed
address, so it carries no per-version snapshot. A deploy repo records addresses:
its `src/generated/<version>/` snapshot pins the address and codehash of what it
deployed, frozen so consumers can rely on them, which makes its release a human
decision about a deployment that already happened.

Everything below about Soldeer is the Solidity lane. `rainix-autopublish` also
carries a cargo lane and an npm lane, which a library repo may use instead of or
alongside it; those gate on their own registry comparison (a normalized crate
content hash against crates.io, the `npm pack` shasum against the published one)
and take their version from the repo's own manifest via `cargo release` /
`npm version`, not from the rules below.

#### Library repos: what publishes, and when

Nothing publishes unless the packaged content changed. The Soldeer gate hashes
what `forge soldeer push --dry-run` would upload, minus two exclusions:
everything under `src/generated/` (derived from source, and a fresh directory
appears there every release, which would otherwise mark every merge as changed),
and `foundry.toml`'s `[external.package]` — or legacy `[package]` — section
together with the comment block attached above it. A push that changed nothing
short-circuits before the pre-publish test suite and the CI gate below.

Nothing bumps, tags or publishes until every other workflow run on that same
commit has finished green. A commit with no other runs at all is an error, not a
pass.

#### Library repos: where the version comes from

The **Soldeer registry is the version ledger** — no file in the repo is. The
published version is

```
max(patch_bump(newest published revision), highest next-v tag merged into HEAD)
```

under semver ordering. Three consequences a maintainer has to hold:

- **The default for every merge is a patch bump.** The pipeline never infers
  semver from a diff. Delete an entire public library and it publishes as a
  patch unless someone says otherwise.
- **`next-v<x.y.z>` git tags are how someone says otherwise.** To cut a minor or
  major, push `next-v<x.y.z>` on the commit that defines that version's content,
  before or as it lands on the release branch. The tag counts only while it is
  reachable from the head being published, which `git tag --merged HEAD`
  decides. That is why the release checkout is full-depth and why the gate
  refuses to run on a shallow one rather than silently missing an intent tag.
  Once the registry has passed it the tag falls inert under the `max`, so
  consumed and stale intent tags need no cleanup. A `next-v` tag whose remainder
  is not `<major>.<minor>.<patch>` fails the run loudly; a typo'd intent is
  never skipped.
- **A package's first publish requires a `next-v` tag** as the explicit version
  seed. With no revision on the registry there is nothing to patch-bump, and the
  gate will not guess `0.1.0`.

The tag is read once, from the checkout of the run that publishes, so both the
timing and the merge method matter — and neither way of getting them wrong goes
red:

- A tag pushed **after** the merge is invisible to the run that just published.
  That release ships as a patch, and the tag then raises whatever merges next,
  mislabelling two versions rather than one.
- A tag on a PR head survives a **merge commit** and does not survive a squash
  or rebase merge: the tagged commit never becomes an ancestor of the release
  branch, so no run ever sees it.

So push the tag on the PR head before the merge, and merge that PR with a merge
commit. There is no retroactive fix — once a version is published the registry
has it, and a repo that allows squash or rebase merges should turn them off if
it intends to use intent tags at all.

So "this is a breaking change" is a claim only a human can make, by tagging
`next-v<major>.0.0`. Nothing today fails a PR that changes the public surface
and ships it as a patch — see #327, which tracks that gate.

#### Library repos: foundry.toml carries no version

Deliberately. It is never read for a version and never rewritten, and its
release-metadata section is excluded from the content hash, so carrying,
editing, or deleting that section is content-neutral. A version added there
publishes nothing and means nothing. Do not add one back.

#### Library repos: what gets written

The Soldeer lane **never commits and never pushes to the branch**. The
`sol-v<x.y.z>` tag and its GitHub release are pushed as a tag ref, independent
of any branch push, so publishing works unchanged on a branch-protected main.
The cargo and npm lanes do commit their version bump and push it, so a repo on
those lanes needs a branch its deploy key can write.

#### Deploy repos: the release order

The deploy, the snapshot and the publish are three separate steps, in this
order, and only the last is `rainix-tag-release`:

1. **Deploy on-chain**, via the repo's own human-driven manual dispatch. It is
   deliberately not part of the release workflow: a per-network, funds- and
   RPC-dependent operation must not gate a one-shot tag publish where one
   transient failure blocks the release.
2. **PR the snapshot.** That PR regenerates and commits the frozen
   `src/generated/<version>/` deploy pins and bumps foundry.toml's
   `[external.package].version` (the legacy `[package]` form is still read). Its
   normal CI runs the append-only gate and the fork suite, so the pins consumers
   will trust are reviewed and verified before they can publish.
3. **Merge it, then push `sol-v<version>`** on the merged commit. The tag is the
   release authorization, and it must be an ancestor of the release branch — a
   tag cut from an unmerged branch is refused.
4. **The workflow verifies and publishes.** It re-attests the live chain matches
   the freshly regenerated pins, requires the tagged commit's frozen snapshot to
   be byte-identical to that regeneration, and only then publishes. Stale,
   hand-edited and never-cut snapshots all fail there, before anything is
   published. It writes to no branch either — main already carries the snapshot
   from step 2.

A deploy repo's `[package].version` **is** the last released version and moves
only at release time, in lockstep with the snapshot it describes — the opposite
of the library rule above.

#### Tag namespaces

The pipeline reads and writes exactly these:

| tag                | who                               | meaning                   |
| ------------------ | --------------------------------- | ------------------------- |
| `sol-v<x.y.z>`     | written by both release workflows | the published release     |
| `next-v<x.y.z>`    | read only, never created by CI    | library version intent    |
| `<crate>-v<x.y.z>` | written by the cargo lane         | the published crate       |
| `npm-<version>`    | written by the npm lane           | the published npm package |

On a deploy repo a `sol-v` push is also the release trigger, so creating one is
authorizing a release.

Every other tag is invisible to the pipeline. In particular a bare `v<x.y.z>` is
**not** an intent tag: the gate ignores every tag without the `next-v` prefix,
so pre-pipeline manual `v*` tags neither seed a version nor block one. A repo
whose first releases predate the pipeline therefore carries one version series
across two namespaces — `v*` for the manual publishes, `sol-v*` from the first
automated one — and a tool enumerating either prefix alone sees a truncated
history.

Never move or delete a `sol-v*` or `next-v*` tag: one rewrites what a release
was, the other rewrites what the next one is numbered.

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
archive and pruning backends. Ethereum, HyperEVM, Robinhood Chain and BNB Smart
Chain are latest-only in every consumer, so they are not held to the archive
bar, and neither are deploy/broadcast paths.

**Health also covers load, not just correctness.** Chain id, historical state
and historical `eth_call` are all correctness questions, and an endpoint that is
throttled rather than broken answers every one of them perfectly — then returns
`408 Request timeout on the free plan` the moment `forge` opens real fork
traffic (rainlanguage/rainix#340). Each candidate is therefore also hit with
`--burst` simultaneous `eth_call`s (16 by default), repeated for as many rounds
as there are samples, and is rejected when a majority of any one round comes
back throttled.

Rounds rather than a single burst, because these endpoints meter a token bucket:
the first burst after an idle period is served out of a full bucket and passes
even on an endpoint that then collapses. A majority rather than any single
failure, because every healthy public endpoint sheds the occasional request
under load, and rejecting on one would make the preflight flakier than the
outage it exists to prevent. `--burst 0` disables the check.

Because public rate limits are per-IP, the hardcoded default _order_ is only a
preference and cannot be right for every runner — the burst is what makes the
selection safe, by rejecting whichever candidate is throttled for the runner
running right now.

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
