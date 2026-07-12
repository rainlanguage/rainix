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

mod frozen_snapshots;
mod no_submodules;
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
        other => {
            eprintln!(
                "rainix-static: unknown subcommand {other:?} \
                 (available: no-submodules, snapshots-append-only, soldeer-gate)"
            );
            std::process::exit(2);
        }
    }
}
