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
//   snapshots-append-only [--base <ref>] [--root <dir>]
//       fail if the branch modifies or deletes an existing per-tag deploy-pin
//       snapshot under <root>/<tag>/ (default root src/generated, base
//       origin/main). Snapshots are frozen once on the base branch; a release
//       ADDS a new <tag>, never edits an existing one. Needs the base ref
//       fetched with history (fetch-depth: 0 + `git fetch origin <base>`).
//   soldeer-gate --package <name> [--github-output <file>]
//       Soldeer next-version content gate: compare the normalized content of what
//       `forge soldeer push --dry-run` would upload against the latest published
//       revision, and emit changed / version / next. Runs inside sol-shell, so
//       `forge` and `curl` are on PATH.
//   rpc-preflight [--root <dir>] [--github-env <file>] [--samples N]
//                 [--timeout N] [--no-archive]
//       Pick a working fork RPC endpoint per network and export it as
//       <NETWORK>_RPC_URL, so a dead upstream (quota, pruning node, gone host)
//       fails over instead of reddening every suite in the org. Candidates come
//       from the RAINIX_RPC_SECRET_<NET> / RAINIX_RPC_VARS_<NET> env vars merged
//       with hardcoded public archive defaults. Never prints a candidate URL.

mod frozen_snapshots;
mod no_submodules;
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
                 (available: no-submodules, snapshots-append-only, soldeer-gate, rpc-preflight)"
            );
            std::process::exit(2);
        }
    }
}
