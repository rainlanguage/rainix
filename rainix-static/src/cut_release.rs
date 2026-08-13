//! `cut-release` — freeze a deploy repo's rolling `candidate` snapshot as the
//! numbered release snapshot the pushed tag names.
//!
//! Model: `<root>/candidate/` is the rolling snapshot of what the current source
//! compiles to (rewritten in full by every pointer-generation run, and aliased by
//! the consumer-facing pin lib, so it is what a release actually publishes). A
//! numbered snapshot (`0_1_5/`, …) is a FROZEN copy of `candidate` taken at the
//! instant a tag releases it — it never changes again, which
//! `snapshots-append-only` enforces.
//!
//! The ordering is the whole point of this living in a tool. Regenerating AFTER
//! the copy is silently wrong whenever the committed `candidate` has drifted from
//! source: the copy freezes the stale bytes into a dir the append-only gate then
//! protects forever, while the regeneration moves `candidate` on to the real
//! ones — so the release publishes one address and permanently records another.
//! Nothing downstream catches it; a self-consistency test checks the
//! *regenerated* candidate against source, and no test compares a numbered dir to
//! `candidate`. So the consumer supplies only the pointer-generation command and
//! this module owns the sequence: **generate -> `forge fmt` -> freeze -> verify**.
//! There is no input that runs after the copy, so the inverted order cannot be
//! expressed.

use crate::frozen_snapshots::is_tag;
use crate::soldeer_gate::read_local_version;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pointer-generation command when the caller supplies none. Every deploy repo on
/// the rolling-candidate model generates its pins with exactly this invocation,
/// so it is the convention rather than a guess; a repo that names its script
/// differently passes `--generate-cmd`.
pub(crate) const DEFAULT_GENERATE_CMD: &str = "forge script ./script/BuildPointers.sol";

/// The rolling snapshot dir. Never a version number: the append-only gate's tag
/// filter is all-numeric, so `candidate` is invisible to it and free to roll.
const CANDIDATE: &str = "candidate";

/// What this release will freeze, resolved from the repo BEFORE anything runs.
struct Plan {
    /// `<root>/candidate` — the rolling snapshot to freeze.
    candidate: PathBuf,
    /// `<root>/<tag>` — the numbered dir to freeze it into.
    frozen: PathBuf,
    /// The version with dots as underscores, e.g. `0.1.5` -> `0_1_5`.
    tag: String,
}

/// Resolve the release from `foundry.toml` and check every precondition, before
/// the generator runs, so a misconfigured release fails without side effects.
fn plan(repo: &Path, root: &str) -> Result<Plan, String> {
    let version = read_local_version(repo).ok_or_else(|| {
        format!(
            "cut-release: {} has no [package].version — the release version is read from there \
             (rainix-tag-release writes it from the pushed tag)",
            repo.join("foundry.toml").display()
        )
    })?;
    let tag = version.replace('.', "_");
    // Strict X.Y.Z, tested against the append-only gate's OWN predicate rather
    // than a restatement of it: a version like `0.1.7-rc1` yields `0_1_7-rc1`,
    // which that gate ignores forever — an orphan snapshot nothing protects.
    // Refuse rather than cut one.
    if !is_tag(&tag) {
        return Err(format!(
            "cut-release: version {version:?} is not strict X.Y.Z — refusing to cut \
             {root}/{tag}, a snapshot dir the append-only gate would ignore forever"
        ));
    }
    let candidate = repo.join(root).join(CANDIDATE);
    if !candidate.is_dir() {
        return Err(format!(
            "cut-release: {} is missing — this repo is not on the rolling-candidate model, \
             so there is nothing to freeze",
            candidate.display()
        ));
    }
    let frozen = repo.join(root).join(&tag);
    if frozen.exists() {
        return Err(format!(
            "cut-release: {} already exists — refusing to overwrite a frozen release snapshot \
             (snapshots are append-only; release a new version instead)",
            frozen.display()
        ));
    }
    Ok(Plan {
        candidate,
        frozen,
        tag,
    })
}

/// Every file under `root`, keyed by its path relative to `root`. Directories
/// carry no content of their own, so an empty one is not represented — git does
/// not track one either.
fn read_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| format!("cut-release: read dir {}: {e}", dir.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| format!("cut-release: read dir {}: {e}", dir.display()))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("cut-release: stat {}: {e}", path.display()))?;
            if kind.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| {
                        format!(
                            "cut-release: {} is not under {}: {e}",
                            path.display(),
                            root.display()
                        )
                    })?
                    .to_path_buf();
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("cut-release: read {}: {e}", path.display()))?;
                out.insert(rel, bytes);
            }
        }
    }
    Ok(out)
}

