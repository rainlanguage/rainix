//! `release-guard` — fail-closed publish guard for `rainix-tag-release`.
//!
//! Under the push-free deploy-release flow (rainlanguage/rainix#338) the
//! workflow no longer cuts the snapshot itself: a reviewed PR lands the frozen
//! `src/generated/<version>/` snapshot and the `[package].version` bump on main,
//! and a human `sol-v<version>` tag on that merged commit triggers publish. So
//! nothing in the workflow guarantees the tagged commit actually carries a
//! freshly-cut snapshot for this version — a mistag, a stale PR, or a hand-edited
//! snapshot would otherwise publish silently. This guard closes that hole: run
//! immediately before `forge soldeer push`, it fails loud (publishing NOTHING)
//! unless all three hold on the tagged commit:
//!
//!   1. Version identity — `foundry.toml`'s first `version = "…"` (the
//!      `[package].version` the old flow used to write from the tag) equals the
//!      tag's version. The bump happened in a PR the workflow did not control, so
//!      the identity the old flow held "by construction" is now VERIFIED.
//!   2. Snapshot present — `src/generated/<version>/` exists in the tagged commit
//!      (`<version>` with `.` → `_`, matching the frozen-snapshot dir convention).
//!      A missing dir means this version was never cut/frozen.
//!   3. Snapshot is a deterministic regeneration — the caller re-runs the repo's
//!      `snapshot-generate-cmd` on the tagged tree (having first removed the
//!      target dir, because a generator may refuse to overwrite a frozen dir),
//!      then this guard requires `git status --porcelain` to be empty: any change
//!      means the committed snapshot is stale, hand-edited, or was never
//!      regenerated for this commit. Deterministic-from-bytecode, so no chain
//!      access is needed; the on-chain attestation is a separate step.
//!
//! The version/dir checks read the tagged commit via git (`HEAD`), independent of
//! the working-tree regeneration; the determinism check reads the working tree
//! after regeneration. The decisions live in the small pure functions below so
//! they are unit-tested and mutation-covered; `run` only wires git and the
//! filesystem to them.

use crate::fail;
use std::process::Command;

/// True iff `v` is a strict `MAJOR.MINOR.PATCH` where each part is one or more
/// ASCII digits. Anything else (pre-release suffixes, extra components, empty
/// parts, non-digits) is rejected — the guard must not derive a snapshot dir
/// name from a version the frozen-snapshot append-only gate would ignore.
pub(crate) fn is_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The frozen-snapshot directory name for a version: `.` → `_`, matching the
/// `is_tag` convention in `frozen_snapshots` (e.g. `0.1.5` → `0_1_5`).
pub(crate) fn version_dir(version: &str) -> String {
    version.replace('.', "_")
}

/// The value of `foundry.toml`'s first `version = "…"` line — the
/// `[package].version` the old tag-release flow wrote from the tag (deploy
/// repos put `[package]` first, so the first `version =` is it, the same line
/// that flow's `sed` targeted and `cut-release.sh`'s `grep -m1` reads). Matches
/// an optional-whitespace `version =` at the start of a line and returns the
/// text inside the first double-quoted string on it. `None` when no such line
/// exists (no version to verify against the tag).
pub(crate) fn foundry_version(content: &str) -> Option<String> {
    for line in content.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("version") else {
            continue;
        };
        // `version` must be followed by `=` (optionally after whitespace), not
        // be a prefix of another key like `versionx`.
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        // First double-quoted string on the line is the value.
        let after_open = rest.find('"')? + 1;
        let close = rest[after_open..].find('"')? + after_open;
        return Some(rest[after_open..close].to_string());
    }
    None
}

/// True iff `git ls-tree HEAD -- <dir>` listed anything, i.e. the directory
/// exists in the tagged commit. Empty output means it does not.
pub(crate) fn dir_present(ls_tree_output: &str) -> bool {
    !ls_tree_output.trim().is_empty()
}

