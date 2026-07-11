//! Append-only enforcement for per-release deploy-pin snapshots.
//!
//! Snapshots under `<root>/<tag>/` (tag = three `_`-separated numbers, e.g.
//! `0_1_4`) are FROZEN once they land on the base branch: downstream consumers
//! pin the address + codehash constants they hold, so a snapshot must never
//! change on `main` or diverge between branches. A release ships new bytecode by
//! ADDING a new `<tag>/` snapshot (bump `[package].version`), never by editing an
//! existing one.
//!
//! This check diffs the working branch against the base and flags any modified or
//! deleted file under an existing tag dir. Adding a new tag dir is allowed;
//! regenerated non-tag files directly under `<root>/` (the current-pin deploy
//! libs) are not snapshots and are ignored.

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

/// True if `path` is a frozen per-tag snapshot FILE: `<root>/<tag>/<something>`.
/// The tag dir itself (no trailing file) and non-tag files directly under `root`
/// are not snapshots.
pub(crate) fn is_snapshot(path: &str, root: &str) -> bool {
    let rest = match path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
        Some(r) => r,
        None => return false,
    };
    match rest.split_once('/') {
        Some((seg, _file)) => is_tag(seg),
        None => false,
    }
}

/// Parse `git diff --name-status` output into offender messages for any modified
/// or deleted snapshot file. Lines look like `M\tpath` or `D\tpath`.
pub(crate) fn parse_offenders(diff: &str, root: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in diff.lines() {
        let mut it = line.splitn(2, '\t');
        let status = it.next().unwrap_or("");
        let path = match it.next() {
            Some(p) => p.trim(),
            None => continue,
        };
        let verb = match status.chars().next() {
            Some('M') => "modified",
            Some('D') => "deleted",
            _ => continue,
        };
        if is_snapshot(path, root) {
            out.push(format!(
                "ERROR: {verb} frozen snapshot {path} — snapshots under {root}/<tag>/ are \
                 append-only. Ship new bytecode as a NEW <tag> (bump [package].version); never \
                 change an existing snapshot (downstream consumers pin these constants)."
            ));
        }
    }
    out
}

/// Diff `base...HEAD` and return the offenders. `base` must already be fetched
/// (in CI, `git fetch origin <base>` first). Returns `Err` if git fails.
pub(crate) fn check(base: &str, root: &str) -> Result<Vec<String>, String> {
    let range = format!("{base}...HEAD");
    // --no-renames so a snapshot rename shows as delete(old)+add(new) and the
    // delete is caught; --diff-filter=MD keeps only modified/deleted entries.
    let out = Command::new("git")
        .args([
            "diff",
            "--no-renames",
            "--name-status",
            "--diff-filter=MD",
            &range,
            "--",
            root,
        ])
        .output()
        .map_err(|e| format!("failed to run git diff: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git diff {range} failed: {} — is the base ref fetched? In CI run `git fetch origin <base>` first.",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse_offenders(&String::from_utf8_lossy(&out.stdout), root))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let root = "src/generated";
        assert!(is_snapshot(
            "src/generated/0_1_4/CloneFactory.pointers.sol",
            root
        ));
        assert!(is_snapshot(
            "src/generated/0_1_10/StoxReceipt.pointers.sol",
            root
        ));
        // the mutable current-pin libs live directly under root, not in a tag dir
        assert!(!is_snapshot("src/generated/LibProdDeployCurrent.sol", root));
        assert!(!is_snapshot(
            "src/generated/CloneFactory.pointers.sol",
            root
        ));
        // a bare tag dir (no file) is not a snapshot file
        assert!(!is_snapshot("src/generated/0_1_4", root));
        // outside root
        assert!(!is_snapshot("src/lib/LibCloneFactoryDeploy.sol", root));
        assert!(!is_snapshot("0_1_4/x.sol", root));
    }

    #[test]
    fn flags_only_modified_or_deleted_snapshots() {
        let root = "src/generated";
        let diff = "A\tsrc/generated/0_1_5/CloneFactory.pointers.sol\n\
                    M\tsrc/generated/0_1_4/CloneFactory.pointers.sol\n\
                    D\tsrc/generated/0_1_3/CloneFactory.pointers.sol\n\
                    M\tsrc/generated/LibProdDeployCurrent.sol\n\
                    M\tsrc/lib/LibCloneFactoryDeploy.sol\n";
        let off = parse_offenders(diff, root);
        assert_eq!(off.len(), 2);
        assert!(off[0]
            .contains("modified frozen snapshot src/generated/0_1_4/CloneFactory.pointers.sol"));
        assert!(off[1]
            .contains("deleted frozen snapshot src/generated/0_1_3/CloneFactory.pointers.sol"));
    }

    #[test]
    fn clean_when_only_adding_a_new_tag() {
        let root = "src/generated";
        let diff = "A\tsrc/generated/0_1_5/CloneFactory.pointers.sol\n\
                    M\tsrc/generated/LibProdDeployCurrent.sol\n";
        assert!(parse_offenders(diff, root).is_empty());
    }
}