/// Write a tree at `root`, which must NOT already exist. `create_dir` rather than
/// `create_dir_all` is the append-only guard at the point of writing: a frozen
/// snapshot is never written over, whatever raced or generated it.
fn write_tree(root: &Path, tree: &BTreeMap<PathBuf, Vec<u8>>) -> Result<(), String> {
    std::fs::create_dir(root)
        .map_err(|e| format!("cut-release: create {}: {e}", root.display()))?;
    for (rel, bytes) in tree {
        let dst = root.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cut-release: create {}: {e}", parent.display()))?;
        }
        std::fs::write(&dst, bytes)
            .map_err(|e| format!("cut-release: write {}: {e}", dst.display()))?;
    }
    Ok(())
}

/// Regenerate, then freeze. `generate` is the caller's regeneration step
/// (pointer-generation command + `forge fmt` in production, a fake in tests); it
/// is taken as an argument so the sequence itself is testable without forge.
///
/// The order below is the invariant this tool exists to hold: the numbered dir
/// must record what the release actually publishes, and what it publishes is
/// `candidate` — the pin lib aliases it — so the copy has to come from a
/// `candidate` already known to match the current source.
fn cut(
    repo: &Path,
    root: &str,
    generate: &mut dyn FnMut() -> Result<(), String>,
) -> Result<String, String> {
    let plan = plan(repo, root)?;

    // 1. REGENERATE (and format) FIRST — never after the copy.
    generate()?;

    // 2. The generator regenerates `candidate` and nothing else. One that froze a
    //    numbered dir itself (a leftover consumer cut-release script) is the
    //    inverted order sneaking back in through the command input; refuse it.
    if plan.frozen.exists() {
        return Err(format!(
            "cut-release: the pointer-generation command created {} itself — it must only \
             regenerate {}; freezing the numbered snapshot is this tool's job, and doing it \
             before regeneration is what records an address the release does not publish",
            plan.frozen.display(),
            plan.candidate.display()
        ));
    }
    if !plan.candidate.is_dir() {
        return Err(format!(
            "cut-release: the pointer-generation command left {} missing — nothing to freeze",
            plan.candidate.display()
        ));
    }
    let tree = read_tree(&plan.candidate)?;
    if tree.is_empty() {
        return Err(format!(
            "cut-release: {} holds no files after regeneration — nothing to freeze",
            plan.candidate.display()
        ));
    }

    // 3. FREEZE, from the just-regenerated candidate.
    write_tree(&plan.frozen, &tree)?;

    // 4. The point of the ordering above, asserted rather than assumed: the
    //    frozen record and the published pin are the same bytes. Both sides are
    //    re-read from disk (the equivalent of `diff -r`), so this also catches the
    //    copy having disturbed `candidate` itself.
    if read_tree(&plan.frozen)? != read_tree(&plan.candidate)? {
        return Err(format!(
            "cut-release: {} does not match {} after the copy",
            plan.frozen.display(),
            plan.candidate.display()
        ));
    }
    Ok(plan.tag)
}

/// Run a command through bash, failing loud on a nonzero exit. `-euo pipefail` so
/// a consumer command written as `a; b` cannot hide a's failure behind b's
/// success.
fn sh(cmd: &str) -> Result<(), String> {
    let status = Command::new("bash")
        .args(["-euo", "pipefail", "-c", cmd])
        .status()
        .map_err(|e| format!("cut-release: failed to spawn {cmd:?}: {e}"))?;
    if !status.success() {
        return Err(format!("cut-release: {cmd:?} exited with {status}"));
    }
    Ok(())
}

