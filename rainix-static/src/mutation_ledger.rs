//! `mutation-ledger` — shape + ancestry gate for `audit/mutation-test-scans.json`.
//!
//! The adversarial-mutation-test skill closes every run by hand-appending a
//! record to a committed `audit/mutation-test-scans.json`. Two automated
//! consumers then trust that file: the audit skill's Pass-2 gate reads it as
//! evidence of when the repo was last mutation-tested, and rain-org-health's
//! `roh-scan` ranks its entries by `timestamp` to date each repo's newest run.
//! Both trust the `commit` it records, and NEITHER validates it — the scanner
//! degrades a malformed ledger to `unknown`, and the skill gate checks little
//! more than that the file is an array. So a record whose `commit` is a short
//! prefix, a typo, or a SHA that was rebased away merges green and surfaces
//! months later as a refused or wrong-based audit (rainlanguage/rain.lib.hash#62).
//!
//! Two halves, both here because they gate one file:
//!
//! - **Shape.** A non-empty JSON array of objects, each carrying the fields the
//!   skill specifies, with a 40-character lowercase-hex `commit` and a strict
//!   `YYYY-MM-DDTHH:MM:SSZ` `timestamp`.
//! - **Ancestry.** Every SHA a record names must be an ancestor of `HEAD`. A
//!   recorded commit that is not reachable mis-bases every later comparison:
//!   "what changed since the last mutation run" is computed against a tree that
//!   is not in this history.
//!
//! `testsAfterCommit` is validated WHEN PRESENT and never required. The skill
//! calls it a must-have today, but 12 of the 18 records already committed
//! across rainlanguage name no such tree, and neither does the presence of one
//! follow the skill version (rain.sol.codegen wrote it at 0.30.0, rain.string
//! did not at 0.33.0). Requiring it would fail those repos on a value that
//! cannot be recovered for a run that is over: the only way to go green would
//! be to invent a SHA, which is the fabricated evidence this gate exists to
//! catch. `skillVersion` is optional for the same reason — rain.extrospection's
//! record carries none.
//!
//! A repo with no ledger passes: this validates a record, it never requires one.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the skill writes the ledger. One path, org-wide — the audit skill and
/// `roh-scan` both hardcode it, so a repo that moves the file is invisible to
/// them regardless of what this gate would say about it.
pub(crate) const LEDGER_PATH: &str = "audit/mutation-test-scans.json";

/// The only tool that writes this ledger. The audit skill's gate reads this
/// field to decide whether the record is one of its own.
pub(crate) const TOOL: &str = "adversarial-mutation-test";

/// A git SHA a record names, tagged with where it came from so a failure names
/// the record and the field rather than just the value.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Sha {
    pub index: usize,
    pub field: &'static str,
    pub value: String,
}

impl Sha {
    fn at(&self) -> String {
        format!("{LEDGER_PATH}[{}].{}", self.index, self.field)
    }
}

/// What `HEAD` knows about a recorded SHA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ancestry {
    /// Reachable from HEAD — the record bases on this history.
    Ancestor,
    /// A real commit here, but not on this branch (rebased away, or a PR head
    /// recorded before a squash-merge rewrote it).
    NotAncestor,
    /// No such object in this repo at all: a typo, or a SHA from elsewhere.
    Unknown,
}

