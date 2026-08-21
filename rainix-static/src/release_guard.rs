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
//! unless all of the following hold on the tagged commit.
//!
//!   1. Version identity — `foundry.toml`'s first `version = "…"` (the
//!      `[package].version` the old flow used to write from the tag) equals the
//!      tag's version. The bump happened in a PR the workflow did not control, so
//!      the identity the old flow held "by construction" is now VERIFIED.
//!   2. Snapshot present — `src/generated/<version>/` exists in the tagged commit
//!      (`<version>` with `.` → `_`, matching the frozen-snapshot dir convention).
//!      A missing dir means this version was never cut/frozen.
//!   3. The release follows the record — no LARGER tag is already frozen. A
//!      release is appended to an append-only record, so re-tagging a version the
//!      record has already moved past is not a release. `cutRelease()` refused
//!      this at tag time (`NonMonotonicRelease`) back when the workflow re-cut the
//!      snapshot; the guard is where that invariant lives now.
//!   4. Snapshot is a deterministic regeneration. Two facts together, because a
//!      release freeze is a COPY of the rolling snapshot: rain.deploy's `freeze`
//!      writes `<tag>/` from the bytes it just regenerated into `candidate/` and
//!      reads back off disk, so "the record matches the candidate" is what being
//!      freshly cut MEANS. So:
//!      a. the caller re-runs the repo's generator on the tagged tree (the
//!      NON-freezing entry point — `forge script ./script/Build.sol`, the same
//!      regeneration `rainix-copy-artifacts` currency-checks with) and this
//!      guard requires `git status --porcelain` to be empty. That proves the
//!      rolling `candidate/` snapshot and every file generated from it — the
//!      alias libs, the released-suites libs — are what the tagged source
//!      regenerates to.
//!
//!      b. `<root>/<version>/` is byte-identical to `<root>/candidate/`. That
//!      proves the frozen record is exactly what a freeze run right now would
//!      write. Stale, hand-edited or never-cut all fail here.
//!
//! (4) deliberately does NOT remove `<root>/<version>/` and re-freeze it
//! (rainlanguage/rainix#341). A deploy repo's generated released-suites lib
//! IMPORTS every frozen record — `LibCloneFactoryReleased.sol` imports
//! `../generated/0_1_9/CloneFactory.sol` — so deleting the directory makes the
//! tree uncompilable and the generator that was supposed to re-create it cannot
//! run at all. Comparing the record against the freshly regenerated candidate
//! proves the same thing without ever mutating the tree, and proves it more
//! directly: it checks the bytes rather than re-running the copy that produced
//! them.
//!
//! Deterministic-from-bytecode (address = f(bytecode) under CREATE2), so no chain
//! access is needed here; the on-chain attestation is a separate step.
//!
//! The version/dir checks read the tagged commit via git (`HEAD`), independent of
//! the working-tree regeneration; the determinism checks read the working tree
//! after regeneration, which the clean-tree check has just pinned to `HEAD`. The
//! decisions live in the small pure functions below so they are unit-tested and
//! mutation-covered; `run` only wires git and the filesystem to them.

use crate::fail;
use crate::frozen_snapshots::is_tag;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// The rolling snapshot directory a release is frozen FROM, under the record
/// root — rain.deploy's `LibRainDeploySnapshot.CANDIDATE`. It is regenerated
/// from source on every build, so it is the fresh side of the determinism
/// comparison while `<tag>/` is the frozen side.
pub(crate) const CANDIDATE: &str = "candidate";

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

