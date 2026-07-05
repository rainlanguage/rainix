// rainix-static — org-wide static checks as subcommands (rainlanguage/rainix#255).
// Each check prints its offenders and the process exits nonzero if any check fails;
// a clean check prints "<name>: clean".
//
// Usage: rainix-static <check> [dir]
// Checks: no-submodules

use std::path::Path;
use std::process::Command;

/// Git submodules are banned org-wide: dependencies come from soldeer / npm /
/// nix flakes, never vendored submodule pointers (they break shallow clones,
/// soldeer consumers, and reproducibility). Two detection legs:
///   1. a ROOT .gitmodules file (git only reads the repo root's; a vendored
///      dependencies/*/.gitmodules is inert content and is NOT flagged);
///   2. any committed gitlink entry (mode 160000) — catches a submodule whose
///      .gitmodules was deleted but whose pointer is still tracked.
/// Returns the offender report lines; empty means clean.
fn no_submodules(dir: &Path) -> Vec<String> {
    let mut offenders = Vec::new();

    if dir.join(".gitmodules").is_file() {
        offenders.push("Root .gitmodules found — submodules are banned (use soldeer/npm/nix):".to_string());
        offenders.push(format!("  {}", dir.join(".gitmodules").display()));
    }

    let gitlinks = gitlink_paths(dir);
    if !gitlinks.is_empty() {
        offenders.push("Committed gitlink entries (mode 160000) found — submodule pointers are banned:".to_string());
        for p in gitlinks {
            offenders.push(format!("  {p}"));
        }
    }

    offenders
}

/// Paths of index entries with mode 160000 (gitlinks) per `git ls-files -s`,
/// whose line format is "<mode> <oid> <stage>\t<path>". A directory that is
/// not a git repo yields none (the file leg still applies to it).
fn gitlink_paths(dir: &Path) -> Vec<String> {
    let out = match Command::new("git").args(["-C"]).arg(dir).args(["ls-files", "-s"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mode = l.split_whitespace().next()?;
            if mode != "160000" {
                return None;
            }
            Some(l.split('\t').nth(1)?.to_string())
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let check = args.get(1).map(String::as_str).unwrap_or("");
    let dir = Path::new(args.get(2).map(String::as_str).unwrap_or("."));
    let (name, offenders) = match check {
        "no-submodules" => ("no-submodules", no_submodules(dir)),
        other => {
            eprintln!("rainix-static: unknown check {other:?} (available: no-submodules)");
            std::process::exit(2);
        }
    };
    if offenders.is_empty() {
        println!("{name}: clean");
    } else {
        for line in offenders {
            println!("{line}");
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_repo(git: bool) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        if git {
            assert!(Command::new("git").arg("-C").arg(&d).args(["init", "-q"]).status().unwrap().success());
        }
        d
    }

    #[test]
    fn clean_repo_passes() {
        let d = tmp_repo(true);
        std::fs::write(d.join("README.md"), "x").unwrap();
        assert!(no_submodules(&d).is_empty());
    }

    #[test]
    fn root_gitmodules_fails_and_is_named() {
        let d = tmp_repo(true);
        std::fs::write(d.join(".gitmodules"), "[submodule \"lib/forge-std\"]\n").unwrap();
        let off = no_submodules(&d);
        assert!(!off.is_empty());
        assert!(off.iter().any(|l| l.contains(".gitmodules")), "{off:?}");
    }

    #[test]
    fn committed_gitlink_without_gitmodules_fails() {
        let d = tmp_repo(true);
        assert!(Command::new("git")
            .arg("-C").arg(&d)
            .args(["update-index", "--add", "--cacheinfo",
                   "160000,0000000000000000000000000000000000000001,lib/ghost"])
            .status().unwrap().success());
        let off = no_submodules(&d);
        assert!(off.iter().any(|l| l.contains("lib/ghost")), "{off:?}");
        assert!(off.iter().any(|l| l.contains("160000")), "{off:?}");
    }

    #[test]
    fn vendored_non_root_gitmodules_is_inert() {
        let d = tmp_repo(true);
        std::fs::create_dir_all(d.join("dependencies/dep")).unwrap();
        std::fs::write(d.join("dependencies/dep/.gitmodules"), "[submodule \"x\"]\n").unwrap();
        assert!(no_submodules(&d).is_empty());
    }

    #[test]
    fn non_git_dir_still_fails_on_file_leg() {
        let d = tmp_repo(false);
        std::fs::write(d.join(".gitmodules"), "[submodule \"x\"]\n").unwrap();
        assert!(!no_submodules(&d).is_empty());
    }
}