/// True for a full git object name: 40 characters, lowercase hex. Uppercase is
/// rejected deliberately — git prints lowercase, so an uppercase SHA was typed
/// or transformed by hand, and the value is only trustworthy when it was
/// copied from git.
pub(crate) fn is_sha40(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Days in a Gregorian month, so `2026-02-31` cannot pass as a date.
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// True for exactly `YYYY-MM-DDTHH:MM:SSZ`, a real date and a real time of day.
///
/// One width and one zone, because `roh-scan` picks the newest run by comparing
/// these as STRINGS — a local offset, a fractional second or a dropped `:SS`
/// all sort wrongly against the rest and silently under-report recency.
pub(crate) fn is_utc_timestamp(s: &str) -> bool {
    const MASK: &[u8; 20] = b"dddd-dd-ddTdd:dd:ddZ";
    let b = s.as_bytes();
    if b.len() != MASK.len() {
        return false;
    }
    for (i, m) in MASK.iter().enumerate() {
        if *m == b'd' {
            if !b[i].is_ascii_digit() {
                return false;
            }
        } else if b[i] != *m {
            return false;
        }
    }
    // Every field is ASCII digits by the mask above, so these parses cannot fail.
    let n = |a: usize, z: usize| s[a..z].parse::<u32>().unwrap_or(u32::MAX);
    let (year, month, day) = (n(0, 4), n(5, 7), n(8, 10));
    let (hour, minute, second) = (n(11, 13), n(14, 16), n(17, 19));
    (1..=12).contains(&month)
        && day >= 1
        && day <= days_in_month(year, month)
        && hour <= 23
        && minute <= 59
        && second <= 59
}

fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Validate one record's SHA-valued field. `required` distinguishes `commit`
/// (a record with no scanned tree names nothing) from `testsAfterCommit` (see
/// the module doc — absent is legal, malformed is not).
fn check_sha_field(
    entry: &serde_json::Value,
    index: usize,
    field: &'static str,
    required: bool,
    offenders: &mut Vec<String>,
    shas: &mut Vec<Sha>,
) {
    let at = format!("{LEDGER_PATH}[{index}].{field}");
    match entry.get(field) {
        None if required => offenders.push(format!(
            "ERROR: {at} is missing — a record must name the commit it was run against"
        )),
        None => {}
        Some(serde_json::Value::String(s)) if is_sha40(s) => shas.push(Sha {
            index,
            field,
            value: s.clone(),
        }),
        Some(serde_json::Value::String(s)) => offenders.push(format!(
            "ERROR: {at}: {s:?} is not a 40-character lowercase-hex git commit — \
             record the full object name (`git rev-parse <ref>`), never a prefix"
        )),
        Some(other) => offenders.push(format!(
            "ERROR: {at} is {} — expected a 40-character lowercase-hex git commit",
            type_name(other)
        )),
    }
}

/// Require a non-empty string field.
fn check_string_field(
    entry: &serde_json::Value,
    index: usize,
    field: &str,
    required: bool,
    offenders: &mut Vec<String>,
) {
    let at = format!("{LEDGER_PATH}[{index}].{field}");
    match entry.get(field) {
        None if required => offenders.push(format!("ERROR: {at} is missing")),
        None => {}
        Some(serde_json::Value::String(s)) if !s.is_empty() => {}
        Some(serde_json::Value::String(_)) => offenders.push(format!("ERROR: {at} is empty")),
        Some(other) => offenders.push(format!(
            "ERROR: {at} is {} — expected a string",
            type_name(other)
        )),
    }
}

/// Check the ledger's shape, returning the offenders and every well-formed SHA
/// it names. A malformed SHA yields an offender and no entry in `shas`: there is
/// nothing to ask git about, and `git merge-base` would fail on it for a second,
/// less useful reason.
pub(crate) fn check_shape(src: &str) -> (Vec<String>, Vec<Sha>) {
    let mut offenders = Vec::new();
    let mut shas = Vec::new();

    let value: serde_json::Value = match serde_json::from_str(src) {
        Ok(v) => v,
        Err(e) => {
            offenders.push(format!(
                "ERROR: {LEDGER_PATH} is not valid JSON: {e} — it is appended by hand, so a \
                 trailing comma or an unclosed brace is the likely cause"
            ));
            return (offenders, shas);
        }
    };

    let entries = match value.as_array() {
        Some(entries) => entries,
        None => {
            offenders.push(format!(
                "ERROR: {LEDGER_PATH} is {} — the ledger is a JSON ARRAY of run records, \
                 appended to; a single object is read by roh-scan as no runs at all",
                type_name(&value)
            ));
            return (offenders, shas);
        }
    };

    if entries.is_empty() {
        offenders.push(format!(
            "ERROR: {LEDGER_PATH} is an empty array — a ledger that records no run is \
             indistinguishable from a repo that has never been mutation-tested, except \
             that it looks like evidence. Delete the file or append the run."
        ));
        return (offenders, shas);
    }

    for (index, entry) in entries.iter().enumerate() {
        if !entry.is_object() {
            offenders.push(format!(
                "ERROR: {LEDGER_PATH}[{index}] is {} — every entry is a run record object",
                type_name(entry)
            ));
            continue;
        }
        let at = format!("{LEDGER_PATH}[{index}]");

        match entry.get("timestamp") {
            Some(serde_json::Value::String(s)) if is_utc_timestamp(s) => {}
            Some(serde_json::Value::String(s)) => offenders.push(format!(
                "ERROR: {at}.timestamp: {s:?} is not YYYY-MM-DDTHH:MM:SSZ — roh-scan ranks \
                 runs by comparing these as strings, so any other width or zone sorts wrongly"
            )),
            Some(other) => offenders.push(format!(
                "ERROR: {at}.timestamp is {} — expected a YYYY-MM-DDTHH:MM:SSZ string",
                type_name(other)
            )),
            None => offenders.push(format!(
                "ERROR: {at}.timestamp is missing — a record that cannot be ranked is dropped \
                 by roh-scan, silently under-reporting when this repo was last scanned"
            )),
        }

        match entry.get("tool") {
            Some(serde_json::Value::String(s)) if s == TOOL => {}
            Some(other) => offenders.push(format!(
                "ERROR: {at}.tool is {}, expected the string {TOOL:?} — the audit skill's \
                 gate reads this field to recognise its own evidence",
                match other {
                    serde_json::Value::String(s) => format!("{s:?}"),
                    _ => type_name(other).to_string(),
                }
            )),
            None => offenders.push(format!("ERROR: {at}.tool is missing (expected {TOOL:?})")),
        }

        check_sha_field(entry, index, "commit", true, &mut offenders, &mut shas);
        // Absent is legal, malformed is not — see the module doc.
        check_sha_field(
            entry,
            index,
            "testsAfterCommit",
            false,
            &mut offenders,
            &mut shas,
        );

        check_string_field(entry, index, "scope", true, &mut offenders);
        check_string_field(entry, index, "skillVersion", false, &mut offenders);

        match entry.get("summary") {
            Some(serde_json::Value::Object(_)) => {}
            Some(other) => offenders.push(format!(
                "ERROR: {at}.summary is {} — expected an object (its inner shape is per-repo \
                 and deliberately not checked)",
                type_name(other)
            )),
            None => offenders.push(format!(
                "ERROR: {at}.summary is missing — a run with no summary records that it \
                 happened and nothing about what it found"
            )),
        }
    }

    (offenders, shas)
}

/// Turn each SHA's ancestry verdict into an offender line. Pure, so the message
/// for every verdict is covered without a git repo; `git_probe` is the thin
/// shell-out underneath it.
pub(crate) fn ancestry_offenders(
    shas: &[Sha],
    probe: impl Fn(&str) -> Result<Ancestry, String>,
) -> Result<Vec<String>, String> {
    let mut offenders = Vec::new();
    for sha in shas {
        match probe(&sha.value)? {
            Ancestry::Ancestor => {}
            Ancestry::NotAncestor => offenders.push(format!(
                "ERROR: {}: {} is not an ancestor of HEAD — it is a commit in this repo but \
                 not on this history, so every \"since the last run\" comparison against it is \
                 based on a tree this branch does not contain. On a squash-merging repo, record \
                 the merged commit in a follow-up rather than the PR head.",
                sha.at(),
                sha.value
            )),
            Ancestry::Unknown => offenders.push(format!(
                "ERROR: {}: {} is not a commit in this repo — a mistyped or rebased-away SHA. \
                 The run it claims to record names a tree that does not exist, so nothing it \
                 reports can be checked.",
                sha.at(),
                sha.value
            )),
        }
    }
    Ok(offenders)
}

/// Run git in `root`, returning `Err` only when git could not be executed at
/// all — a nonzero exit is an answer here, not a failure.
fn git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git {}: {e}", args.join(" ")))
}

