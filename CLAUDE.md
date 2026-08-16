# CLAUDE.md

Only what a capable agent would get _wrong_ by looking at the repo. Everything
discoverable — layout, dev shells, build tasks, pinned versions, which command
CI runs — is deliberately absent, and stays absent (rainlanguage/rainix#298).

## Tooling is Rust — never Python, never bash logic

Anything the flake or CI needs that does real **work** — a content gate, a
hash-and-normalize, a version bump, an allowlist check, JSON parsing, a
comparison — ships as a subcommand of the `rainix-static` Rust binary and is
invoked directly. There is no ambient `python3` in CI, by policy, so a
`python3 -c …` step (or a base64'd script decoded at runtime) is a defect.

Bash may orchestrate — wire steps together, set env, move files. The moment it
parses, hashes, computes or branches over data, that logic is Rust; de-bashing
into a thin bash wrapper is still bash. When in doubt, Rust.

New repo conventions are enforced the same way, as a static check, so they hold
mechanically rather than by reviewer memory.

## Never use an unpinned flake ref

Reusable workflows invoke dev shells as
`nix develop github:rainlanguage/rainix/<sha>#<devshell>` — an explicit commit
sha, never the bare `github:rainlanguage/rainix#…` HEAD form. Unpinned, nix
resolves HEAD through `api.github.com`, which **burst-rate-limits (429)** under
CI load (the error body comes back gzipped and nix mis-parses it as JSON); this
was the dominant org-wide CI flake. A full sha makes nix skip that call and
fetch the tarball directly. Authenticating does NOT help — it is a secondary
limit, not missing auth. Every ref in a workflow file shares one sha, via that
file's top-level `env.RAINIX_SHA`.
