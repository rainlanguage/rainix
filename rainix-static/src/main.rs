// rainix-static — general rainix tooling as subcommands (rainlanguage/rainix#255).
// Houses org-wide static checks AND the CI release-tooling that would otherwise be
// inline bash or Python in a workflow. Per the "tooling is Rust" rule (CLAUDE.md),
// logic — hashing, JSON parsing, version math, content gates — lives here as one
// testable binary; workflows only orchestrate it. Each subcommand is its own
// module: static checks in `no_submodules`, CI release tooling in `soldeer_gate`.
//
// Static checks print their offenders and exit nonzero on failure ("<name>: clean"
// otherwise). Tooling subcommands print machine outputs (key=value lines) to the
// file named by --github-output, or to stdout when it is omitted.
//
// Usage: rainix-static <subcommand> [args]
// Subcommands:
//   no-submodules [dir]
//       fail if the repo vendors git submodules.
//   agent-context-cap [dir]
//       fail if the agent context the repo loads at the START of every session
//       exceeds the byte cap — it is in the window on every turn whether the
//       turn needs it or not, so its size taxes all work done in the repo. The
//       total is CLAUDE.md (or .claude/CLAUDE.md), plus everything they pull in
//       transitively via @path imports, plus every .claude/rules/**.md without
//       `paths:` frontmatter. On-demand context is NOT charged: path-scoped
//       rules, subdirectory CLAUDE.md, CLAUDE.local.md. Prints the per-file
//       breakdown on failure. The cap is a floor-only ratchet that may only
//       ever be lowered. A repo with no agent context passes.
//   prompt-cap --paths <globs> --cap <bytes> [--root <dir>]
//       the same check as agent-context-cap, over the prompt files a repo
//       points it at: a prompt is read whole at launch and re-read on every
//       turn of the run, so its bytes are paid per turn. The cap is on the
//       TOTAL over the matched glob (a per-file cap is evaded by splitting the
//       file), and any repo file a prompt NAMES is charged with it — one hop,
//       since telling the agent to read a file loads it, while what that file
//       mentions is nobody's instruction.
//       Nothing is stripped: a shell script reads the bytes on disk. Which
//       files are prompts and what they may weigh is per-repo, so both are an
//       input, and a glob matching nothing is an error rather than a pass.
//   snapshots-append-only [--base <ref>] [--root <dir>]
//       fail if the branch modifies or deletes an existing per-tag deploy-pin
//       snapshot under <root>/<tag>/ (default root src/generated, base
//       origin/main). Snapshots are frozen once on the base branch; a release
//       ADDS a new <tag>, never edits an existing one. Needs the base ref
//       fetched with history (fetch-depth: 0 + `git fetch origin <base>`).
//   soldeer-gate --package <name> [--github-output <file>]
//       Soldeer content gate: compare the normalized content of what
//       `forge soldeer push --dry-run` would upload against the newest published
//       revision (foundry.toml's `[external.package]` / legacy `[package]`
//       release-metadata section is excluded from the hash), derive the publish
//       version as max(patch_bump(newest published), newest `next-v<x.y.z>`
//       intent tag merged into HEAD) under semver ordering — a first publish
//       requires an intent tag — and emit changed / version. Needs a full-depth
//       checkout with tags; runs inside sol-shell, so `forge`, `curl` and `git`
//       are on PATH.
//   rpc-preflight [--root <dir>] [--github-env <file>] [--samples N]
//                 [--timeout N] [--no-archive]
//       Pick a working fork RPC endpoint per network and export it as
//       <NETWORK>_RPC_URL, so a dead upstream (quota, pruning node, gone host)
//       fails over instead of reddening every suite in the org. Candidates come
//       from the RAINIX_RPC_SECRET_<NET> / RAINIX_RPC_VARS_<NET> env vars merged
//       with hardcoded public archive defaults. Never prints a candidate URL.

mod agent_context_cap;
mod context_bytes;
mod frozen_snapshots;
mod no_submodules;
mod prompt_cap;
mod rpc_preflight;
mod soldeer_gate;

use std::path::Path;

/// Print a GitHub Actions error annotation and exit nonzero. Shared by every
/// subcommand, so it lives at the crate root (`crate::fail`).
pub(crate) fn fail(msg: &str) -> ! {
    eprintln!("::error::{msg}");
    std::process::exit(1);
}