/// The `version = "…"` value inside `foundry.toml`'s package-metadata table —
/// `[package]` (legacy) or `[external.package]` (current deploy-repo form), the
/// table the release version lives in. Tracks the active TOML table header and
/// returns the version ONLY when inside that table, so a `version =` in some
/// other table earlier in the file (e.g. a tool section) can never be mistaken
/// for the release version. `None` when the package table has no `version =`
/// line (nothing to verify against the tag). A bare-string TOML header check is
/// enough here: foundry.toml is machine-shaped and these two headers sit at
/// column 0; the guard fails closed (no version found) on anything exotic.
pub(crate) fn foundry_version(content: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.lines() {
        let t = line.trim();
        // A table header switches the active section. Only `[package]` /
        // `[external.package]` are the release-version table; any other header
        // (including `[package.metadata.*]` subtables) leaves it.
        if t.starts_with('[') && t.ends_with(']') {
            in_package = t == "[package]" || t == "[external.package]";
            continue;
        }
        if !in_package {
            continue;
        }
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

/// The release-tag directory names sitting DIRECTLY under `root` in the tagged
/// commit, from `git ls-tree --name-only HEAD -- <root>/` (which prints one
/// `<root>/<entry>` line per entry, and nothing at all when the commit carries
/// no such path).
///
/// Filtered through `frozen_snapshots::is_tag`, so what counts as a release here
/// is what the append-only gate counts as one — the rolling `candidate/` dir and
/// any loose file under the root are not releases, and one rule decides that for
/// both.
pub(crate) fn tag_dirs(ls_tree_names: &str, root: &str) -> Vec<String> {
    ls_tree_names
        .lines()
        .filter_map(|line| {
            let entry = line.trim().strip_prefix(root)?.strip_prefix('/')?;
            // `is_tag` also settles depth: every part of a tag is all-digits, so
            // a path separator anywhere in the entry fails it. A file INSIDE a
            // tag dir is therefore not itself a tag dir, and needs no separate
            // check to say so.
            if !is_tag(entry) {
                return None;
            }
            Some(entry.to_string())
        })
        .collect()
}

/// Numeric ordering of two all-digit strings, without parsing them into a
/// fixed-width integer.
///
/// The tag-prefix check upstream admits any run of digits, so a version
/// component can be longer than `u64`/`u128` holds; parsing would overflow or
/// panic inside a publish guard. Once leading zeros are gone, more digits is
/// strictly larger and equal digit counts order lexicographically, which is
/// exact for any length.
fn digits_cmp(a: &str, b: &str) -> Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Release ordering of two `_`-separated tag names: most significant component
/// first, each compared numerically.
fn tag_cmp(a: &str, b: &str) -> Ordering {
    let ap: Vec<&str> = a.split('_').collect();
    let bp: Vec<&str> = b.split('_').collect();
    for (x, y) in ap.iter().zip(bp.iter()) {
        match digits_cmp(x, y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    ap.len().cmp(&bp.len())
}

/// Every tag in `tags` that is a LATER release than `dir` — the releases this
/// one would have to be appended in front of. Empty means `dir` is the newest
/// frozen release, which is the only thing a tag may publish.
pub(crate) fn newer_tags(tags: &[String], dir: &str) -> Vec<String> {
    tags.iter()
        .filter(|t| tag_cmp(t, dir) == Ordering::Greater)
        .cloned()
        .collect()
}

/// Every way the frozen record differs from what a fresh regeneration produced.
/// Empty means the frozen release IS the current regeneration, byte for byte.
///
/// A freeze copies the rolling snapshot verbatim, so an equal record is exactly
/// what freezing right now would write. Both directions are checked (a file only
/// one side holds is a difference), and an EMPTY candidate is flagged rather than
/// silently matching an empty record: two empty directories compare equal, and a
/// release with no record is not a release.
pub(crate) fn record_mismatches(
    frozen: &BTreeMap<String, Vec<u8>>,
    candidate: &BTreeMap<String, Vec<u8>>,
    frozen_dir: &str,
    candidate_dir: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    if candidate.is_empty() {
        out.push(format!(
            "{candidate_dir}/ holds no files — the generator regenerated nothing, so there is \
             nothing to check the frozen release against"
        ));
    }
    if frozen.is_empty() {
        out.push(format!(
            "{frozen_dir}/ holds no files — a release with an empty record is not a release"
        ));
    }
    for (name, bytes) in frozen {
        match candidate.get(name) {
            None => out.push(format!(
                "{frozen_dir}/{name} is in the frozen release but a fresh regeneration does not \
                 produce it"
            )),
            Some(fresh) if fresh != bytes => out.push(format!(
                "{frozen_dir}/{name} differs from the freshly regenerated {candidate_dir}/{name}"
            )),
            Some(_) => {}
        }
    }
    for name in candidate.keys() {
        if !frozen.contains_key(name) {
            out.push(format!(
                "{candidate_dir}/{name} is freshly regenerated but the frozen release does not \
                 hold it"
            ));
        }
    }
    out
}

/// Every file under `dir`, keyed by its path relative to `dir`. `None` when
/// `dir` does not exist. Recursive, so a record that grows a subdirectory is
/// compared rather than ignored.
fn read_record(dir: &Path) -> Option<BTreeMap<String, Vec<u8>>> {
    if !dir.is_dir() {
        return None;
    }
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).unwrap_or_else(|e| {
            fail(&format!(
                "release-guard: cannot read {}: {e}",
                current.display()
            ))
        });
        for entry in entries {
            let path = entry
                .unwrap_or_else(|e| {
                    fail(&format!(
                        "release-guard: cannot read {}: {e}",
                        current.display()
                    ))
                })
                .path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let bytes = std::fs::read(&path).unwrap_or_else(|e| {
                fail(&format!(
                    "release-guard: cannot read {}: {e}",
                    path.display()
                ))
            });
            out.insert(rel, bytes);
        }
    }
    Some(out)
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

/// Print every offender under one `::error::` headline and exit nonzero.
fn fail_with(headline: &str, offenders: &[String]) -> ! {
    eprintln!("::error::{headline}");
    for o in offenders {
        eprintln!("  {o}");
    }
    std::process::exit(1);
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

    // 2/3. The record as the TAGGED COMMIT carries it, read via HEAD rather than
    //      the working tree, so nothing the regeneration wrote can stand in for a
    //      commit that never carried it.
    let dir = version_dir(version);
    let path = format!("{root}/{dir}");
    let frozen_tags = tag_dirs(
        &git_stdout(&["ls-tree", "--name-only", "HEAD", "--", &format!("{root}/")]),
        root,
    );
    if !frozen_tags.contains(&dir) {
        fail(&format!(
            "release-guard: {path}/ is not present in the tagged commit — this version was \
             never cut/frozen; land the snapshot in a PR before tagging"
        ));
    }
    let newer = newer_tags(&frozen_tags, &dir);
    if !newer.is_empty() {
        fail_with(
            &format!(
                "release-guard: {root}/ already holds a release later than {version} — the record \
                 is append-only, so a tag may only publish its newest release. Later releases:"
            ),
            &newer,
        );
    }

    // 4a. The caller re-ran the repo's NON-freezing generator on the tagged tree.
    //     Any working-tree change means the tagged source does not regenerate to
    //     the files the commit carries. This runs BEFORE the record comparison
    //     below, because a clean tree is what makes the on-disk record equal to
    //     the one in the tagged commit.
    let porcelain = git_stdout(&["status", "--porcelain", "--untracked-files=all"]);
    let dirty = dirty_offenders(&porcelain);
    if !dirty.is_empty() {
        fail_with(
            &format!(
                "release-guard: re-running the snapshot generator changed the tree — the tagged \
                 commit's generated files are not what {version}'s source regenerates to. \
                 Regenerate them in a PR and re-tag the merged commit. Offending paths:"
            ),
            &dirty,
        );
    }

    // 4b. The frozen release is byte-identical to the rolling snapshot that was
    //     just regenerated, which is precisely what freezing it now would write.
    let candidate_path = format!("{root}/{CANDIDATE}");
    let frozen = read_record(Path::new(&path)).unwrap_or_else(|| {
        fail(&format!(
            "release-guard: {path}/ is in the tagged commit but not on disk — the working tree \
             must carry the release being published"
        ))
    });
    let candidate = read_record(Path::new(&candidate_path)).unwrap_or_else(|| {
        fail(&format!(
            "release-guard: {candidate_path}/ not found — a release freezes the rolling snapshot, \
             so there is nothing to prove {version} is a fresh regeneration of"
        ))
    });
    let mismatches = record_mismatches(&frozen, &candidate, &path, &candidate_path);
    if !mismatches.is_empty() {
        fail_with(
            &format!(
                "release-guard: the frozen release {path}/ is not what a fresh regeneration \
                 produces — it is stale, hand-edited, or was never cut from this source. Cut it \
                 in a PR and re-tag the merged commit. Differences:"
            ),
            &mismatches,
        );
    }

    println!(
        "release-guard: clean — {foundry} version, {path}/ present and newest, tree regenerates \
         unchanged, frozen release matches {candidate_path}/"
    );
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
            foundry_version("[package]\nversion=\"1.2.3\"").as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            foundry_version("[package]\n  version   =   \"1.2.3\"  ").as_deref(),
            Some("1.2.3")
        );
    }

    #[test]
    fn foundry_version_reads_external_package_table() {
        // Current deploy-repo form: `[external.package]` with a comment block
        // between the header and the version line.
        let toml = "[external.package]\nname = \"rain-extrospection-deploy\"\n\
                    # version of the LAST publish\nversion = \"0.1.0\"\n";
        assert_eq!(foundry_version(toml).as_deref(), Some("0.1.0"));
    }

    #[test]
    fn foundry_version_ignores_version_in_an_earlier_table() {
        // A `version` in a table BEFORE the package table must NOT shadow the
        // real release version: otherwise the guard could match a foreign
        // version to the tag while [package].version differs, and publish a
        // package whose manifest version does not match the tag.
        let toml = "[tool.whatever]\nversion = \"9.9.9\"\n\n\
                    [package]\nname = \"pkg\"\nversion = \"0.1.5\"\n";
        assert_eq!(foundry_version(toml).as_deref(), Some("0.1.5"));
    }

    #[test]
    fn foundry_version_ignores_version_in_a_later_table() {
        // Symmetric: a `version` in a table AFTER [package] must not be read
        // either — only the package table's own version counts.
        let toml = "[package]\nversion = \"0.1.5\"\n\n[other]\nversion = \"9.9.9\"\n";
        assert_eq!(foundry_version(toml).as_deref(), Some("0.1.5"));
    }

    #[test]
    fn foundry_version_ignores_version_prefixed_keys_and_values() {
        // Inside [package]: `versionx` is a different key; a `version` inside a
        // value is not a version line at column start.
        let toml = "[package]\nversionx = \"9.9.9\"\nname = \"version = 1.0.0\"\n\
                    version = \"0.2.0\"\n";
        assert_eq!(foundry_version(toml).as_deref(), Some("0.2.0"));
    }

    #[test]
    fn foundry_version_none_when_no_package_table() {
        // No package table at all, and a version line outside one, are both
        // "no release version" — the guard fails closed.
        assert_eq!(foundry_version("[profile.default]\nsrc = 'src'\n"), None);
        assert_eq!(foundry_version("version = \"1.2.3\"\n"), None);
        assert_eq!(foundry_version(""), None);
    }

    #[test]
    fn tag_dirs_decides_whether_the_release_being_published_is_present() {
        // Invariant 2 reads the same listing invariant 3 does: the release is
        // present in the tagged commit iff its tag dir is one of the entries.
        let tags = tag_dirs(
            "src/generated/0_1_5\nsrc/generated/candidate\n",
            "src/generated",
        );
        assert!(tags.iter().any(|t| t == "0_1_5"));
        assert!(!tags.iter().any(|t| t == "0_1_9"));
        assert!(tag_dirs("", "src/generated").iter().all(|t| t != "0_1_5"));
    }

    #[test]
    fn dirty_offenders_empty_tree_is_clean() {
        assert!(dirty_offenders("").is_empty());
        assert!(dirty_offenders("\n\n   \n").is_empty());
    }

    #[test]
    fn dirty_offenders_lists_every_change() {
        // A modified frozen snapshot and an untracked new file both count.
        let porcelain = " M src/generated/0_1_5/CloneFactory.sol\n\
                          ?? src/generated/0_1_5/Extra.sol\n";
        let off = dirty_offenders(porcelain);
        assert_eq!(off.len(), 2);
        assert!(off[0].contains("0_1_5/CloneFactory.sol"));
        assert!(off[1].contains("0_1_5/Extra.sol"));
    }

    // ---- tag_dirs -----------------------------------------------------

    #[test]
    fn tag_dirs_lists_only_release_tags_directly_under_root() {
        let listing = "src/generated/0_1_9\n\
                       src/generated/0_1_10\n\
                       src/generated/candidate\n\
                       src/generated/README.md\n\
                       src/generated/0_1_9/CloneFactory.sol\n";
        assert_eq!(
            tag_dirs(listing, "src/generated"),
            vec!["0_1_9".to_string(), "0_1_10".to_string()]
        );
    }

    #[test]
    fn tag_dirs_empty_when_nothing_is_generated() {
        // `git ls-tree` on a path the commit does not carry prints nothing.
        assert!(tag_dirs("", "src/generated").is_empty());
        assert!(tag_dirs("   \n\n", "src/generated").is_empty());
    }

    #[test]
    fn tag_dirs_ignores_entries_outside_root() {
        let listing = "src/lib/0_1_9\nother/generated/0_1_9\n";
        assert!(tag_dirs(listing, "src/generated").is_empty());
    }

    // ---- newer_tags ---------------------------------------------------

    #[test]
    fn newer_tags_empty_when_the_release_is_the_newest_frozen() {
        let tags = vec!["0_1_3".to_string(), "0_1_9".to_string()];
        assert!(newer_tags(&tags, "0_1_9").is_empty());
    }

    #[test]
    fn newer_tags_flags_a_release_that_does_not_follow_the_record() {
        // Re-tagging an older version once a newer one is frozen: the old
        // cut-at-tag-time flow refused this via NonMonotonicRelease.
        let tags = vec!["0_1_5".to_string(), "0_1_9".to_string()];
        assert_eq!(newer_tags(&tags, "0_1_5"), vec!["0_1_9".to_string()]);
    }

    #[test]
    fn newer_tags_orders_components_numerically_not_lexically() {
        // "0_1_10" sorts BEFORE "0_1_9" as a string; as a release it follows it.
        let tags = vec!["0_1_9".to_string(), "0_1_10".to_string()];
        assert!(newer_tags(&tags, "0_1_10").is_empty());
        assert_eq!(newer_tags(&tags, "0_1_9"), vec!["0_1_10".to_string()]);
    }

    #[test]
    fn newer_tags_compares_the_most_significant_component_first() {
        let tags = vec!["1_0_0".to_string(), "0_99_99".to_string()];
        assert_eq!(newer_tags(&tags, "0_99_99"), vec!["1_0_0".to_string()]);
        assert!(newer_tags(&tags, "1_0_0").is_empty());
    }

    #[test]
    fn newer_tags_handles_components_too_long_for_a_fixed_width_integer() {
        // The tag-prefix regex admits any digit run, so a component can exceed
        // u64/u128. Comparison must stay exact rather than overflow or panic.
        let big = format!("0_1_{}", "9".repeat(40));
        let bigger = format!("0_1_1{}", "0".repeat(40));
        let tags = vec![big.clone(), bigger.clone()];
        assert_eq!(newer_tags(&tags, &big), vec![bigger.clone()]);
        assert!(newer_tags(&tags, &bigger).is_empty());
    }

    #[test]
    fn newer_tags_ignores_leading_zeros_when_comparing() {
        let tags = vec!["0_1_9".to_string()];
        assert!(newer_tags(&tags, "0_01_009").is_empty());
        // And the other way round: a padded spelling of the SAME release is not
        // a later release. Comparing digit runs without stripping the padding
        // would call the longer string the bigger number and flag it.
        assert!(newer_tags(&["0_1_09".to_string()], "0_1_9").is_empty());
    }

    // ---- record_mismatches --------------------------------------------

    fn record(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn record_mismatches_clean_when_the_frozen_record_is_the_fresh_regeneration() {
        let frozen = record(&[("CloneFactory.sol", "pins"), ("Other.sol", "more")]);
        let candidate = record(&[("CloneFactory.sol", "pins"), ("Other.sol", "more")]);
        assert!(record_mismatches(
            &frozen,
            &candidate,
            "src/generated/0_1_9",
            "src/generated/candidate"
        )
        .is_empty());
    }

    #[test]
    fn record_mismatches_flags_a_frozen_file_whose_content_drifted() {
        let frozen = record(&[("CloneFactory.sol", "old pins")]);
        let candidate = record(&[("CloneFactory.sol", "new pins")]);
        let off = record_mismatches(
            &frozen,
            &candidate,
            "src/generated/0_1_9",
            "src/generated/candidate",
        );
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("src/generated/0_1_9/CloneFactory.sol"));
        assert!(off[0].contains("differs"));
    }

    #[test]
    fn record_mismatches_flags_a_file_only_the_frozen_record_holds() {
        let frozen = record(&[("A.sol", "x"), ("Ghost.sol", "y")]);
        let candidate = record(&[("A.sol", "x")]);
        let off = record_mismatches(
            &frozen,
            &candidate,
            "src/generated/0_1_9",
            "src/generated/candidate",
        );
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("Ghost.sol"));
    }

    #[test]
    fn record_mismatches_flags_a_file_only_a_fresh_regeneration_produces() {
        let frozen = record(&[("A.sol", "x")]);
        let candidate = record(&[("A.sol", "x"), ("New.sol", "y")]);
        let off = record_mismatches(
            &frozen,
            &candidate,
            "src/generated/0_1_9",
            "src/generated/candidate",
        );
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("New.sol"));
    }

    #[test]
    fn record_mismatches_flags_an_empty_candidate_rather_than_matching_an_empty_record() {
        // Two empty dirs are "equal"; a release with no record is not a release.
        let empty = record(&[]);
        let off = record_mismatches(
            &empty,
            &empty,
            "src/generated/0_1_9",
            "src/generated/candidate",
        );
        assert_eq!(off.len(), 2);
        assert!(off
            .iter()
            .any(|o| o.contains("src/generated/candidate/ holds no files")));
        assert!(off
            .iter()
            .any(|o| o.contains("src/generated/0_1_9/ holds no files")));
    }

    #[test]
    fn record_mismatches_compares_bytes_not_lengths() {
        let frozen = record(&[("A.sol", "abc")]);
        let candidate = record(&[("A.sol", "abd")]);
        assert_eq!(record_mismatches(&frozen, &candidate, "f", "c").len(), 1);
    }
}