/// True when the checkout has a grafted history. Ancestry is unanswerable there
/// — every commit older than the shallow boundary looks unreachable — so the
/// caller must refuse rather than report a false non-ancestor.
pub(crate) fn is_shallow(root: &Path) -> Result<bool, String> {
    let out = git(root, &["rev-parse", "--is-shallow-repository"])?;
    if !out.status.success() {
        return Err(format!(
            "git rev-parse --is-shallow-repository failed: {} — is {} a git checkout?",
            String::from_utf8_lossy(&out.stderr).trim(),
            root.display()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim() == "true")
}

/// Ask git where `sha` sits relative to `HEAD`.
pub(crate) fn git_probe(root: &Path, sha: &str) -> Result<Ancestry, String> {
    // Resolved first and separately: `merge-base --is-ancestor` exits 128 for an
    // unknown object and 1 for a known non-ancestor, and telling those apart
    // from the exit code alone means parsing git's stderr prose.
    let exists = git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{sha}^{{commit}}"),
        ],
    )?;
    if !exists.status.success() {
        return Ok(Ancestry::Unknown);
    }
    let out = git(root, &["merge-base", "--is-ancestor", sha, "HEAD"])?;
    match out.status.code() {
        Some(0) => Ok(Ancestry::Ancestor),
        Some(1) => Ok(Ancestry::NotAncestor),
        other => Err(format!(
            "git merge-base --is-ancestor {sha} HEAD exited {other:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// The subcommand. Returns the offenders, or `Err` for a condition that makes
/// the check unanswerable rather than failed.
pub(crate) fn check(root: &Path, path: &str) -> Result<Option<Vec<String>>, String> {
    let ledger: PathBuf = root.join(path);
    if !ledger.is_file() {
        // A repo that has never been mutation-tested, which is not a defect.
        return Ok(None);
    }
    let src = std::fs::read_to_string(&ledger)
        .map_err(|e| format!("failed to read {}: {e}", ledger.display()))?;

    let (mut offenders, shas) = check_shape(&src);

    if !shas.is_empty() {
        if is_shallow(root)? {
            return Err(format!(
                "{path} names commits but {} is a shallow checkout, where ancestry is \
                 unanswerable — fetch full history first (`git fetch --no-tags \
                 --filter=tree:0 --unshallow origin`)",
                root.display()
            ));
        }
        offenders.extend(ancestry_offenders(&shas, |sha| git_probe(root, sha))?);
    }

    Ok(Some(offenders))
}

/// Print the verdict and exit. `fail` is the crate-wide `::error::` annotation.
pub(crate) fn run(root: &Path, path: &str) {
    match check(root, path) {
        Err(e) => crate::fail(&format!("mutation-ledger: {e}")),
        Ok(None) => println!("mutation-ledger: no {path}; skip"),
        Ok(Some(offenders)) if offenders.is_empty() => println!("mutation-ledger: clean"),
        Ok(Some(offenders)) => {
            for line in offenders {
                println!("{line}");
            }
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// One valid record, the shape every repo in the org carries today.
    const GOOD: &str = r#"[
      {
        "timestamp": "2026-09-03T09:40:00Z",
        "commit": "5055221444d547aba034b88fbd2525973b17d9a9",
        "testsAfterCommit": "9675b6e169516372e9fcd1a5449d32dc4b1fb3a9",
        "publishedTag": "v0.1.0",
        "commitsAheadOfTag": 10,
        "scope": "whole repo",
        "tool": "adversarial-mutation-test",
        "skillVersion": "0.35.0",
        "summary": { "mutants": 12, "killed": 12 }
      }
    ]"#;

    fn shape_errors(src: &str) -> Vec<String> {
        check_shape(src).0
    }

    #[test]
    fn a_valid_record_is_clean_and_yields_both_shas() {
        let (offenders, shas) = check_shape(GOOD);
        assert!(offenders.is_empty(), "{offenders:?}");
        assert_eq!(
            shas,
            vec![
                Sha {
                    index: 0,
                    field: "commit",
                    value: "5055221444d547aba034b88fbd2525973b17d9a9".into()
                },
                Sha {
                    index: 0,
                    field: "testsAfterCommit",
                    value: "9675b6e169516372e9fcd1a5449d32dc4b1fb3a9".into()
                },
            ]
        );
    }

    /// raindex, rain.string, rain.math.binary and five more carry records
    /// written before the skill required `testsAfterCommit`. They are valid.
    #[test]
    fn an_omitted_tests_after_commit_is_legal() {
        let src = GOOD.replace(
            r#""testsAfterCommit": "9675b6e169516372e9fcd1a5449d32dc4b1fb3a9","#,
            "",
        );
        let (offenders, shas) = check_shape(&src);
        assert!(offenders.is_empty(), "{offenders:?}");
        assert_eq!(shas.len(), 1);
        assert_eq!(shas[0].field, "commit");
    }

    /// …but writing it wrong is not, and neither is writing it as null: the
    /// skill's rule is "equal to `commit` when the run landed nothing", never
    /// a placeholder.
    #[test]
    fn a_malformed_tests_after_commit_is_caught() {
        for bad in [r#""9675b6e""#, "null", "42"] {
            let src = GOOD.replace(r#""9675b6e169516372e9fcd1a5449d32dc4b1fb3a9""#, bad);
            let errs = shape_errors(&src);
            assert_eq!(errs.len(), 1, "{bad}: {errs:?}");
            assert!(errs[0].contains("[0].testsAfterCommit"), "{errs:?}");
        }
    }

    /// The live defect in rainlanguage/rain.solmem and ST0x-Technology/st0x.oracle.
    #[test]
    fn a_short_commit_is_caught() {
        let src = GOOD.replace("5055221444d547aba034b88fbd2525973b17d9a9", "b3bd859");
        let errs = shape_errors(&src);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0].commit"), "{errs:?}");
        assert!(errs[0].contains("never a prefix"), "{errs:?}");
    }

    #[test]
    fn an_uppercase_commit_is_caught() {
        let src = GOOD.replace(
            "5055221444d547aba034b88fbd2525973b17d9a9",
            "5055221444D547ABA034B88FBD2525973B17D9A9",
        );
        let errs = shape_errors(&src);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0].commit"), "{errs:?}");
    }

    #[test]
    fn a_non_hex_commit_of_the_right_length_is_caught() {
        let src = GOOD.replace(
            "5055221444d547aba034b88fbd2525973b17d9a9",
            "zzz5221444d547aba034b88fbd2525973b17d9a9",
        );
        let errs = shape_errors(&src);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0].commit"), "{errs:?}");
    }

    #[test]
    fn a_missing_commit_is_caught() {
        let src = GOOD.replace(
            r#""commit": "5055221444d547aba034b88fbd2525973b17d9a9","#,
            "",
        );
        let errs = shape_errors(&src);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0].commit is missing"), "{errs:?}");
    }

    #[test]
    fn a_trailing_comma_is_caught_as_invalid_json() {
        let src = GOOD.replace("}\n    ]", "},\n    ]");
        let errs = shape_errors(&src);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("not valid JSON"), "{errs:?}");
    }

    #[test]
    fn a_bare_object_is_not_a_ledger() {
        let src = GOOD.trim().trim_start_matches('[').trim_end_matches(']');
        let errs = shape_errors(src);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("JSON ARRAY"), "{errs:?}");
    }

    #[test]
    fn an_empty_array_is_not_a_ledger() {
        let errs = shape_errors("[]");
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("empty array"), "{errs:?}");
    }

    #[test]
    fn a_non_object_entry_is_caught_without_masking_its_neighbours() {
        let src = format!(
            "[\"whoops\", {}]",
            GOOD.trim().trim_start_matches('[').trim_end_matches(']')
        );
        let errs = shape_errors(&src);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0] is a string"), "{errs:?}");
    }

    #[test]
    fn every_bad_record_is_reported_not_just_the_first() {
        let src = r#"[
          {"timestamp": "2026-01-01T00:00:00Z", "commit": "abc", "tool": "adversarial-mutation-test", "scope": "x", "summary": {}},
          {"timestamp": "2026-01-02T00:00:00Z", "commit": "def", "tool": "adversarial-mutation-test", "scope": "x", "summary": {}}
        ]"#;
        let errs = shape_errors(src);
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert!(errs[0].contains("[0].commit"), "{errs:?}");
        assert!(errs[1].contains("[1].commit"), "{errs:?}");
    }

    #[test]
    fn the_wrong_tool_is_caught() {
        let src = GOOD.replace("adversarial-mutation-test", "some-other-tool");
        let errs = shape_errors(&src);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0].tool"), "{errs:?}");
    }

    #[test]
    fn a_missing_scope_or_summary_is_caught() {
        let no_scope = GOOD.replace(r#""scope": "whole repo","#, "");
        let errs = shape_errors(&no_scope);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0].scope is missing"), "{errs:?}");

        let no_summary = GOOD.replace(r#""summary":"#, r#""notSummary":"#);
        let errs = shape_errors(&no_summary);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0].summary is missing"), "{errs:?}");
    }

    /// rain.extrospection's record carries no `skillVersion`, and that is not a
    /// reason to red its CI — but an empty or non-string one is a typo.
    #[test]
    fn skill_version_is_optional_but_shape_checked_when_written() {
        let absent = GOOD.replace(r#""skillVersion": "0.35.0","#, "");
        assert!(shape_errors(&absent).is_empty());

        let empty = GOOD.replace(r#""skillVersion": "0.35.0""#, r#""skillVersion": """#);
        let errs = shape_errors(&empty);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("[0].skillVersion is empty"), "{errs:?}");
    }

    #[test]
    fn timestamps_must_be_utc_seconds_precision() {
        assert!(is_utc_timestamp("2026-09-03T09:40:00Z"));
        assert!(is_utc_timestamp("2024-02-29T23:59:59Z")); // leap day
        assert!(!is_utc_timestamp("2026-02-29T00:00:00Z")); // 2026 is not a leap year
        assert!(!is_utc_timestamp("2026-02-31T00:00:00Z"));
        assert!(!is_utc_timestamp("2026-13-01T00:00:00Z"));
        assert!(!is_utc_timestamp("2026-00-01T00:00:00Z"));
        assert!(!is_utc_timestamp("2026-09-00T00:00:00Z"));
        assert!(!is_utc_timestamp("2026-09-03T24:00:00Z"));
        assert!(!is_utc_timestamp("2026-09-03T09:60:00Z"));
        assert!(!is_utc_timestamp("2026-09-03T09:40:60Z"));
        assert!(!is_utc_timestamp("2026-09-03 09:40:00Z")); // space, not T
        assert!(!is_utc_timestamp("2026-09-03T09:40:00")); // no zone
        assert!(!is_utc_timestamp("2026-09-03T09:40Z")); // no seconds
        assert!(!is_utc_timestamp("2026-09-03T09:40:00+00:00")); // offset sorts wrongly
        assert!(!is_utc_timestamp("2026-09-03T09:40:00.000Z")); // fractional sorts wrongly
        assert!(!is_utc_timestamp("2026-09-03T09:40:00ZZ"));
    }

    #[test]
    fn a_bad_timestamp_is_caught_in_a_record() {
        for bad in [
            r#""2026-09-03 09:40:00Z""#,
            r#""2026-09-03T09:40Z""#,
            r#""2026-02-31T00:00:00Z""#,
            "1757000000",
        ] {
            let src = GOOD.replace(r#""2026-09-03T09:40:00Z""#, bad);
            let errs = shape_errors(&src);
            assert_eq!(errs.len(), 1, "{bad}: {errs:?}");
            assert!(errs[0].contains("[0].timestamp"), "{errs:?}");
        }
    }

    #[test]
    fn sha40_shape() {
        assert!(is_sha40("5055221444d547aba034b88fbd2525973b17d9a9"));
        assert!(!is_sha40("5055221444d547aba034b88fbd2525973b17d9a")); // 39
        assert!(!is_sha40("5055221444d547aba034b88fbd2525973b17d9a9a")); // 41
        assert!(!is_sha40("5055221"));
        assert!(!is_sha40(""));
        assert!(!is_sha40("5055221444D547ABA034B88FBD2525973B17D9A9"));
        assert!(!is_sha40("gggg221444d547aba034b88fbd2525973b17d9a9"));
    }

    fn sha(field: &'static str, value: &str) -> Sha {
        Sha {
            index: 0,
            field,
            value: value.to_string(),
        }
    }

    #[test]
    fn an_ancestor_produces_no_offender() {
        let shas = vec![sha("commit", "5055221444d547aba034b88fbd2525973b17d9a9")];
        let out = ancestry_offenders(&shas, |_| Ok(Ancestry::Ancestor)).unwrap();
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn a_non_ancestor_and_an_unknown_sha_read_differently() {
        let shas = vec![sha("commit", "5055221444d547aba034b88fbd2525973b17d9a9")];
        let not = ancestry_offenders(&shas, |_| Ok(Ancestry::NotAncestor)).unwrap();
        assert_eq!(not.len(), 1);
        assert!(not[0].contains("is not an ancestor of HEAD"), "{not:?}");
        assert!(
            not[0].contains("audit/mutation-test-scans.json[0].commit"),
            "{not:?}"
        );

        let unknown = ancestry_offenders(&shas, |_| Ok(Ancestry::Unknown)).unwrap();
        assert_eq!(unknown.len(), 1);
        assert!(
            unknown[0].contains("is not a commit in this repo"),
            "{unknown:?}"
        );
    }

    #[test]
    fn a_probe_failure_is_an_error_not_a_pass() {
        let shas = vec![sha("commit", "5055221444d547aba034b88fbd2525973b17d9a9")];
        assert!(ancestry_offenders(&shas, |_| Err("git exploded".into())).is_err());
    }

    #[test]
    fn every_recorded_sha_is_probed_and_named_by_its_own_field() {
        let shas = vec![
            sha("commit", "1111111111111111111111111111111111111111"),
            sha(
                "testsAfterCommit",
                "2222222222222222222222222222222222222222",
            ),
        ];
        let out = ancestry_offenders(&shas, |_| Ok(Ancestry::Unknown)).unwrap();
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out[0].contains("[0].commit"), "{out:?}");
        assert!(out[1].contains("[0].testsAfterCommit"), "{out:?}");
    }

    // ---- real git, so `git_probe`'s exit-code reading is covered rather than
    // ---- described. `pkgs.git` is in the derivation's nativeCheckInputs.

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct TempRepo(PathBuf);

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    impl TempRepo {
        /// A repo with two commits on `HEAD` and one commit on a side branch
        /// that `HEAD` cannot reach.
        fn new() -> TempRepo {
            let dir = std::env::temp_dir().join(format!(
                "rainix-mutation-ledger-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let repo = TempRepo(dir);
            repo.git(&["init", "--initial-branch=main", "."]);
            repo.commit("a");
            repo.commit("b");
            repo
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn git(&self, args: &[&str]) -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(&self.0)
                .args(args)
                // Hermetic: no user, system or global config, and an identity
                // that does not depend on one existing.
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("GIT_AUTHOR_NAME", "rainix")
                .env("GIT_AUTHOR_EMAIL", "rainix@example.com")
                .env("GIT_COMMITTER_NAME", "rainix")
                .env("GIT_COMMITTER_EMAIL", "rainix@example.com")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }

        fn commit(&self, name: &str) -> String {
            std::fs::write(self.0.join(name), name).unwrap();
            self.git(&["add", name]);
            self.git(&["commit", "-m", name]);
            self.git(&["rev-parse", "HEAD"])
        }
    }

    #[test]
    fn git_probe_separates_ancestor_non_ancestor_and_unknown() {
        let repo = TempRepo::new();
        let head = repo.git(&["rev-parse", "HEAD"]);
        let first = repo.git(&["rev-parse", "HEAD~1"]);

        // A commit that exists but is not on this branch.
        repo.git(&["checkout", "-q", "-b", "side", "HEAD~1"]);
        let side = repo.commit("side");
        repo.git(&["checkout", "-q", "main"]);

        assert_eq!(git_probe(repo.path(), &head).unwrap(), Ancestry::Ancestor);
        assert_eq!(git_probe(repo.path(), &first).unwrap(), Ancestry::Ancestor);
        assert_eq!(
            git_probe(repo.path(), &side).unwrap(),
            Ancestry::NotAncestor
        );
        assert_eq!(
            git_probe(repo.path(), "0123456789abcdef0123456789abcdef01234567").unwrap(),
            Ancestry::Unknown
        );
        assert!(!is_shallow(repo.path()).unwrap());
    }

    /// A tag or branch NAME resolves in git but is not what the ledger records;
    /// the shape half rejects it before git is ever asked, so a moving ref can
    /// never be mistaken for a pinned tree.
    #[test]
    fn a_ref_name_never_reaches_the_ancestry_probe() {
        let src = GOOD.replace("5055221444d547aba034b88fbd2525973b17d9a9", "main");
        let (offenders, shas) = check_shape(&src);
        assert_eq!(offenders.len(), 1, "{offenders:?}");
        assert!(shas.iter().all(|s| s.field != "commit"), "{shas:?}");
    }

    #[test]
    fn a_repo_with_no_ledger_passes_rather_than_failing_for_absence() {
        let repo = TempRepo::new();
        assert_eq!(check(repo.path(), LEDGER_PATH).unwrap(), None);
    }

    #[test]
    fn a_real_ledger_of_real_commits_is_clean_end_to_end() {
        let repo = TempRepo::new();
        let head = repo.git(&["rev-parse", "HEAD"]);
        let first = repo.git(&["rev-parse", "HEAD~1"]);
        std::fs::create_dir_all(repo.path().join("audit")).unwrap();
        std::fs::write(
            repo.path().join(LEDGER_PATH),
            GOOD.replace("5055221444d547aba034b88fbd2525973b17d9a9", &first)
                .replace("9675b6e169516372e9fcd1a5449d32dc4b1fb3a9", &head),
        )
        .unwrap();
        assert_eq!(check(repo.path(), LEDGER_PATH).unwrap(), Some(vec![]));
    }

    #[test]
    fn a_ledger_naming_a_commit_from_nowhere_fails_end_to_end() {
        let repo = TempRepo::new();
        let offenders = check(repo.path(), LEDGER_PATH);
        assert_eq!(offenders.unwrap(), None);

        std::fs::create_dir_all(repo.path().join("audit")).unwrap();
        std::fs::write(repo.path().join(LEDGER_PATH), GOOD).unwrap();
        let offenders = check(repo.path(), LEDGER_PATH).unwrap().unwrap();
        assert_eq!(offenders.len(), 2, "{offenders:?}");
        assert!(
            offenders
                .iter()
                .all(|o| o.contains("is not a commit in this repo")),
            "{offenders:?}"
        );
    }

    /// Ancestry cannot be answered on a grafted history, so the check refuses
    /// instead of reporting every older commit as unreachable.
    #[test]
    fn a_shallow_checkout_is_refused_rather_than_answered_wrongly() {
        let origin = TempRepo::new();
        origin.commit("c");
        let dir = std::env::temp_dir().join(format!(
            "rainix-mutation-ledger-shallow-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let clone = TempRepo(dir);
        let out = Command::new("git")
            .args(["clone", "-q", "--depth", "1", "--no-local"])
            .arg(format!("file://{}", origin.path().display()))
            .arg(clone.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "clone: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(is_shallow(clone.path()).unwrap());

        std::fs::create_dir_all(clone.path().join("audit")).unwrap();
        std::fs::write(clone.path().join(LEDGER_PATH), GOOD).unwrap();
        let err = check(clone.path(), LEDGER_PATH).unwrap_err();
        assert!(err.contains("shallow checkout"), "{err}");
    }

    #[test]
    fn outside_a_git_checkout_the_ancestry_half_errors_rather_than_passing() {
        let dir = std::env::temp_dir().join(format!(
            "rainix-mutation-ledger-nogit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let plain = TempRepo(dir);
        std::fs::create_dir_all(plain.path().join("audit")).unwrap();
        std::fs::write(plain.path().join(LEDGER_PATH), GOOD).unwrap();
        // A `.git` FILE git cannot parse, rather than no `.git` at all: git walks
        // up from the directory it is handed, so an absent one would assert
        // against whatever repo happens to contain $TMPDIR.
        std::fs::write(plain.path().join(".git"), "not a gitfile").unwrap();
        assert!(check(plain.path(), LEDGER_PATH).is_err());
    }
}
