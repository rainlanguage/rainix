//! Append-only enforcement for per-release deploy-pin snapshots.
//!
//! Snapshots under `<root>/<tag>/` (tag = three `_`-separated numbers, e.g.
//! `0_1_4`) are FROZEN once they land on the base branch: downstream consumers
//! pin the address + codehash constants they hold, so a snapshot's bytes must
//! never change on `main` or diverge between branches. A release ships new
//! bytecode by ADDING a new `<tag>/` snapshot (bump `[package].version`), never
//! by editing an existing one.
//!
//! What is frozen is the bytes, not the filename. Every snapshot on the base
//! must survive at HEAD as the verbatim PREFIX of a regular file in the same
//! tag dir: either at its own path, or at a path the branch added there (a
//! rename). Anything after the prefix is an addition — a later top-level
//! declaration cannot change the value of a constant before it — so appending
//! is allowed; editing, truncating or moving a snapshot to another tag dir is
//! not. A snapshot that is not a regular file on the base has no frozen bytes,
//! so any change to it is flagged. Each added file carries at most one renamed
//! snapshot, and deletions are paired with added files by a maximum one-to-one
//! matching. Adding a new tag dir is allowed; regenerated non-tag files
//! directly under `<root>/` (the current-pin deploy libs) are not snapshots and
//! are ignored.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// True if `seg` is a release-tag dir name: three `_`-separated non-empty numeric
/// parts, e.g. `0_1_4` or `12_0_255`.
pub(crate) fn is_tag(seg: &str) -> bool {
    let parts: Vec<&str> = seg.split('_').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The tag dir name of a frozen per-tag snapshot FILE `<root>/<tag>/<something>`,
/// or `None` if `path` is not one. The tag dir itself (no trailing file) and
/// non-tag files directly under `root` are not snapshots.
pub(crate) fn snapshot_tag<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    let (seg, _file) = rest.split_once('/')?;
    is_tag(seg).then_some(seg)
}

/// True if `path` is a frozen per-tag snapshot FILE: `<root>/<tag>/<something>`.
pub(crate) fn is_snapshot(path: &str, root: &str) -> bool {
    snapshot_tag(path, root).is_some()
}

/// One `git diff --no-renames --raw` entry under the root.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) status: char,
    /// The file's mode at the merge base (`000000` for an addition).
    pub(crate) base_mode: String,
    /// The file's mode at HEAD (`000000` for a deletion).
    pub(crate) head_mode: String,
    pub(crate) path: String,
}

/// True if `mode` is a regular file's. A symlink, submodule or absent file
/// holds no snapshot bytes, whatever `git show` prints for it.
fn is_regular(mode: &str) -> bool {
    matches!(mode, "100644" | "100755")
}

/// Parse `git diff --no-renames --raw -z` output: NUL-terminated
/// `:base_mode head_mode base_sha head_sha status`, `path` fields in pairs.
/// Paths are taken verbatim; with `-z` git neither quotes nor escapes them.
pub(crate) fn parse_changes(diff: &str) -> Vec<Change> {
    let mut fields = diff.split('\0');
    let mut changes = Vec::new();
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let meta: Vec<&str> = meta.split(' ').collect();
        if let [base_mode, head_mode, _, _, status] = meta[..] {
            if let Some(status) = status.chars().next() {
                changes.push(Change {
                    status,
                    base_mode: base_mode.trim_start_matches(':').to_string(),
                    head_mode: head_mode.to_string(),
                    path: path.to_string(),
                });
            }
        }
    }
    changes
}

fn offender(verb: &str, path: &str, root: &str) -> String {
    format!(
        "ERROR: {verb} frozen snapshot {path} — snapshots under {root}/<tag>/ are \
         append-only. Ship new bytecode as a NEW <tag> (bump [package].version); never \
         change an existing snapshot (downstream consumers pin these constants). A rename \
         within its tag dir, or an addition after it, passes only if the frozen bytes \
         survive verbatim as the start of the file."
    )
}

