//! `comment-loc-cap` — fail when comment lines exceed twice the code lines,
//! summed over every tracked source file under the given paths.
//!
//! One aggregate cap per BUCKET, strict: at twice passes. A single prose-heavy
//! file is fine if its bucket is under. Buckets exist because one aggregate
//! over the whole repo lets a code-heavy test tree pay for prose in `src`:
//! tests run long and assert in bulk, so they carry a ratio far under the cap
//! and raise the denominator the prose is measured against. Splitting them
//! means each tree answers for its own.
//!
//! Counting is `scc`'s, not ours. It lexes per language, so a marker inside a
//! string literal is code and a doc comment is a comment. What stays here is
//! WHICH files count: an extension allowlist, because `scc` scores Markdown
//! prose as comments and a README would otherwise fail every repo on its own.
//!
//! Only files `git ls-files` reports are read, so a vendored or generated tree
//! that is not tracked never fails the check.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Whether a file counts toward the cap, by extension.
///
/// Prose formats are absent deliberately: `scc` counts Markdown text as
/// comments, so counting `.md` would fail any repo with documentation.
pub(crate) fn is_counted(path: &str) -> bool {
    let Some(ext) = path.rsplit('/').next().and_then(|n| n.rsplit_once('.')) else {
        return false;
    };
    matches!(
        ext.1,
        "sol"
            | "rs"
            | "ts"
            | "tsx"
            | "mts"
            | "cts"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "sh"
            | "bash"
            | "toml"
            | "yaml"
            | "yml"
            | "nix"
    )
}

/// Comment and code line counts of one file.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Counts {
    pub(crate) comment: usize,
    pub(crate) code: usize,
}

/// The buckets the cap is applied to when the caller names none. Two, so that
/// `test` answers for its own ratio rather than funding `src`'s.
///
/// `.github` stays with `src` because `test` is the tree being split out, and
/// these were one bucket before. It does dilute `src` — rain.deploy's src is
/// 2.18 alone and 1.97 pooled with its `.github` and its generated tree — so a
/// third bucket is a live question, not a settled one.
pub(crate) const DEFAULT_BUCKETS: [&str; 2] = ["src .github", "test"];

/// Why a scan counted nothing. The two cases are not interchangeable: one is
/// about the repo's shape, the other is a broken toolchain.
#[derive(Debug)]
pub(crate) enum ScanError {
    /// The path set selected no tracked counted file. A misconfiguration when
    /// a caller named those paths, and ordinary when a DEFAULT bucket names a
    /// directory this repo does not have.
    NoSourceFile(String),
    /// git or scc failed, or reported something unreadable. Never skippable:
    /// counting nothing because the counter is broken must not read as a repo
    /// that has nothing to count.
    Failed(String),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::NoSourceFile(m) | ScanError::Failed(m) => f.write_str(m),
        }
    }
}

/// Split a bucket's paths on whitespace and commas.
pub(crate) fn parse_paths(spec: &str) -> Vec<String> {
    spec.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Every counted file `scc` reported, keyed by the path it was given.
fn parse_scc(stdout: &[u8]) -> Result<HashMap<String, Counts>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(stdout).map_err(|e| format!("scc output is not JSON: {e}"))?;
    let languages = value
        .as_array()
        .ok_or_else(|| "scc output is not an array of languages".to_string())?;
    let mut counts = HashMap::new();
    for language in languages {
        let Some(files) = language.get("Files").and_then(|f| f.as_array()) else {
            continue;
        };
        for file in files {
            let name = file
                .get("Location")
                .and_then(|n| n.as_str())
                .ok_or_else(|| "scc reported a file with no Location".to_string())?;
            let field = |key: &str| {
                file.get(key)
                    .and_then(|n| n.as_u64())
                    .ok_or_else(|| format!("scc reported {name} with no {key}"))
            };
            counts.insert(
                name.to_string(),
                Counts {
                    comment: field("Comment")? as usize,
                    code: field("Code")? as usize,
                },
            );
        }
    }
    Ok(counts)
}

