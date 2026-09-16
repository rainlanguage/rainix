//! `install-staged-config` — install what a repo's codegen staged in
//! `.staged-config/` over the files of the same name at the repo root.
//!
//! A repo can generate its own `foundry.toml` network sections and
//! `.env.example` endpoint variables from a single roster, but it cannot WRITE
//! the project root's `foundry.toml`: foundry refuses every filesystem
//! cheatcode write to it, whatever `fs_permissions` says and however the path
//! is spelled. So the generator reads each file, splices its blocks and writes
//! the result to `.staged-config/`, and this installs it. Nothing here needs
//! forge, nix or `--ffi`; granting `--ffi` to every consumer's build is the
//! alternative this avoids.
//!
//! An ABSENT directory is a skip. This runs in every sol repo in the org, and
//! the directory's presence is the only signal available there — a repo whose
//! codegen stages nothing, which is every repo that generates no config, must
//! not go red for being what it is. Every OTHER shape is refused, because a
//! staged file that is not installed leaves the committed config saying
//! whatever it said while the build reports success, and that silent pass is
//! the whole reason this step exists.

use std::path::{Path, PathBuf};

/// Where the codegen stages. Fixed rather than an input: the generator writing
/// it and the step installing it have to agree on one path, and a per-repo
/// override is a way for them to stop agreeing.
pub(crate) const STAGED_DIR: &str = ".staged-config";

#[derive(Debug)]
pub(crate) enum Outcome {
    /// No `.staged-config/` at all: this repo's codegen stages nothing.
    NothingStaged,
    /// The file names installed over the repo root.
    Installed(Vec<String>),
}