/// Try to give deleted snapshot `d` a carrier among `carries[d]`, re-homing the
/// snapshot already carried by a candidate when it has another carrier (an
/// augmenting path). `carrier[a]` is the deleted snapshot added file `a` carries.
fn augment(
    d: usize,
    carries: &[Vec<usize>],
    carrier: &mut [Option<usize>],
    seen: &mut [bool],
) -> bool {
    for &a in &carries[d] {
        if seen[a] {
            continue;
        }
        seen[a] = true;
        let free = match carrier[a] {
            None => true,
            Some(other) => augment(other, carries, carrier, seen),
        };
        if free {
            carrier[a] = Some(d);
            return true;
        }
    }
    false
}

/// The offenders among `changes`. `base` holds the merge-base bytes of every
/// changed snapshot that is a regular file there, `head` the HEAD bytes of
/// every changed snapshot that is a regular file at HEAD. A path missing from
/// `base` has no frozen bytes to keep, and one missing from `head` cannot keep
/// any, so a symlink whose target text matches a snapshot never passes as it.
///
/// Deleted snapshots are paired with added files by a maximum one-to-one
/// matching, so the order of `changes` cannot strand a deletion that has a
/// valid carrier.
pub(crate) fn find_offenders(
    changes: &[Change],
    root: &str,
    base: &BTreeMap<String, Vec<u8>>,
    head: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    let keeps = |file: &Change, frozen: &str| match (head.get(&file.path), base.get(frozen)) {
        (Some(h), Some(b)) => h.starts_with(b),
        _ => false,
    };
    let of = |status: char| -> Vec<&Change> {
        changes
            .iter()
            .filter(|c| c.status == status && is_snapshot(&c.path, root))
            .collect()
    };
    let deleted = of('D');
    let added = of('A');
    let carries: Vec<Vec<usize>> = deleted
        .iter()
        .map(|d| {
            (0..added.len())
                .filter(|&a| {
                    snapshot_tag(&added[a].path, root) == snapshot_tag(&d.path, root)
                        && keeps(added[a], &d.path)
                })
                .collect()
        })
        .collect();
    let mut carrier: Vec<Option<usize>> = vec![None; added.len()];
    for d in 0..deleted.len() {
        augment(d, &carries, &mut carrier, &mut vec![false; added.len()]);
    }
    let renamed: BTreeSet<&str> = carrier
        .iter()
        .flatten()
        .map(|&d| deleted[d].path.as_str())
        .collect();
    let mut out = Vec::new();
    for change in changes {
        let path = change.path.as_str();
        if !is_snapshot(path, root) {
            continue;
        }
        match change.status {
            'M' | 'T' if !keeps(change, path) => out.push(offender("modified", path, root)),
            'D' if !renamed.contains(path) => out.push(offender("deleted", path, root)),
            _ => {}
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {} — is the base ref fetched? In CI run `git fetch origin <base>` first.",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

/// Diff `base...HEAD` in the repository at `dir` and return the offenders.
/// `base` must already be fetched (in CI, `git fetch origin <base>` first).
/// Returns `Err` if git fails.
pub(crate) fn check(dir: &Path, base: &str, root: &str) -> Result<Vec<String>, String> {
    let range = format!("{base}...HEAD");
    // --no-renames so a rename shows as delete(old)+add(new) and the pairing is
    // decided here, by content, rather than by git's similarity heuristic.
    let diff = git(
        dir,
        &[
            "diff",
            "--no-renames",
            "--raw",
            "-z",
            "--diff-filter=AMDT",
            &range,
            "--",
            root,
        ],
    )?;
    let changes = parse_changes(&String::from_utf8_lossy(&diff));
    let merge_base = String::from_utf8_lossy(&git(dir, &["merge-base", base, "HEAD"])?)
        .trim()
        .to_string();
    let mut base_content = BTreeMap::new();
    let mut head_content = BTreeMap::new();
    for change in changes.iter().filter(|c| is_snapshot(&c.path, root)) {
        if is_regular(&change.base_mode) {
            let bytes = git(dir, &["show", &format!("{merge_base}:{}", change.path)])?;
            base_content.insert(change.path.clone(), bytes);
        }
        if is_regular(&change.head_mode) {
            let bytes = git(dir, &["show", &format!("HEAD:{}", change.path)])?;
            head_content.insert(change.path.clone(), bytes);
        }
    }
    Ok(find_offenders(&changes, root, &base_content, &head_content))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "src/generated";

    /// `changes` from a readable `status\tpath` per line fixture, sent through
    /// the NUL-delimited raw parser. Every file present on either side is a
    /// regular file.
    fn changes(diff: &str) -> Vec<Change> {
        let z: String = diff
            .lines()
            .map(|line| {
                let (status, path) = line.split_once('\t').unwrap();
                let base_mode = if status == "A" { "000000" } else { "100644" };
                let head_mode = if status == "D" { "000000" } else { "100644" };
                format!(":{base_mode} {head_mode} 0000000 0000000 {status}\0{path}\0")
            })
            .collect();
        parse_changes(&z)
    }

    fn content(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(p, c)| (p.to_string(), c.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn tag_names() {
        assert!(is_tag("0_1_4"));
        assert!(is_tag("12_0_255"));
        assert!(!is_tag("0_1")); // two parts
        assert!(!is_tag("0_1_4_5")); // four parts
        assert!(!is_tag("0_1_")); // empty trailing part
        assert!(!is_tag("_1_4")); // empty leading part
        assert!(!is_tag("v0_1_4")); // non-digit
        assert!(!is_tag("LibProdDeployCurrent.sol"));
    }

    #[test]
    fn snapshot_paths() {
        assert!(is_snapshot("src/generated/0_1_4/CloneFactory.sol", ROOT));
        assert!(is_snapshot("src/generated/0_1_10/StoxReceipt.sol", ROOT));
        assert_eq!(
            snapshot_tag("src/generated/0_1_10/StoxReceipt.sol", ROOT),
            Some("0_1_10")
        );
        // the mutable current-pin libs live directly under root, not in a tag dir
        assert!(!is_snapshot("src/generated/LibProdDeployCurrent.sol", ROOT));
        assert!(!is_snapshot("src/generated/CloneFactory.sol", ROOT));
        // the rolling candidate dir is not a release tag
        assert!(!is_snapshot(
            "src/generated/candidate/CloneFactory.sol",
            ROOT
        ));
        // a bare tag dir (no file) is not a snapshot file
        assert!(!is_snapshot("src/generated/0_1_4", ROOT));
        // outside root
        assert!(!is_snapshot("src/lib/LibCloneFactoryDeploy.sol", ROOT));
        assert!(!is_snapshot("0_1_4/x.sol", ROOT));
    }

    #[test]
    fn flags_edited_or_deleted_snapshots() {
        let c = changes(
            "A\tsrc/generated/0_1_5/CloneFactory.sol\n\
             M\tsrc/generated/0_1_4/CloneFactory.sol\n\
             D\tsrc/generated/0_1_3/CloneFactory.sol\n\
             M\tsrc/generated/LibProdDeployCurrent.sol\n\
             M\tsrc/lib/LibCloneFactoryDeploy.sol\n",
        );
        let base = content(&[
            (
                "src/generated/0_1_4/CloneFactory.sol",
                "bytes32 constant H = 0x01;\n",
            ),
            (
                "src/generated/0_1_3/CloneFactory.sol",
                "bytes32 constant H = 0x03;\n",
            ),
        ]);
        let head = content(&[
            (
                "src/generated/0_1_5/CloneFactory.sol",
                "bytes32 constant H = 0x05;\n",
            ),
            (
                "src/generated/0_1_4/CloneFactory.sol",
                "bytes32 constant H = 0x02;\n",
            ),
        ]);
        let off = find_offenders(&c, ROOT, &base, &head);
        assert_eq!(off.len(), 2);
        assert!(off[0].contains("modified frozen snapshot src/generated/0_1_4/CloneFactory.sol"));
        assert!(off[1].contains("deleted frozen snapshot src/generated/0_1_3/CloneFactory.sol"));
    }

    #[test]
    fn clean_when_only_adding_a_new_tag() {
        let c = changes(
            "A\tsrc/generated/0_1_5/CloneFactory.sol\n\
             M\tsrc/generated/LibProdDeployCurrent.sol\n",
        );
        let head = content(&[("src/generated/0_1_5/CloneFactory.sol", "x")]);
        assert!(find_offenders(&c, ROOT, &BTreeMap::new(), &head).is_empty());
    }

    #[test]
    fn a_rename_keeping_the_bytes_passes() {
        let c = changes(
            "A\tsrc/generated/0_1_4/CloneFactory.sol\n\
             D\tsrc/generated/0_1_4/CloneFactory.pointers.sol\n",
        );
        let frozen = "bytes32 constant H = 0x01;\n";
        let base = content(&[("src/generated/0_1_4/CloneFactory.pointers.sol", frozen)]);
        let head = content(&[("src/generated/0_1_4/CloneFactory.sol", frozen)]);
        assert!(find_offenders(&c, ROOT, &base, &head).is_empty());
    }

    #[test]
    fn appending_after_the_frozen_bytes_passes_in_place_or_renamed() {
        let frozen = "bytes32 constant H = 0x01;\n";
        let appended = "bytes32 constant H = 0x01;\nbytes constant DEPENDENCIES = hex\"\";\n";
        let c = changes(
            "M\tsrc/generated/0_1_4/A.sol\n\
             D\tsrc/generated/0_1_4/B.pointers.sol\n\
             A\tsrc/generated/0_1_4/B.sol\n",
        );
        let base = content(&[
            ("src/generated/0_1_4/A.sol", frozen),
            ("src/generated/0_1_4/B.pointers.sol", frozen),
        ]);
        let head = content(&[
            ("src/generated/0_1_4/A.sol", appended),
            ("src/generated/0_1_4/B.sol", appended),
        ]);
        assert!(find_offenders(&c, ROOT, &base, &head).is_empty());
    }

    #[test]
    fn a_rename_that_edits_the_frozen_bytes_is_a_deletion() {
        let c = changes(
            "D\tsrc/generated/0_1_4/CloneFactory.pointers.sol\n\
             A\tsrc/generated/0_1_4/CloneFactory.sol\n",
        );
        let base = content(&[(
            "src/generated/0_1_4/CloneFactory.pointers.sol",
            "bytes32 constant H = 0x01;\n",
        )]);
        // Inserting before the frozen bytes, and changing them, both fail.
        for edited in [
            "// x\nbytes32 constant H = 0x01;\n",
            "bytes32 constant H = 0x02;\n",
        ] {
            let head = content(&[("src/generated/0_1_4/CloneFactory.sol", edited)]);
            let off = find_offenders(&c, ROOT, &base, &head);
            assert_eq!(off.len(), 1);
            assert!(off[0]
                .contains("deleted frozen snapshot src/generated/0_1_4/CloneFactory.pointers.sol"));
        }
    }

    #[test]
    fn a_truncation_in_place_is_a_modification() {
        let c = changes("M\tsrc/generated/0_1_4/A.sol\n");
        let base = content(&[("src/generated/0_1_4/A.sol", "a\nb\n")]);
        let head = content(&[("src/generated/0_1_4/A.sol", "a\n")]);
        let off = find_offenders(&c, ROOT, &base, &head);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("modified frozen snapshot src/generated/0_1_4/A.sol"));
    }

    #[test]
    fn a_rename_into_another_tag_dir_is_a_deletion() {
        let frozen = "bytes32 constant H = 0x01;\n";
        let c = changes(
            "D\tsrc/generated/0_1_4/A.sol\n\
             A\tsrc/generated/0_1_5/A.sol\n",
        );
        let base = content(&[("src/generated/0_1_4/A.sol", frozen)]);
        let head = content(&[("src/generated/0_1_5/A.sol", frozen)]);
        let off = find_offenders(&c, ROOT, &base, &head);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("deleted frozen snapshot src/generated/0_1_4/A.sol"));
    }

    #[test]
    fn a_deleted_snapshot_cannot_pair_with_a_file_the_base_already_had() {
        // B already exists on the base, so it carries its own frozen bytes and
        // cannot also stand in for a deleted A, even with identical content.
        let frozen = "bytes32 constant H = 0x01;\n";
        let c = changes(
            "D\tsrc/generated/0_1_4/A.sol\n\
             M\tsrc/generated/0_1_4/B.sol\n",
        );
        let base = content(&[
            ("src/generated/0_1_4/A.sol", frozen),
            ("src/generated/0_1_4/B.sol", frozen),
        ]);
        let head = content(&[(
            "src/generated/0_1_4/B.sol",
            "bytes32 constant H = 0x01;\n// x\n",
        )]);
        let off = find_offenders(&c, ROOT, &base, &head);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("deleted frozen snapshot src/generated/0_1_4/A.sol"));
    }

    #[test]
    fn one_added_file_carries_at_most_one_renamed_snapshot() {
        // Two identical frozen files cannot both be "renamed" into one file.
        let frozen = "bytes32 constant H = 0x01;\n";
        let c = changes(
            "D\tsrc/generated/0_1_4/A.sol\n\
             D\tsrc/generated/0_1_4/B.sol\n\
             A\tsrc/generated/0_1_4/C.sol\n",
        );
        let base = content(&[
            ("src/generated/0_1_4/A.sol", frozen),
            ("src/generated/0_1_4/B.sol", frozen),
        ]);
        let head = content(&[("src/generated/0_1_4/C.sol", frozen)]);
        let off = find_offenders(&c, ROOT, &base, &head);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("deleted frozen snapshot src/generated/0_1_4/B.sol"));
    }

    #[test]
    fn renames_pair_one_to_one_regardless_of_order() {
        // A shorter frozen snapshot is a prefix of a longer one, so every file
        // that can carry the longer can also carry the shorter. Pairing the
        // shorter first with the only file the longer fits must not strand the
        // longer: A -> Azz and AB -> ABx is a valid assignment.
        let base = content(&[
            ("src/generated/0_1_4/A.sol", "A"),
            ("src/generated/0_1_4/AB.sol", "AB"),
        ]);
        let head = content(&[
            ("src/generated/0_1_4/ABx.sol", "ABx"),
            ("src/generated/0_1_4/Azz.sol", "Azz"),
        ]);
        for diff in [
            "D\tsrc/generated/0_1_4/A.sol\nD\tsrc/generated/0_1_4/AB.sol\nA\tsrc/generated/0_1_4/ABx.sol\nA\tsrc/generated/0_1_4/Azz.sol\n",
            "A\tsrc/generated/0_1_4/Azz.sol\nA\tsrc/generated/0_1_4/ABx.sol\nD\tsrc/generated/0_1_4/AB.sol\nD\tsrc/generated/0_1_4/A.sol\n",
        ] {
            assert_eq!(find_offenders(&changes(diff), ROOT, &base, &head), Vec::<String>::new());
        }
    }

    /// A throwaway git repository with `main` holding the base snapshots and
    /// HEAD on a branch off it.
    struct Repo(std::path::PathBuf);

    impl Repo {
        fn new(name: &str) -> Repo {
            let dir = std::env::temp_dir().join(format!(
                "rainix-frozen-snapshots-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let repo = Repo(dir);
            repo.git(&["init", "-q", "--initial-branch=main"]);
            repo.git(&["config", "user.email", "test@example.invalid"]);
            repo.git(&["config", "user.name", "test"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo.git(&["config", "core.symlinks", "true"]);
            repo
        }

        fn git(&self, args: &[&str]) {
            let out = Command::new("git")
                .current_dir(&self.0)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        fn write(&self, path: &str, bytes: &str) {
            let p = self.0.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let _ = std::fs::remove_file(&p);
            std::fs::write(p, bytes).unwrap();
        }

        fn symlink(&self, path: &str, target: &str) {
            let p = self.0.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let _ = std::fs::remove_file(&p);
            std::os::unix::fs::symlink(target, p).unwrap();
        }

        fn commit_all(&self) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", "x"]);
        }

        fn branch(&self) {
            self.git(&["checkout", "-q", "-b", "head"]);
        }

        fn offenders(&self) -> Vec<String> {
            check(&self.0, "main", ROOT).unwrap()
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_nul_delimited_paths_verbatim() {
        assert_eq!(
            parse_changes(
                ":100644 100755 9bda8c3 2000c07 M\0src/generated/0_1_4/a\tb\nc .sol \0\
                 :100644 000000 9bda8c3 0000000 D\0src/generated/0_1_4/\"q\".sol\0\
                 :100644 120000 9bda8c3 62d8fe9 T\0src/generated/0_1_4/s.sol\0"
            ),
            vec![
                Change {
                    status: 'M',
                    base_mode: "100644".to_string(),
                    head_mode: "100755".to_string(),
                    path: "src/generated/0_1_4/a\tb\nc .sol ".to_string(),
                },
                Change {
                    status: 'D',
                    base_mode: "100644".to_string(),
                    head_mode: "000000".to_string(),
                    path: "src/generated/0_1_4/\"q\".sol".to_string(),
                },
                Change {
                    status: 'T',
                    base_mode: "100644".to_string(),
                    head_mode: "120000".to_string(),
                    path: "src/generated/0_1_4/s.sol".to_string(),
                },
            ]
        );
    }

    #[test]
    fn edits_and_deletions_of_snapshots_with_quoted_names_are_flagged() {
        // git quotes a path holding a tab, newline, quote or backslash unless
        // the diff is NUL-delimited; a quoted path is not under the root.
        let repo = Repo::new("quoted");
        for name in [
            "tab\ted.sol",
            "line\ned.sol",
            "q\"uote.sol",
            "back\\slash.sol",
        ] {
            repo.write(&format!("{ROOT}/0_1_4/edit-{name}"), "frozen\n");
            repo.write(&format!("{ROOT}/0_1_4/del-{name}"), "frozen\n");
        }
        repo.commit_all();
        repo.branch();
        for name in [
            "tab\ted.sol",
            "line\ned.sol",
            "q\"uote.sol",
            "back\\slash.sol",
        ] {
            repo.write(&format!("{ROOT}/0_1_4/edit-{name}"), "edited\n");
            std::fs::remove_file(repo.0.join(format!("{ROOT}/0_1_4/del-{name}"))).unwrap();
        }
        repo.commit_all();
        let off = repo.offenders();
        assert_eq!(off.len(), 8, "{off:#?}");
        for name in [
            "tab\ted.sol",
            "line\ned.sol",
            "q\"uote.sol",
            "back\\slash.sol",
        ] {
            let edited = format!("modified frozen snapshot {ROOT}/0_1_4/edit-{name} ");
            let deleted = format!("deleted frozen snapshot {ROOT}/0_1_4/del-{name} ");
            assert!(off.iter().any(|o| o.contains(&edited)), "{edited}");
            assert!(off.iter().any(|o| o.contains(&deleted)), "{deleted}");
        }
    }

    #[test]
    fn a_snapshot_swapped_for_a_symlink_is_a_modification() {
        // `git show` prints a symlink's target text, so a target equal to the
        // frozen bytes would otherwise read as an unchanged snapshot.
        let repo = Repo::new("symlink-swap");
        repo.write(&format!("{ROOT}/0_1_4/A.sol"), "frozen\n");
        repo.commit_all();
        repo.branch();
        repo.symlink(&format!("{ROOT}/0_1_4/A.sol"), "frozen\n");
        repo.commit_all();
        let off = repo.offenders();
        assert_eq!(off.len(), 1, "{off:#?}");
        assert!(off[0].contains(&format!("modified frozen snapshot {ROOT}/0_1_4/A.sol ")));
    }

    #[test]
    fn a_symlink_cannot_carry_a_renamed_snapshot() {
        let repo = Repo::new("symlink-rename");
        repo.write(&format!("{ROOT}/0_1_4/A.pointers.sol"), "frozen\n");
        repo.commit_all();
        repo.branch();
        std::fs::remove_file(repo.0.join(format!("{ROOT}/0_1_4/A.pointers.sol"))).unwrap();
        repo.symlink(&format!("{ROOT}/0_1_4/A.sol"), "frozen\n");
        repo.commit_all();
        let off = repo.offenders();
        assert_eq!(off.len(), 1, "{off:#?}");
        assert!(off[0].contains(&format!(
            "deleted frozen snapshot {ROOT}/0_1_4/A.pointers.sol "
        )));
    }

    #[test]
    fn a_symlink_snapshot_swapped_for_its_target_text_is_a_modification() {
        // A symlink on the base has no frozen bytes; its target text is not
        // what a consumer compiles, so a regular file holding it keeps nothing.
        let repo = Repo::new("symlink-base-swap");
        repo.symlink(&format!("{ROOT}/0_1_4/A.sol"), "frozen\n");
        repo.commit_all();
        repo.branch();
        repo.write(&format!("{ROOT}/0_1_4/A.sol"), "frozen\n");
        repo.commit_all();
        let off = repo.offenders();
        assert_eq!(off.len(), 1, "{off:#?}");
        assert!(off[0].contains(&format!("modified frozen snapshot {ROOT}/0_1_4/A.sol ")));
    }

    #[test]
    fn a_symlink_snapshot_cannot_be_renamed_into_its_target_text() {
        let repo = Repo::new("symlink-base-rename");
        repo.symlink(&format!("{ROOT}/0_1_4/A.pointers.sol"), "frozen\n");
        repo.commit_all();
        repo.branch();
        std::fs::remove_file(repo.0.join(format!("{ROOT}/0_1_4/A.pointers.sol"))).unwrap();
        repo.write(&format!("{ROOT}/0_1_4/A.sol"), "frozen\n");
        repo.commit_all();
        let off = repo.offenders();
        assert_eq!(off.len(), 1, "{off:#?}");
        assert!(off[0].contains(&format!(
            "deleted frozen snapshot {ROOT}/0_1_4/A.pointers.sol "
        )));
    }

    #[test]
    fn an_executable_snapshot_keeping_its_bytes_passes() {
        use std::os::unix::fs::PermissionsExt;
        let repo = Repo::new("exec");
        repo.write(&format!("{ROOT}/0_1_4/A.sol"), "frozen\n");
        repo.write(&format!("{ROOT}/0_1_4/B.pointers.sol"), "frozen\n");
        repo.commit_all();
        repo.branch();
        let a = repo.0.join(format!("{ROOT}/0_1_4/A.sol"));
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_file(repo.0.join(format!("{ROOT}/0_1_4/B.pointers.sol"))).unwrap();
        repo.write(&format!("{ROOT}/0_1_4/B.sol"), "frozen\n");
        let b = repo.0.join(format!("{ROOT}/0_1_4/B.sol"));
        std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o755)).unwrap();
        repo.commit_all();
        assert_eq!(repo.offenders(), Vec::<String>::new());
    }

    #[test]
    fn a_clean_branch_in_a_real_repo_passes() {
        let repo = Repo::new("clean");
        repo.write(&format!("{ROOT}/0_1_4/A.pointers.sol"), "frozen\n");
        repo.commit_all();
        repo.branch();
        std::fs::remove_file(repo.0.join(format!("{ROOT}/0_1_4/A.pointers.sol"))).unwrap();
        repo.write(&format!("{ROOT}/0_1_4/A.sol"), "frozen\nappended\n");
        repo.write(&format!("{ROOT}/0_1_5/A.sol"), "new\n");
        repo.commit_all();
        assert_eq!(repo.offenders(), Vec::<String>::new());
    }
}
