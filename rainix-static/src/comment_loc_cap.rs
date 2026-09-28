//! `comment-loc-cap` — fail when comment lines exceed twice the code lines,
//! summed over every tracked source file under the given paths.
//!
//! One aggregate cap, strict: at twice passes. A single prose-heavy file is
//! fine if the tree is under.
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

/// Split the `--paths` value on whitespace and commas.
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
/// ls-files` order. `Err` when git or scc fails, or when nothing is counted: a
/// path set that selects no source file is a misconfiguration, not a pass.
pub(crate) fn scan(root: &Path, paths: &[String]) -> Result<Vec<(String, Counts)>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--"])
        .args(paths)
        .output()
        .map_err(|e| format!("git ls-files failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let names: Vec<String> = out
        .stdout
        .split(|&c| c == 0)
        .filter(|n| !n.is_empty())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .filter(|n| is_counted(n))
        .collect();
    if names.is_empty() {
        return Err(format!(
            "no tracked source file under {} (checked from {})",
            paths.join(" "),
            root.display()
        ));
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
        .map_err(|e| format!("scc failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "scc failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut counted = parse_scc(&out.stdout)?;

    names
        .into_iter()
        .map(|name| {
            let counts = counted
                .remove(&name)
                .ok_or_else(|| format!("scc did not report {name}"))?;
            Ok((name, counts))
        })
        .collect()
}

/// Totals over every scanned file. Empty when comment lines are at or under
/// twice the code lines in aggregate; otherwise the totals and every file's
/// counts, heaviest comment share first.
pub(crate) fn report(files: &[(String, Counts)]) -> Vec<String> {
    let comment: usize = files.iter().map(|(_, c)| c.comment).sum();
    let code: usize = files.iter().map(|(_, c)| c.code).sum();
    let cap = 2 * code;
    if comment <= cap {
        return Vec::new();
    }
    let mut lines = vec![
        format!(
            "comment-loc-cap: {comment} comment lines against a cap of {cap} (twice {code} code lines) across {} files:",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn empty_and_twice_pass_over_twice_fails() {
        let at = |comment, code| report(&[("f".to_string(), Counts { comment, code })]);
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
        let lines = report(&files);
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(
            lines[0]
                .contains("8 comment lines against a cap of 6 (twice 3 code lines) across 2 files"),
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
            report(&files).is_empty(),
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
        assert!(report(&files).is_empty());
    }

    #[test]
    fn a_path_set_selecting_no_source_file_is_an_error() {
        let d = repo();
        let e = scan(&d, &["test".into()]).unwrap_err();
        assert!(e.contains("no tracked source file under test"), "{e}");
    }

    #[test]
    fn outside_a_git_checkout_is_an_error() {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-commentcap-nogit-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        let e = scan(&d, &["src".into()]).unwrap_err();
        assert!(e.contains("git ls-files failed"), "{e}");
    }
}