/// Install every staged file over the root file of the same name, then remove
/// the staging directory — so it is never a place a stale generated file can
/// sit, and a bare re-run reports nothing staged rather than re-installing
/// what a previous run left.
///
/// Every entry is validated before anything is copied: a refusal installs no
/// files and leaves the staging directory in place to be looked at.
pub(crate) fn install(root: &Path) -> Result<Outcome, String> {
    let staged = root.join(STAGED_DIR);

    // symlink_metadata, not metadata: a symlink at this path is refused rather
    // than followed, because the codegen creates a directory there and nothing
    // else is what it wrote.
    match std::fs::symlink_metadata(&staged) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Outcome::NothingStaged),
        Err(e) => return Err(format!("{STAGED_DIR}/ could not be read: {e}")),
        Ok(md) if !md.is_dir() => {
            return Err(format!(
                "{STAGED_DIR} is {} and not a directory. The codegen stages the generated config there, one flat file per file it installs over.",
                if md.file_type().is_symlink() {
                    "a symlink"
                } else {
                    "a file"
                }
            ))
        }
        Ok(_) => {}
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&staged)
        .map_err(|e| format!("{STAGED_DIR}/ could not be read: {e}"))?
        .map(|e| {
            e.map(|e| e.path())
                .map_err(|e| format!("{STAGED_DIR}/ could not be read: {e}"))
        })
        .collect::<Result<_, _>>()?;
    entries.sort();

    if entries.is_empty() {
        return Err(format!(
            "{STAGED_DIR}/ holds no files. The codegen stages the generated config there; it wrote nothing."
        ));
    }

    let mut plan: Vec<(PathBuf, PathBuf, String)> = Vec::new();
    for src in entries {
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{STAGED_DIR}/ holds an unnamed entry"))?;

        let md = std::fs::symlink_metadata(&src)
            .map_err(|e| format!("{STAGED_DIR}/{name} could not be read: {e}"))?;
        if !md.is_file() {
            return Err(format!(
                "{STAGED_DIR}/{name} is not a regular file. Each staged entry is installed over the file of that name at the repo root, so a directory or a symlink here is a generated file that would never be installed."
            ));
        }

        let dest = root.join(&name);
        // metadata, not symlink_metadata: a symlinked root file is written
        // through to its target, which is what installing over it means.
        match std::fs::metadata(&dest) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "{STAGED_DIR}/{name} names no file at the repo root. A staged file is spliced FROM the root file of the same name, so a name matching nothing is a rename or a typo — and installing it would write an untracked new file that `git diff --exit-code` cannot see, leaving the real file stale and the build green."
                ))
            }
            Err(e) => return Err(format!("{name} could not be read: {e}")),
            Ok(md) if !md.is_file() => {
                return Err(format!(
                    "{name} at the repo root is not a file, so {STAGED_DIR}/{name} cannot be installed over it."
                ))
            }
            Ok(_) => {}
        }

        plan.push((src, dest, name));
    }

    let mut installed = Vec::new();
    for (src, dest, name) in plan {
        std::fs::copy(&src, &dest)
            .map_err(|e| format!("{STAGED_DIR}/{name} could not be installed over {name}: {e}"))?;
        installed.push(name);
    }

    std::fs::remove_dir_all(&staged)
        .map_err(|e| format!("{STAGED_DIR}/ could not be removed after installing: {e}"))?;

    Ok(Outcome::Installed(installed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_root() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-staged-config-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A root carrying the two files a config generator splices, and a staging
    /// directory holding the regenerated form of whichever are named.
    fn repo(staged: &[(&str, &str)]) -> PathBuf {
        let root = tmp_root();
        std::fs::write(root.join("foundry.toml"), "committed foundry\n").unwrap();
        std::fs::write(root.join(".env.example"), "committed env\n").unwrap();
        std::fs::create_dir_all(root.join(STAGED_DIR)).unwrap();
        for (name, body) in staged {
            std::fs::write(root.join(STAGED_DIR).join(name), body).unwrap();
        }
        root
    }

    fn read(path: PathBuf) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn an_absent_staging_directory_stages_nothing() {
        let root = tmp_root();
        std::fs::write(root.join("foundry.toml"), "committed foundry\n").unwrap();
        assert!(matches!(install(&root), Ok(Outcome::NothingStaged)));
        assert_eq!(read(root.join("foundry.toml")), "committed foundry\n");
    }

    #[test]
    fn staged_files_are_installed_over_the_root_files_of_the_same_name() {
        let root = repo(&[
            ("foundry.toml", "generated foundry\n"),
            (".env.example", "generated env\n"),
        ]);

        let installed = match install(&root) {
            Ok(Outcome::Installed(names)) => names,
            _ => panic!("expected an install"),
        };

        assert_eq!(installed, vec![".env.example", "foundry.toml"]);
        assert_eq!(read(root.join("foundry.toml")), "generated foundry\n");
        assert_eq!(read(root.join(".env.example")), "generated env\n");
    }

    /// The staging directory is never left behind, so nothing can install a
    /// generated file a later run did not produce.
    #[test]
    fn the_staging_directory_is_removed_and_a_rerun_stages_nothing() {
        let root = repo(&[("foundry.toml", "generated foundry\n")]);

        install(&root).unwrap();
        assert!(!root.join(STAGED_DIR).exists());

        match install(&root) {
            Ok(Outcome::NothingStaged) => {}
            _ => panic!("a re-run must report nothing staged, not re-install"),
        }
        assert_eq!(read(root.join("foundry.toml")), "generated foundry\n");
    }

    /// The failure the step exists for: staging that produced nothing must not
    /// report success, because the committed config is then stale and green.
    #[test]
    fn an_empty_staging_directory_is_refused() {
        let root = repo(&[]);
        let err = install(&root).unwrap_err();
        assert!(err.contains("holds no files"), "{err}");
    }

    #[test]
    fn a_file_at_the_staging_path_is_refused() {
        let root = tmp_root();
        std::fs::write(root.join(STAGED_DIR), "not a directory\n").unwrap();
        let err = install(&root).unwrap_err();
        assert!(err.contains("not a directory"), "{err}");
    }

    #[test]
    fn a_symlink_at_the_staging_path_is_refused() {
        let root = tmp_root();
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join(STAGED_DIR)).unwrap();
        let err = install(&root).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
    }

    /// A staged entry that is not a flat file is a generated file that would
    /// silently never be installed.
    #[test]
    fn a_staged_subdirectory_is_refused() {
        let root = repo(&[("foundry.toml", "generated foundry\n")]);
        std::fs::create_dir_all(root.join(STAGED_DIR).join("nested")).unwrap();
        let err = install(&root).unwrap_err();
        assert!(err.contains("nested"), "{err}");
        assert!(err.contains("not a regular file"), "{err}");
    }

    #[test]
    fn a_staged_symlink_is_refused() {
        let root = repo(&[("foundry.toml", "generated foundry\n")]);
        std::os::unix::fs::symlink(
            root.join("foundry.toml"),
            root.join(STAGED_DIR).join(".env.example"),
        )
        .unwrap();
        let err = install(&root).unwrap_err();
        assert!(err.contains(".env.example"), "{err}");
        assert!(err.contains("not a regular file"), "{err}");
    }

    /// Installing a name that matches nothing writes an untracked file, which
    /// the currency check cannot see — so the real file stays stale and green.
    #[test]
    fn a_staged_file_naming_nothing_at_the_root_is_refused() {
        let root = repo(&[("foundry.tml", "generated foundry\n")]);
        let err = install(&root).unwrap_err();
        assert!(err.contains("foundry.tml"), "{err}");
        assert!(err.contains("names no file at the repo root"), "{err}");
        assert!(!root.join("foundry.tml").exists());
    }

    #[test]
    fn a_directory_at_the_destination_is_refused() {
        let root = repo(&[("generated", "generated\n")]);
        std::fs::create_dir_all(root.join("generated")).unwrap();
        let err = install(&root).unwrap_err();
        assert!(err.contains("not a file"), "{err}");
    }

    /// Validation precedes every copy: one bad entry installs none of them,
    /// rather than leaving the root half generated and half committed.
    #[test]
    fn a_refusal_installs_nothing_and_leaves_the_staging_directory() {
        let root = repo(&[
            (".env.example", "generated env\n"),
            ("foundry.tml", "generated foundry\n"),
        ]);

        install(&root).unwrap_err();

        assert_eq!(read(root.join(".env.example")), "committed env\n");
        assert!(root.join(STAGED_DIR).is_dir());
        assert!(root.join(STAGED_DIR).join(".env.example").is_file());
    }
}