/// Working-tree changes reported by `git status --porcelain` as offender lines.
/// Any non-blank line is a change (modified, added, deleted, or untracked); an
/// empty result means the tree is clean. `.git/info/exclude` already hides the
/// devShell's generated `.pre-commit-config.yaml`, so it never appears here.
pub(crate) fn dirty_offenders(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// Run a git command and return its stdout; fail loud (with stderr) on spawn
/// error or nonzero exit.
fn git_stdout(args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .output()
        .unwrap_or_else(|e| fail(&format!("git {}: failed to spawn: {e}", args.join(" "))));
    if !out.status.success() {
        fail(&format!(
            "git {}: {} ({})",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim(),
            out.status
        ));
    }
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The publish guard. `version` is the tag's version, `root` the snapshot root
/// (default `src/generated`), `foundry` the manifest path (default
/// `foundry.toml`). Fails loud and exits nonzero on the first violated
/// invariant; prints `release-guard: clean …` and returns when all hold.
pub(crate) fn run(version: &str, root: &str, foundry: &str) {
    // A non-semver version has no valid frozen-snapshot dir name; refuse before
    // deriving one.
    if !is_semver(version) {
        fail(&format!(
            "release-guard: tag version {version:?} is not MAJOR.MINOR.PATCH"
        ));
    }

    // 1. Version identity: foundry.toml's [package].version == the tag version.
    let content = std::fs::read_to_string(foundry)
        .unwrap_or_else(|e| fail(&format!("release-guard: cannot read {foundry}: {e}")));
    match foundry_version(&content) {
        None => fail(&format!(
            "release-guard: {foundry} has no `version = \"…\"` line to check against tag \
             version {version}"
        )),
        Some(v) if v != version => fail(&format!(
            "release-guard: {foundry} version {v:?} does not match tag version {version:?} — \
             the release commit's version bump and the pushed tag must agree"
        )),
        Some(_) => {}
    }

    // 2. Snapshot present in the tagged commit (read via HEAD, not the
    //    regenerated working tree, so a dir the generator just recreated cannot
    //    hide a commit that never carried it).
    let dir = version_dir(version);
    let path = format!("{root}/{dir}");
    let listed = git_stdout(&["ls-tree", "HEAD", "--", &path]);
    if !dir_present(&listed) {
        fail(&format!(
            "release-guard: {path}/ is not present in the tagged commit — this version was \
             never cut/frozen; land the snapshot in a PR before tagging"
        ));
    }

    // 3. Deterministic regeneration: the caller already re-ran the generator on
    //    the tagged tree; any working-tree change means the committed snapshot is
    //    not what a fresh regeneration produces (stale, hand-edited, or never cut
    //    for this commit).
    let porcelain = git_stdout(&["status", "--porcelain", "--untracked-files=all"]);
    let offenders = dirty_offenders(&porcelain);
    if !offenders.is_empty() {
        eprintln!(
            "::error::release-guard: re-running the snapshot generator changed the tree — the \
             committed snapshot for {version} is stale, hand-edited, or was never regenerated for \
             this commit. Regenerate it in a PR and re-tag the merged commit. Offending paths:"
        );
        for o in offenders {
            eprintln!("  {o}");
        }
        std::process::exit(1);
    }

    println!("release-guard: clean — {foundry} version, {path}/ present, snapshot deterministic");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_accepts_major_minor_patch() {
        assert!(is_semver("0.1.5"));
        assert!(is_semver("12.0.255"));
        assert!(is_semver("0.0.0"));
    }

    #[test]
    fn semver_rejects_non_x_y_z() {
        assert!(!is_semver("0.1")); // two parts
        assert!(!is_semver("0.1.5.6")); // four parts
        assert!(!is_semver("0.1.")); // empty trailing
        assert!(!is_semver(".1.5")); // empty leading
        assert!(!is_semver("0.1.5-rc1")); // pre-release suffix
        assert!(!is_semver("v0.1.5")); // non-digit
        assert!(!is_semver("0.1.x")); // non-digit
        assert!(!is_semver("")); // empty
    }

    #[test]
    fn version_dir_dots_to_underscores() {
        assert_eq!(version_dir("0.1.5"), "0_1_5");
        assert_eq!(version_dir("12.0.255"), "12_0_255");
    }

    #[test]
    fn foundry_version_reads_first_version_line() {
        let toml = "[package]\nname = \"rain-factory-deploy\"\nversion = \"0.1.5\"\n\n\
                    [profile.default]\nsrc = 'src'\n";
        assert_eq!(foundry_version(toml).as_deref(), Some("0.1.5"));
    }

    #[test]
    fn foundry_version_tolerates_whitespace_variants() {
        assert_eq!(
            foundry_version("version=\"1.2.3\"").as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            foundry_version("  version   =   \"1.2.3\"  ").as_deref(),
            Some("1.2.3")
        );
    }

    #[test]
    fn foundry_version_takes_the_first_when_several() {
        // Only the first `version =` (the [package] line, first in deploy
        // repos) is the release version; later ones must not shadow it.
        let toml = "[package]\nversion = \"0.1.5\"\n\n[other]\nversion = \"9.9.9\"\n";
        assert_eq!(foundry_version(toml).as_deref(), Some("0.1.5"));
    }

    #[test]
    fn foundry_version_ignores_version_prefixed_keys_and_values() {
        // `versionx` is a different key; a `version` inside a value is not a
        // version line at column start.
        let toml = "versionx = \"9.9.9\"\nname = \"version = 1.0.0\"\nversion = \"0.2.0\"\n";
        assert_eq!(foundry_version(toml).as_deref(), Some("0.2.0"));
    }

    #[test]
    fn foundry_version_none_when_absent() {
        assert_eq!(foundry_version("[profile.default]\nsrc = 'src'\n"), None);
        assert_eq!(foundry_version(""), None);
    }

    #[test]
    fn dir_present_true_only_on_nonempty_listing() {
        assert!(dir_present("040000 tree abc123\tsrc/generated/0_1_5\n"));
        assert!(!dir_present(""));
        assert!(!dir_present("   \n  \n"));
    }

    #[test]
    fn dirty_offenders_empty_tree_is_clean() {
        assert!(dirty_offenders("").is_empty());
        assert!(dirty_offenders("\n\n   \n").is_empty());
    }

    #[test]
    fn dirty_offenders_lists_every_change() {
        // A modified frozen snapshot and an untracked new file both count.
        let porcelain = " M src/generated/0_1_5/CloneFactory.pointers.sol\n\
                          ?? src/generated/0_1_5/Extra.pointers.sol\n";
        let off = dirty_offenders(porcelain);
        assert_eq!(off.len(), 2);
        assert!(off[0].contains("0_1_5/CloneFactory.pointers.sol"));
        assert!(off[1].contains("0_1_5/Extra.pointers.sol"));
    }
}