/// Every tracked, counted file under `paths` with its counts, in `git
/// ls-files` order. `Err` when git or scc fails, or when nothing is counted —
/// which of the two is the caller's to act on, so they are distinct variants
/// rather than one string.
pub(crate) fn scan(root: &Path, paths: &[String]) -> Result<Vec<(String, Counts)>, ScanError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--"])
        .args(paths)
        .output()
        .map_err(|e| ScanError::Failed(format!("git ls-files failed to spawn: {e}")))?;
    if !out.status.success() {
        return Err(ScanError::Failed(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let names: Vec<String> = out
        .stdout
        .split(|&c| c == 0)
        .filter(|n| !n.is_empty())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .filter(|n| is_counted(n))
        .collect();
    if names.is_empty() {
        return Err(ScanError::NoSourceFile(format!(
            "no tracked source file under {} (checked from {})",
            paths.join(" "),
            root.display()
        )));
    }

    // The file list is explicit rather than handing scc the paths, so only
    // TRACKED files are counted. The ignore logic is off for the same reason:
    // a tracked file that also matches an ignore rule is still a file this
    // repo ships, and scc skipping it would quietly shrink the denominator.
    let out = Command::new("scc")
        .current_dir(root)
        .args([
            "--format",
            "json",
            "--by-file",
            "--no-gitignore",
            "--no-ignore",
        ])
        .args(&names)
        .output()
        .map_err(|e| ScanError::Failed(format!("scc failed to spawn: {e}")))?;
    if !out.status.success() {
        return Err(ScanError::Failed(format!(
            "scc failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let mut counted = parse_scc(&out.stdout).map_err(ScanError::Failed)?;

    names
        .into_iter()
        .map(|name| {
            let counts = counted
                .remove(&name)
                .ok_or_else(|| ScanError::Failed(format!("scc did not report {name}")))?;
            Ok((name, counts))
        })
        .collect()
}

/// One bucket's comment and code lines, summed.
fn totals(files: &[(String, Counts)]) -> Counts {
    Counts {
        comment: files.iter().map(|(_, c)| c.comment).sum(),
        code: files.iter().map(|(_, c)| c.code).sum(),
    }
}

/// Totals over one bucket's files. Empty when comment lines are at or under
/// twice the code lines in aggregate; otherwise the totals and every file's
/// counts, heaviest comment share first. `bucket` names the path set in the
/// header, because with several buckets the totals alone do not say which one
/// is over.
fn report(bucket: &str, files: &[(String, Counts)]) -> Vec<String> {
    let Counts { comment, code } = totals(files);
    let cap = 2 * code;
    if comment <= cap {
        return Vec::new();
    }
    let mut lines = vec![
        format!(
            "comment-loc-cap: {bucket}: {comment} comment lines against a cap of {cap} (twice {code} code lines) across {} files:",
            files.len()
        ),
        "  comment    code  file".to_string(),
    ];
    let mut sorted: Vec<&(String, Counts)> = files.iter().collect();
    sorted.sort_by(|(_, a), (_, b)| (b.comment * a.code.max(1)).cmp(&(a.comment * b.code.max(1))));
    for (name, c) in sorted {
        lines.push(format!("  {:>7} {:>7}  {name}", c.comment, c.code));
    }
    lines
}

/// What to print for every bucket, and whether any bucket is over its cap.
///
/// A bucket with no files prints as skipped rather than passing silently: a
/// caller reading the log sees that the ratio it expected was never measured.
/// `Err` only when NO bucket counted a file, which is a repo the cap has not
/// been pointed at at all.
pub(crate) fn report_buckets(
    buckets: &[(String, Vec<(String, Counts)>)],
) -> Result<(Vec<String>, bool), String> {
    if buckets.iter().all(|(_, files)| files.is_empty()) {
        return Err(format!(
            "no tracked source file under any bucket: {}",
            buckets
                .iter()
                .map(|(b, _)| format!("`{b}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut lines = Vec::new();
    let mut over = false;
    for (bucket, files) in buckets {
        if files.is_empty() {
            lines.push(format!(
                "comment-loc-cap: {bucket}: no tracked source file — skipped"
            ));
            continue;
        }
        let bucket_lines = report(bucket, files);
        if bucket_lines.is_empty() {
            // The counts, not just a verdict: a bucket's ratio is the number
            // worth watching between runs, and a pass that prints none leaves
            // the only reading of it to the run that fails.
            let Counts { comment, code } = totals(files);
            lines.push(format!(
                "comment-loc-cap: {bucket}: clean — {comment} comment against a cap of {} (twice {code} code lines) across {} files",
                2 * code,
                files.len()
            ));
        } else {
            over = true;
            lines.extend(bucket_lines);
        }
    }
    Ok((lines, over))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn empty_and_twice_pass_over_twice_fails() {
        let at = |comment, code| report("src", &[("f".to_string(), Counts { comment, code })]);
        assert!(at(0, 0).is_empty());
        assert!(at(6, 3).is_empty());
        assert!(!at(7, 3).is_empty());
    }

    #[test]
    fn counted_by_extension() {
        assert!(is_counted("src/A.sol"));
        assert!(is_counted("a/b.test.ts"));
        assert!(is_counted("x.yml"));
        assert!(is_counted("flake.nix"));
        assert!(is_counted(".github/workflows/ci.yaml"));
        // Prose: scc scores its text as comments, so counting it would fail
        // every repo that has documentation.
        assert!(!is_counted("README.md"));
        assert!(!is_counted("dir.sol/noext"));
    }

    #[test]
    fn scc_json_is_parsed_by_location() {
        let json = br#"[{"Name":"Rust","Files":[{"Location":"src/a.rs","Code":10,"Comment":3}]},
                        {"Name":"YAML","Files":[{"Location":"ci.yaml","Code":5,"Comment":1}]}]"#;
        let counts = parse_scc(json).unwrap();
        assert_eq!(
            counts["src/a.rs"],
            Counts {
                comment: 3,
                code: 10
            }
        );
        assert_eq!(
            counts["ci.yaml"],
            Counts {
                comment: 1,
                code: 5
            }
        );
    }

    #[test]
    fn scc_output_that_is_not_json_is_an_error() {
        assert!(parse_scc(b"not json").unwrap_err().contains("not JSON"));
    }

    #[test]
    fn paths_split_on_whitespace_and_commas() {
        assert_eq!(
            parse_paths(" src\ttest\n,script,, "),
            ["src", "test", "script"]
        );
    }

    fn repo() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-commentcap-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("test")).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .arg("-C")
                .arg(&d)
                .args(args)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        std::fs::write(
            d.join("src/Over.sol"),
            "// a\n// b\n// c\n// d\n// e\n// f\n// g\nx;\ny;\n",
        )
        .unwrap();
        std::fs::write(d.join("src/Ok.sol"), "// a\nx;\n").unwrap();
        std::fs::write(d.join("src/notes.md"), "# all\n# comment\n").unwrap();
        std::fs::write(d.join("test/Untracked.sol"), "// a\n// b\nx;\n").unwrap();
        git(&["add", "src"]);
        d
    }

    #[test]
    fn aggregate_over_is_reported_with_totals_and_every_file() {
        let d = repo();
        let files = scan(&d, &["src".into(), "test".into()]).unwrap();
        assert_eq!(
            files,
            vec![
                (
                    "src/Ok.sol".to_string(),
                    Counts {
                        comment: 1,
                        code: 1
                    }
                ),
                (
                    "src/Over.sol".to_string(),
                    Counts {
                        comment: 7,
                        code: 2
                    }
                ),
            ]
        );
        let lines = report("src test", &files);
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(
            lines[0].contains(
                "src test: 8 comment lines against a cap of 6 (twice 3 code lines) across 2 files"
            ),
            "{lines:?}"
        );
        assert!(lines[2].ends_with("7       2  src/Over.sol"), "{lines:?}");
        assert!(lines[3].ends_with("1       1  src/Ok.sol"), "{lines:?}");
    }

    #[test]
    fn a_file_over_on_its_own_passes_when_the_aggregate_is_under() {
        let d = repo();
        std::fs::write(d.join("src/Code.sol"), "x;\ny;\nz;\n").unwrap();
        assert!(Command::new("git")
            .arg("-C")
            .arg(&d)
            .args(["add", "src"])
            .status()
            .unwrap()
            .success());
        let files = scan(&d, &["src".into()]).unwrap();
        assert!(
            report("src", &files).is_empty(),
            "8 comment lines against a cap of 12 is under"
        );
    }

    #[test]
    fn a_clean_scan_reports_nothing() {
        let d = repo();
        assert!(Command::new("git")
            .arg("-C")
            .arg(&d)
            .args(["rm", "-qf", "src/Over.sol"])
            .status()
            .unwrap()
            .success());
        let files = scan(&d, &["src".into()]).unwrap();
        assert!(report("src", &files).is_empty());
    }

    #[test]
    fn a_path_set_selecting_no_source_file_is_an_error() {
        let d = repo();
        let e = scan(&d, &["test".into()]).unwrap_err();
        assert!(matches!(e, ScanError::NoSourceFile(_)), "{e:?}");
        assert!(
            e.to_string().contains("no tracked source file under test"),
            "{e}"
        );
    }

    /// The whole reason buckets exist: `test`'s code is what funded `src`'s
    /// prose under one aggregate, and `src` is where the ratio is worth
    /// reading.
    #[test]
    fn a_bucket_over_its_own_cap_fails_though_the_repo_aggregate_is_under() {
        let d = repo();
        std::fs::write(d.join("test/Heavy.sol"), "x;\n".repeat(20)).unwrap();
        assert!(Command::new("git")
            .arg("-C")
            .arg(&d)
            // Only this file: `repo()` leaves an untracked one beside it, and
            // the counts below are this file's alone.
            .args(["add", "test/Heavy.sol"])
            .status()
            .unwrap()
            .success());
        let src = scan(&d, &["src".into()]).unwrap();
        let test = scan(&d, &["test".into()]).unwrap();

        let mut aggregate = src.clone();
        aggregate.extend(test.clone());
        assert!(
            report("src test", &aggregate).is_empty(),
            "8 comment lines against twice 23 code lines is under in aggregate"
        );

        let (lines, over) =
            report_buckets(&[("src".to_string(), src), ("test".to_string(), test)]).unwrap();
        assert!(over, "{lines:?}");
        assert!(
            lines[0].contains("src: 8 comment lines against a cap of 6"),
            "{lines:?}"
        );
        // A passing bucket still reports its ratio, so the number can be read
        // between runs rather than only when it has already been breached.
        assert!(
            lines.iter().any(|l| l.contains(
                "test: clean — 0 comment against a cap of 40 (twice 20 code lines) across 1 files"
            )),
            "{lines:?}"
        );
    }

    #[test]
    fn an_empty_bucket_is_skipped_rather_than_failing_while_another_counts() {
        let d = repo();
        assert!(Command::new("git")
            .arg("-C")
            .arg(&d)
            .args(["rm", "-qf", "src/Over.sol"])
            .status()
            .unwrap()
            .success());
        let src = scan(&d, &["src".into()]).unwrap();

        let (lines, over) =
            report_buckets(&[("src".to_string(), src), ("test".to_string(), Vec::new())]).unwrap();
        assert!(!over, "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("test: no tracked source file — skipped")),
            "{lines:?}"
        );
    }

    #[test]
    fn every_bucket_empty_is_an_error() {
        let e = report_buckets(&[
            ("src".to_string(), Vec::new()),
            ("test".to_string(), Vec::new()),
        ])
        .unwrap_err();
        assert!(
            e.contains("no tracked source file under any bucket: `src`, `test`"),
            "{e}"
        );
    }

    /// The default must hold `test` apart from `src`, not merely list both.
    #[test]
    fn the_default_buckets_hold_test_apart_from_src() {
        let with_src: Vec<Vec<String>> = DEFAULT_BUCKETS
            .iter()
            .map(|b| parse_paths(b))
            .filter(|p| p.iter().any(|d| d == "src"))
            .collect();
        assert_eq!(with_src.len(), 1, "{DEFAULT_BUCKETS:?}");
        assert!(!with_src[0].iter().any(|d| d == "test"), "{with_src:?}");
        assert!(
            DEFAULT_BUCKETS.iter().any(|b| parse_paths(b) == ["test"]),
            "{DEFAULT_BUCKETS:?}"
        );
    }

    #[test]
    fn outside_a_git_checkout_is_an_error() {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-commentcap-nogit-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        let e = scan(&d, &["src".into()]).unwrap_err();
        // `Failed`, never `NoSourceFile`: a broken counter must not read as a
        // bucket with nothing in it, which a default bucket would skip.
        assert!(matches!(e, ScanError::Failed(_)), "{e:?}");
        assert!(e.to_string().contains("git ls-files failed"), "{e}");
    }
}