/// Value following `--name` (or `--name=value`) in the argument list.
fn flag(args: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&prefix) {
            return Some(v.to_string());
        }
    }
    None
}

/// Numeric value following `--name`, or `default` when absent. A present but
/// unparseable value is a typo, not a request for the default — fail loud.
fn num(args: &[String], name: &str, default: u32) -> u32 {
    match flag(args, name) {
        None => default,
        Some(v) => v
            .parse()
            .unwrap_or_else(|_| fail(&format!("{name}: {v:?} is not a positive integer"))),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sub = args.get(1).map(String::as_str).unwrap_or("");
    match sub {
        "no-submodules" => {
            let dir = Path::new(args.get(2).map(String::as_str).unwrap_or("."));
            let offenders = no_submodules::check(dir);
            if offenders.is_empty() {
                println!("no-submodules: clean");
            } else {
                for line in offenders {
                    println!("{line}");
                }
                std::process::exit(1);
            }
        }
        "agent-context-cap" => {
            let dir = Path::new(args.get(2).map(String::as_str).unwrap_or("."));
            let (total, offenders) = agent_context_cap::check(dir);
            if offenders.is_empty() {
                println!(
                    "agent-context-cap: clean — {total} bytes loaded at session start (cap {})",
                    agent_context_cap::CAP_BYTES
                );
            } else {
                for line in offenders {
                    println!("{line}");
                }
                std::process::exit(1);
            }
        }
        "prompt-cap" => {
            let root = flag(&args, "--root").unwrap_or_else(|| ".".to_string());
            let patterns = prompt_cap::parse_patterns(
                &flag(&args, "--paths")
                    .unwrap_or_else(|| fail("prompt-cap: --paths <globs> required, one per line")),
            );
            let cap =
                flag(&args, "--cap").unwrap_or_else(|| fail("prompt-cap: --cap <bytes> required"));
            let cap: u64 = cap.parse().unwrap_or_else(|_| {
                fail(&format!("prompt-cap: --cap {cap:?} is not a byte count"))
            });
            match prompt_cap::check(Path::new(&root), &patterns, cap) {
                Err(e) => fail(&format!("prompt-cap: {e}")),
                Ok((total, offenders)) if offenders.is_empty() => {
                    println!("prompt-cap: clean — {total} bytes of prompt (cap {cap})")
                }
                Ok((_, offenders)) => {
                    for line in offenders {
                        println!("{line}");
                    }
                    std::process::exit(1);
                }
            }
        }
        "soldeer-gate" => {
            let pkg = flag(&args, "--package")
                .unwrap_or_else(|| fail("soldeer-gate: --package <name> required"));
            soldeer_gate::run(&pkg, flag(&args, "--github-output").as_deref());
        }
        "snapshots-append-only" => {
            let base = flag(&args, "--base").unwrap_or_else(|| "origin/main".to_string());
            let root = flag(&args, "--root").unwrap_or_else(|| "src/generated".to_string());
            match frozen_snapshots::check(&base, &root) {
                Err(e) => fail(&e),
                Ok(offenders) if offenders.is_empty() => println!("snapshots-append-only: clean"),
                Ok(offenders) => {
                    for line in offenders {
                        println!("{line}");
                    }
                    std::process::exit(1);
                }
            }
        }
        "rpc-preflight" => {
            let root = flag(&args, "--root").unwrap_or_else(|| ".".to_string());
            // There is no stdout fallback on purpose: the selected URL may be
            // secret-derived, so the only sink it may reach is a file. Without
            // one, fail rather than print.
            let github_env = flag(&args, "--github-env")
                .or_else(|| std::env::var("GITHUB_ENV").ok())
                .unwrap_or_else(|| {
                    fail("rpc-preflight: --github-env <file> required (or set GITHUB_ENV)")
                });
            let samples = num(&args, "--samples", 3);
            let timeout = num(&args, "--timeout", 15);
            // Deploy/broadcast paths only ever read head state; requiring
            // archive there would reject a healthy pruning endpoint.
            let archive = !args.iter().any(|a| a == "--no-archive");
            rpc_preflight::run(
                Path::new(&root),
                Path::new(&github_env),
                samples,
                timeout,
                archive,
            );
        }
        other => {
            eprintln!(
                "rainix-static: unknown subcommand {other:?} \
                 (available: no-submodules, agent-context-cap, prompt-cap, \
                 snapshots-append-only, soldeer-gate, rpc-preflight)"
            );
            std::process::exit(2);
        }
    }
}
