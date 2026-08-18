//! `soldeer-package-build` — build the package exactly as it publishes.
//!
//! `.soldeerignore` is a second, hand-maintained definition of what a library
//! is, disjoint from the source graph `forge build` walks in the repo: the repo
//! tree is complete, so a file the filter drops, or a shipped file whose import
//! the filter drops, is invisible to every check that runs against the repo.
//! This subcommand takes what `forge soldeer push --dry-run` would upload,
//! unpacks it into a scratch project with the build config and dependencies a
//! consumer supplies, and builds it — so an unresolvable import in the
//! published tree is red here instead of in a consumer's `forge build` after
//! `soldeer install`.

use crate::fail;
use crate::soldeer_gate::{newest_zip, read_local_field, read_zip, remove_zips, run_cmd};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// Files a consumer supplies and `.soldeerignore` commonly excludes: the build
/// config, its remappings, and its dependency lock. Taken from the repo when
/// the package does not ship them, so the published tree has something to build
/// with.
pub(crate) const SCAFFOLD_FILES: [&str; 3] = ["foundry.toml", "remappings.txt", "soldeer.lock"];

/// A zip entry name as a path under the scratch project, or None when it
/// escapes that root. Absolute paths, drive prefixes, `..` and `.` are rejected
/// rather than normalized, so a hostile or malformed entry name cannot write
/// outside the scratch directory.
pub(crate) fn safe_entry_path(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in Path::new(name).components() {
        match c {
            Component::Normal(part) => out.push(part),
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Copy each of `SCAFFOLD_FILES` the package does not ship from `root` into
/// `scratch`, and return the names copied in `SCAFFOLD_FILES` order. A file the
/// package ships is left alone — it is what a consumer would get. A file
/// neither side has is simply absent.
pub(crate) fn scaffold_missing(root: &Path, scratch: &Path) -> Vec<&'static str> {
    let mut copied = Vec::new();
    for f in SCAFFOLD_FILES {
        let dest = scratch.join(f);
        let src = root.join(f);
        if dest.exists() || !src.exists() {
            continue;
        }
        std::fs::copy(&src, &dest).unwrap_or_else(|e| {
            fail(&format!(
                "copy {} to {}: {e}",
                src.display(),
                dest.display()
            ))
        });
        copied.push(f);
    }
    copied
}

/// True when `foundry.toml` content opens a `[dependencies]` table, in either
/// the inline (`[dependencies]`) or per-dependency (`[dependencies.forge-std]`)
/// form, i.e. `forge soldeer install` has something to resolve.
pub(crate) fn declares_dependencies(toml: &str) -> bool {
    toml.lines().any(|l| {
        let l = l.trim();
        l == "[dependencies]" || l.starts_with("[dependencies.")
    })
}

/// Write one package entry under `scratch`, creating its parent directories.
/// Returns true when the entry is a Solidity source.
fn write_entry(scratch: &Path, name: &str, content: &[u8]) -> bool {
    let rel = safe_entry_path(name)
        .unwrap_or_else(|| fail(&format!("package entry {name:?} escapes the package root")));
    let dest = scratch.join(&rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| fail(&format!("create {}: {e}", parent.display())));
    }
    std::fs::write(&dest, content)
        .unwrap_or_else(|e| fail(&format!("write {}: {e}", dest.display())));
    rel.extension().is_some_and(|x| x == "sol")
}

/// Build the package `root` publishes, in `scratch` (a temp directory when
/// None). Runs inside sol-shell, so `forge` is on PATH.
pub(crate) fn run(root: &Path, scratch: Option<&Path>) {
    let toml_path = root.join("foundry.toml");
    let (name, version) = match (
        read_local_field(root, "name"),
        read_local_field(root, "version"),
    ) {
        (Some(n), Some(v)) => (n, v),
        _ => {
            println!(
                "soldeer-package-build: {} declares no [package] name and version, so no package publishes from it — skipping",
                toml_path.display()
            );
            return;
        }
    };

    // `forge soldeer push --dry-run` writes the package zip into `root` under a
    // name derived from the directory, so clear any stale zip first and take the
    // newest one afterwards. The scratch tree is created only after the zip has
    // been read and removed, so a scratch directory under `root` is not itself
    // part of what gets packaged.
    remove_zips(root);
    let spec = format!("{name}~{version}");
    run_cmd(
        Command::new("forge")
            .current_dir(root)
            .args(["soldeer", "push", &spec, "--dry-run"]),
        "forge soldeer push --dry-run",
    );
    let zip = newest_zip(root).unwrap_or_else(|| fail("forge dry-run produced no .zip"));
    let entries = read_zip(&zip);
    remove_zips(root);

    let scratch = match scratch {
        Some(p) => p.to_path_buf(),
        None => std::env::temp_dir().join(format!(
            "rainix-soldeer-package-build-{}",
            std::process::id()
        )),
    };
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)
        .unwrap_or_else(|e| fail(&format!("create {}: {e}", scratch.display())));

    let mut sol = 0usize;
    for (entry, content) in &entries {
        if write_entry(&scratch, entry, content) {
            sol += 1;
        }
    }
    scaffold_missing(root, &scratch);

    let toml = match std::fs::read_to_string(scratch.join("foundry.toml")) {
        Ok(t) => t,
        Err(e) => fail(&format!(
            "{spec} ships no foundry.toml and {} could not be read ({e}), so the published tree cannot be built",
            toml_path.display()
        )),
    };
    // Dependencies never ship inside a package; a consumer resolves them from
    // the declared `[dependencies]`, and so does this build.
    if declares_dependencies(&toml) {
        run_cmd(
            Command::new("forge")
                .current_dir(&scratch)
                .args(["soldeer", "install"]),
            "forge soldeer install",
        );
    }

    let status = Command::new("forge")
        .current_dir(&scratch)
        .arg("build")
        .status()
        .unwrap_or_else(|e| fail(&format!("forge build: failed to spawn: {e}")));
    if !status.success() {
        fail(&format!(
            "{spec} does not build as published — the unpacked tree is at {}. \
             Every source it needs must either be in the package or come from a declared dependency; \
             a path that resolves in the repo but not there is excluded by .soldeerignore.",
            scratch.display()
        ));
    }
    let _ = std::fs::remove_dir_all(&scratch);
    println!("soldeer-package-build: clean — {spec} builds as published ({sol} Solidity files)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-package-build-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn entry_paths_stay_inside_the_package() {
        assert_eq!(
            safe_entry_path("src/lib/LibFs.sol"),
            Some(PathBuf::from("src/lib/LibFs.sol"))
        );
        assert_eq!(
            safe_entry_path("README.md"),
            Some(PathBuf::from("README.md"))
        );
    }

    #[test]
    fn escaping_entry_paths_are_rejected() {
        assert_eq!(safe_entry_path(""), None);
        assert_eq!(safe_entry_path("/etc/passwd"), None);
        assert_eq!(safe_entry_path("../outside.sol"), None);
        assert_eq!(safe_entry_path("src/../../outside.sol"), None);
        assert_eq!(safe_entry_path("./src/A.sol"), None);
    }

    #[test]
    fn a_solidity_entry_is_counted_and_written_with_its_parents() {
        let d = tmp_dir();
        assert!(write_entry(&d, "src/lib/A.sol", b"contract A {}"));
        assert_eq!(
            std::fs::read_to_string(d.join("src/lib/A.sol")).unwrap(),
            "contract A {}"
        );
        assert!(!write_entry(&d, "README.md", b"hi"));
    }

    #[test]
    fn scaffolding_takes_only_what_the_package_omits() {
        let root = tmp_dir();
        let scratch = tmp_dir();
        std::fs::write(root.join("foundry.toml"), "root toml").unwrap();
        std::fs::write(root.join("remappings.txt"), "root remappings").unwrap();
        // soldeer.lock exists in neither; foundry.toml ships in the package.
        std::fs::write(scratch.join("foundry.toml"), "package toml").unwrap();

        assert_eq!(scaffold_missing(&root, &scratch), vec!["remappings.txt"]);
        assert_eq!(
            std::fs::read_to_string(scratch.join("foundry.toml")).unwrap(),
            "package toml"
        );
        assert_eq!(
            std::fs::read_to_string(scratch.join("remappings.txt")).unwrap(),
            "root remappings"
        );
        assert!(!scratch.join("soldeer.lock").exists());
    }

    #[test]
    fn dependencies_table_detection() {
        assert!(declares_dependencies(
            "[profile.default]\n\n[dependencies]\nforge-std = \"1\"\n"
        ));
        assert!(declares_dependencies("  [dependencies]  \n"));
        assert!(declares_dependencies(
            "[dependencies.forge-std]\nversion = \"1\"\n"
        ));
        assert!(!declares_dependencies("[profile.default]\nsrc = \"src\"\n"));
        assert!(!declares_dependencies("[dependencies_notreally]\n"));
    }
}