/// Cut the release in the current directory: run `generate_cmd`, format, then
/// freeze `<root>/candidate` into `<root>/<tag>`. Returns the frozen tag.
pub(crate) fn run(root: &str, generate_cmd: &str) -> Result<String, String> {
    cut(Path::new("."), root, &mut || {
        sh(generate_cmd)?;
        // Formats BEFORE the copy, so the frozen dir is byte-identical to
        // `candidate` rather than to its pre-format form.
        sh("forge fmt")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-cut-release-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const ROOT: &str = "src/generated";

    /// A deploy repo on the candidate model: a version, and a rolling candidate
    /// snapshot holding one pointer file.
    fn repo(version: &str) -> PathBuf {
        let d = tmp_dir();
        std::fs::write(
            d.join("foundry.toml"),
            format!("[package]\nname = \"x-deploy\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
        std::fs::create_dir_all(d.join(ROOT).join(CANDIDATE)).unwrap();
        std::fs::write(
            d.join(ROOT).join(CANDIDATE).join("X.pointers.sol"),
            b"address constant DEPLOYED_ADDRESS = address(0xAAA);\n",
        )
        .unwrap();
        d
    }

    fn read(p: &Path) -> String {
        String::from_utf8(std::fs::read(p).unwrap()).unwrap()
    }

    /// A generator that records that it ran and does nothing else.
    fn counting(count: &mut usize) -> impl FnMut() -> Result<(), String> + '_ {
        move || {
            *count += 1;
            Ok(())
        }
    }

    #[test]
    fn freezes_candidate_into_the_numbered_dir() {
        let d = repo("0.1.5");
        let mut ran = 0;
        let tag = cut(&d, ROOT, &mut counting(&mut ran)).unwrap();
        assert_eq!(tag, "0_1_5");
        assert_eq!(ran, 1);
        // The frozen dir is a byte-identical copy, and the gate recognises it.
        assert!(is_tag(&tag));
        assert_eq!(
            read(&d.join(ROOT).join("0_1_5/X.pointers.sol")),
            read(&d.join(ROOT).join("candidate/X.pointers.sol")),
        );
        // …and candidate itself is untouched by the freeze.
        assert!(d.join(ROOT).join(CANDIDATE).is_dir());
    }

    #[test]
    fn freezes_nested_files_too() {
        let d = repo("1.2.3");
        std::fs::create_dir_all(d.join(ROOT).join("candidate/sub")).unwrap();
        std::fs::write(d.join(ROOT).join("candidate/sub/Y.pointers.sol"), b"y\n").unwrap();
        cut(&d, ROOT, &mut counting(&mut 0)).unwrap();
        assert_eq!(read(&d.join(ROOT).join("1_2_3/sub/Y.pointers.sol")), "y\n");
    }

    /// The ordering this tool exists to enforce: the frozen dir records what the
    /// generator produced, not what was committed before it ran.
    #[test]
    fn regenerates_before_freezing() {
        let d = repo("0.2.0");
        let candidate = d.join(ROOT).join(CANDIDATE);
        let mut generate = || {
            // A drifted candidate being brought back in line with source.
            std::fs::write(
                candidate.join("X.pointers.sol"),
                b"address constant DEPLOYED_ADDRESS = address(0xBBB);\n",
            )
            .unwrap();
            std::fs::write(candidate.join("New.pointers.sol"), b"new\n").unwrap();
            Ok(())
        };
        cut(&d, ROOT, &mut generate).unwrap();
        let frozen = d.join(ROOT).join("0_2_0");
        // Freezing first would have recorded 0xAAA — the address the release does
        // not publish — and would not hold the new file at all.
        assert_eq!(
            read(&frozen.join("X.pointers.sol")),
            "address constant DEPLOYED_ADDRESS = address(0xBBB);\n"
        );
        assert_eq!(read(&frozen.join("New.pointers.sol")), "new\n");
    }

    /// The same ordering seen from the generator's side: nothing is frozen yet
    /// while it runs.
    #[test]
    fn nothing_is_frozen_while_the_generator_runs() {
        let d = repo("0.3.1");
        let frozen = d.join(ROOT).join("0_3_1");
        let mut existed_during_generate = true;
        {
            let frozen = frozen.clone();
            let mut generate = || {
                existed_during_generate = frozen.exists();
                Ok(())
            };
            cut(&d, ROOT, &mut generate).unwrap();
        }
        assert!(!existed_during_generate);
        assert!(frozen.is_dir());
    }

    #[test]
    fn refuses_when_the_generator_freezes_the_numbered_dir_itself() {
        let d = repo("0.4.0");
        let root_dir = d.join(ROOT);
        let mut generate = || {
            std::fs::create_dir_all(root_dir.join("0_4_0")).unwrap();
            std::fs::write(root_dir.join("0_4_0/X.pointers.sol"), b"stale\n").unwrap();
            Ok(())
        };
        let err = cut(&d, ROOT, &mut generate).unwrap_err();
        assert!(err.contains("created"), "{err}");
        // The tool never wrote over it, and never blessed it as the release.
        assert_eq!(read(&d.join(ROOT).join("0_4_0/X.pointers.sol")), "stale\n");
    }

    #[test]
    fn a_failing_generator_freezes_nothing() {
        let d = repo("0.5.0");
        let mut generate = || Err("boom".to_string());
        assert_eq!(cut(&d, ROOT, &mut generate).unwrap_err(), "boom");
        assert!(!d.join(ROOT).join("0_5_0").exists());
    }

    #[test]
    fn rejects_versions_the_append_only_gate_would_ignore() {
        // Each of these would freeze an orphan dir no gate protects.
        for version in ["0.1.7-rc1", "0.1", "1.2.3.4", "0.1.5+build", "v0.1.5", ""] {
            let d = repo(version);
            let mut ran = 0;
            let err = cut(&d, ROOT, &mut counting(&mut ran)).unwrap_err();
            assert!(err.contains("strict X.Y.Z"), "{version}: {err}");
            // The guard runs before any work: nothing regenerated, nothing frozen.
            assert_eq!(ran, 0, "{version}");
            let dirs: Vec<_> = std::fs::read_dir(d.join(ROOT))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(dirs, vec![std::ffi::OsString::from(CANDIDATE)], "{version}");
        }
    }

    #[test]
    fn accepts_every_strict_version() {
        for (version, tag) in [
            ("0.1.5", "0_1_5"),
            ("1.0.0", "1_0_0"),
            ("12.0.255", "12_0_255"),
            ("0.1.10", "0_1_10"),
        ] {
            let d = repo(version);
            assert_eq!(cut(&d, ROOT, &mut counting(&mut 0)).unwrap(), tag);
            assert!(d.join(ROOT).join(tag).is_dir());
        }
    }

    #[test]
    fn refuses_without_a_candidate() {
        let d = repo("0.1.5");
        std::fs::remove_dir_all(d.join(ROOT).join(CANDIDATE)).unwrap();
        let mut ran = 0;
        let err = cut(&d, ROOT, &mut counting(&mut ran)).unwrap_err();
        assert!(err.contains("rolling-candidate model"), "{err}");
        assert_eq!(ran, 0);
        assert!(!d.join(ROOT).join("0_1_5").exists());
    }

    #[test]
    fn refuses_an_empty_candidate() {
        let d = repo("0.1.5");
        std::fs::remove_file(d.join(ROOT).join("candidate/X.pointers.sol")).unwrap();
        let err = cut(&d, ROOT, &mut counting(&mut 0)).unwrap_err();
        assert!(err.contains("nothing to freeze"), "{err}");
        assert!(!d.join(ROOT).join("0_1_5").exists());
    }

    #[test]
    fn refuses_to_overwrite_a_frozen_snapshot() {
        let d = repo("0.1.5");
        std::fs::create_dir_all(d.join(ROOT).join("0_1_5")).unwrap();
        std::fs::write(d.join(ROOT).join("0_1_5/X.pointers.sol"), b"frozen\n").unwrap();
        let mut ran = 0;
        let err = cut(&d, ROOT, &mut counting(&mut ran)).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(ran, 0);
        // Untouched: the frozen bytes downstream consumers pin are still there.
        assert_eq!(read(&d.join(ROOT).join("0_1_5/X.pointers.sol")), "frozen\n");
    }

    #[test]
    fn refuses_without_a_package_version() {
        let d = repo("0.1.5");
        std::fs::write(d.join("foundry.toml"), "[package]\nname = \"x-deploy\"\n").unwrap();
        let err = cut(&d, ROOT, &mut counting(&mut 0)).unwrap_err();
        assert!(err.contains("[package].version"), "{err}");
    }

    #[test]
    fn write_tree_refuses_an_existing_dir() {
        let d = tmp_dir();
        let dst = d.join("0_1_5");
        std::fs::create_dir(&dst).unwrap();
        let mut tree = BTreeMap::new();
        tree.insert(PathBuf::from("X.sol"), b"x".to_vec());
        assert!(write_tree(&dst, &tree).is_err());
        assert!(!dst.join("X.sol").exists());
    }

    #[test]
    fn read_tree_is_relative_and_recursive() {
        let d = tmp_dir();
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("top"), b"1").unwrap();
        std::fs::write(d.join("a/mid"), b"2").unwrap();
        std::fs::write(d.join("a/b/deep"), b"3").unwrap();
        let tree = read_tree(&d).unwrap();
        assert_eq!(tree.len(), 3);
        assert_eq!(tree[Path::new("top")], b"1");
        assert_eq!(tree[Path::new("a/mid")], b"2");
        assert_eq!(tree[Path::new("a/b/deep")], b"3");
    }

    /// The consumer's generate command is arbitrary shell, so a failure anywhere
    /// in it has to abort the cut — a release that freezes what a half-failed
    /// generator left behind is the same silent-corruption class this tool exists
    /// to close.
    #[test]
    fn a_command_that_fails_anywhere_fails_the_cut() {
        assert!(sh("true").is_ok());
        assert!(sh("false").is_err());
        // -e: an early failure is not hidden by a later success.
        assert!(sh("false; true").is_err());
        // -o pipefail: nor by a successful tail of a pipe.
        assert!(sh("false | true").is_err());
        // -u: an unset variable (a typo'd path) is an error, not an empty string.
        assert!(sh("echo \"${RAINIX_CUT_RELEASE_UNSET_PROBE}\"").is_err());
    }
}
