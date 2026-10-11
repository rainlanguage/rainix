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
//! must survive at HEAD as the verbatim PREFIX of a file in the same tag dir:
//! either at its own path, or at a path the branch added there (a rename).
//! Anything after the prefix is an addition — a later top-level declaration
//! cannot change the value of a constant before it — so appending is allowed;
//! editing, truncating or moving a snapshot to another tag dir is not. Each
//! added file carries at most one renamed snapshot. Adding a new tag dir is
//! allowed; regenerated non-tag files directly under `<root>/` (the current-pin
//! deploy libs) are not snapshots and are ignored.

use std::collections::{BTreeMap, BTreeSet};
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

/// One `git diff --no-renames --name-status` entry under the root.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) status: char,
    pub(crate) path: String,
}

/// Parse `git diff --name-status` output (`A\tpath`, `M\tpath`, `D\tpath`).
pub(crate) fn parse_changes(diff: &str) -> Vec<Change> {
    diff.lines()
        .filter_map(|line| {
            let (status, path) = line.split_once('\t')?;
            Some(Change {
                status: status.chars().next()?,
                path: path.trim().to_string(),
            })
        })
        .collect()
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

/// The offenders among `changes`. `base` holds the merge-base content of every
/// modified or deleted snapshot, `head` the HEAD content of every modified or
/// added snapshot.
pub(crate) fn find_offenders(
    changes: &[Change],
    root: &str,
    base: &BTreeMap<String, Vec<u8>>,
    head: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    let starts_with = |file: &str, frozen: &str| match (head.get(file), base.get(frozen)) {
        (Some(h), Some(b)) => h.starts_with(b),
        _ => false,
    };
    let mut out = Vec::new();
    let mut claimed: BTreeSet<&str> = BTreeSet::new();
    for change in changes {
        let path = change.path.as_str();
        let Some(tag) = snapshot_tag(path, root) else {
            continue;
        };
        match change.status {
            'M' => {
                if !starts_with(path, path) {
                    out.push(offender("modified", path, root));
                }
            }
            'D' => {
                let renamed_to = changes.iter().find(|added| {
                    added.status == 'A'
                        && snapshot_tag(&added.path, root) == Some(tag)
                        && !claimed.contains(added.path.as_str())
                        && starts_with(&added.path, path)
                });
                match renamed_to {
                    Some(added) => {
                        claimed.insert(added.path.as_str());
                    }
                    None => out.push(offender("deleted", path, root)),
                }
            }
            _ => {}
        }
    }
    out
}

fn git(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
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

/// Diff `base...HEAD` and return the offenders. `base` must already be fetched
/// (in CI, `git fetch origin <base>` first). Returns `Err` if git fails.
pub(crate) fn check(base: &str, root: &str) -> Result<Vec<String>, String> {
    let range = format!("{base}...HEAD");
    // --no-renames so a rename shows as delete(old)+add(new) and the pairing is
    // decided here, by content, rather than by git's similarity heuristic.
    let diff = git(&[
        "diff",
        "--no-renames",
        "--name-status",
        "--diff-filter=AMD",
        &range,
        "--",
        root,
    ])?;
    let changes = parse_changes(&String::from_utf8_lossy(&diff));
    let merge_base = String::from_utf8_lossy(&git(&["merge-base", base, "HEAD"])?)
        .trim()
        .to_string();
    let mut base_content = BTreeMap::new();
    let mut head_content = BTreeMap::new();
    for change in changes.iter().filter(|c| is_snapshot(&c.path, root)) {
        if matches!(change.status, 'M' | 'D') {
            let bytes = git(&["show", &format!("{merge_base}:{}", change.path)])?;
            base_content.insert(change.path.clone(), bytes);
        }
        if matches!(change.status, 'M' | 'A') {
            let bytes = git(&["show", &format!("HEAD:{}", change.path)])?;
            head_content.insert(change.path.clone(), bytes);
        }
    }
    Ok(find_offenders(&changes, root, &base_content, &head_content))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "src/generated";

    fn changes(diff: &str) -> Vec<Change> {
        parse_changes(diff)
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
}
