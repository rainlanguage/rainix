//! `comment-loc-cap` — fail when a tracked source file has more comment lines
//! than code lines.
//!
//! The cap is per file and strict: a file whose comment line count exceeds its
//! code line count fails, equal passes. Blank lines count as neither. A line is
//! a comment line when everything on it is inside a comment; a line carrying
//! any code, with or without a trailing comment, is a code line. Comment syntax
//! follows the extension: `//` and `/* */` for Solidity, Rust and JS/TS; `#`
//! for shell, TOML and YAML; `#` plus `/* */` for Nix. A line-1 shebang is
//! code. Files with any other extension are not counted.
//!
//! Only files `git ls-files` reports under the given paths are read, so a
//! vendored or generated tree that is not tracked never fails the check.

use std::path::Path;
use std::process::Command;

/// Which markers open a comment in a file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Syntax {
    /// `//` to end of line, `/* … */` block.
    CStyle,
    /// `#` to end of line.
    Hash,
    /// `#` to end of line, `/* … */` block.
    HashBlock,
}

impl Syntax {
    fn line_marker(self) -> &'static str {
        match self {
            Syntax::CStyle => "//",
            Syntax::Hash | Syntax::HashBlock => "#",
        }
    }

    fn has_block(self) -> bool {
        matches!(self, Syntax::CStyle | Syntax::HashBlock)
    }
}

/// The comment syntax a file uses, by extension; `None` for a file the check
/// does not count.
pub(crate) fn syntax_for(path: &str) -> Option<Syntax> {
    let ext = path.rsplit('/').next()?.rsplit_once('.')?.1;
    match ext {
        "sol" | "rs" | "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => {
            Some(Syntax::CStyle)
        }
        "sh" | "bash" | "toml" | "yaml" | "yml" => Some(Syntax::Hash),
        "nix" => Some(Syntax::HashBlock),
        _ => None,
    }
}

/// Comment and code line counts of one file.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Counts {
    pub(crate) comment: usize,
    pub(crate) code: usize,
}

impl Counts {
    pub(crate) fn over(self) -> bool {
        self.comment > self.code
    }
}

/// Classify every line of `text`. Block comment state carries across lines;
/// a `"…"` or `'…'` literal on a code line is skipped so a marker inside it
/// does not open a comment.
pub(crate) fn count(text: &str, syntax: Syntax) -> Counts {
    let marker = syntax.line_marker().as_bytes();
    let mut counts = Counts::default();
    let mut in_block = false;
    for (n, line) in text.lines().enumerate() {
        let b = line.as_bytes();
        let mut has_code = false;
        let mut has_comment = false;
        let mut i = 0;
        if n == 0 && b.starts_with(b"#!") {
            has_code = true;
            i = b.len();
        }
        while i < b.len() {
            if in_block {
                has_comment = true;
                match find(b, i, b"*/") {
                    Some(end) => {
                        in_block = false;
                        i = end + 2;
                    }
                    None => break,
                }
                continue;
            }
            if b[i].is_ascii_whitespace() {
                i += 1;
                continue;
            }
            if b[i..].starts_with(marker) {
                has_comment = true;
                break;
            }
            if syntax.has_block() && b[i..].starts_with(b"/*") {
                in_block = true;
                i += 2;
                continue;
            }
            has_code = true;
            if b[i] == b'"' || b[i] == b'\'' {
                i = string_end(b, i);
            } else {
                i += 1;
            }
        }
        if has_code {
            counts.code += 1;
        } else if has_comment {
            counts.comment += 1;
        }
    }
    counts
}

/// Byte index of `needle` in `b` at or after `from`.
fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    (from..=b.len().saturating_sub(needle.len())).find(|&i| b[i..].starts_with(needle))
}

/// Index just past the literal opened by the quote at `open`, honouring
/// backslash escapes; the end of the line when it never closes.
fn string_end(b: &[u8], open: usize) -> usize {
    let q = b[open];
    let mut i = open + 1;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q {
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// Split the `--paths` value on whitespace and commas.
pub(crate) fn parse_paths(spec: &str) -> Vec<String> {
    spec.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Every tracked, counted file under `paths` with its counts, in `git
/// ls-files` order. `Err` when git fails or nothing is counted: a path set
/// that selects no source file is a misconfiguration, not a pass.
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
    let mut files = Vec::new();
    for name in out.stdout.split(|&c| c == 0).filter(|n| !n.is_empty()) {
        let name = String::from_utf8_lossy(name).into_owned();
        let Some(syntax) = syntax_for(&name) else {
            continue;
        };
        let bytes =
            std::fs::read(root.join(&name)).map_err(|e| format!("{name}: cannot read: {e}"))?;
        files.push((name, count(&String::from_utf8_lossy(&bytes), syntax)));
    }
    if files.is_empty() {
        return Err(format!(
            "no tracked source file under {} (checked from {})",
            paths.join(" "),
            root.display()
        ));
    }
    Ok(files)
}

/// The report lines for the files over the cap: a header, then one row per
/// offender with both counts. Empty when every file passes.
pub(crate) fn report(files: &[(String, Counts)]) -> Vec<String> {
    let over: Vec<&(String, Counts)> = files.iter().filter(|(_, c)| c.over()).collect();
    if over.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        format!(
            "comment-loc-cap: {} of {} files have more comment lines than code lines:",
            over.len(),
            files.len()
        ),
        "  comment    code  file".to_string(),
    ];
    for (name, c) in over {
        lines.push(format!("  {:>7} {:>7}  {name}", c.comment, c.code));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn c(text: &str) -> Counts {
        count(text, Syntax::CStyle)
    }

    #[test]
    fn line_comments_and_blank_lines() {
        let t = "// a\n/// b\n\nuint x;\n   \n// c\n";
        assert_eq!(c(t), Counts { comment: 3, code: 1 });
    }

    #[test]
    fn code_with_trailing_comment_is_code() {
        assert_eq!(c("x = 1; // why\n"), Counts { comment: 0, code: 1 });
    }

    #[test]
    fn block_comment_spans_lines() {
        let t = "/**\n * doc\n */\nfunction f() {}\n/* a */ x;\ny; /* b\nstill b */\n/* c */\n";
        assert_eq!(c(t), Counts { comment: 5, code: 3 });
    }

    #[test]
    fn marker_inside_string_does_not_comment() {
        assert_eq!(c("s = \"http://x\";\n"), Counts { comment: 0, code: 1 });
        assert_eq!(c("s = '/*';\nt = 1;\n"), Counts { comment: 0, code: 2 });
        assert_eq!(c("s = \"\\\"//\";\n"), Counts { comment: 0, code: 1 });
        assert_eq!(
            count("a: \"#1\"\nb: it's # c\n", Syntax::Hash),
            Counts { comment: 0, code: 2 }
        );
    }

    #[test]
    fn hash_syntax_ignores_c_markers_and_counts_shebang_as_code() {
        let t = "#!/usr/bin/env bash\n# c\nx=1 # t\n/* not a comment */\n";
        assert_eq!(count(t, Syntax::Hash), Counts { comment: 1, code: 3 });
        assert_eq!(
            count("#!x\n# c\n", Syntax::HashBlock),
            Counts { comment: 1, code: 1 }
        );
    }

    #[test]
    fn nix_takes_both_hash_and_block() {
        let t = "# c\n/* d\ne */\n{ x = 1; }\n";
        assert_eq!(count(t, Syntax::HashBlock), Counts { comment: 3, code: 1 });
    }

    #[test]
    fn rust_attribute_is_code() {
        assert_eq!(c("#[test]\nfn f() {}\n"), Counts { comment: 0, code: 2 });
    }

    #[test]
    fn empty_and_equal_pass_strictly_more_fails() {
        assert!(!Counts::default().over());
        assert!(!Counts { comment: 3, code: 3 }.over());
        assert!(Counts { comment: 4, code: 3 }.over());
    }

    #[test]
    fn syntax_by_extension() {
        assert_eq!(syntax_for("src/A.sol"), Some(Syntax::CStyle));
        assert_eq!(syntax_for("a/b.test.ts"), Some(Syntax::CStyle));
        assert_eq!(syntax_for("x.yml"), Some(Syntax::Hash));
        assert_eq!(syntax_for("flake.nix"), Some(Syntax::HashBlock));
        assert_eq!(syntax_for("README.md"), None);
        assert_eq!(syntax_for("dir.sol/noext"), None);
    }

    #[test]
    fn paths_split_on_whitespace_and_commas() {
        assert_eq!(parse_paths(" src\ttest\n,script,, "), ["src", "test", "script"]);
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
        std::fs::write(d.join("src/Over.sol"), "// a\n// b\n// c\nx;\ny;\n").unwrap();
        std::fs::write(d.join("src/Ok.sol"), "// a\nx;\n").unwrap();
        std::fs::write(d.join("src/notes.md"), "# all\n# comment\n").unwrap();
        std::fs::write(d.join("test/Untracked.sol"), "// a\n// b\nx;\n").unwrap();
        git(&["add", "src"]);
        d
    }

    #[test]
    fn offender_is_reported_with_both_counts_and_others_are_not() {
        let d = repo();
        let files = scan(&d, &["src".into(), "test".into()]).unwrap();
        assert_eq!(
            files,
            vec![
                ("src/Ok.sol".to_string(), Counts { comment: 1, code: 1 }),
                ("src/Over.sol".to_string(), Counts { comment: 3, code: 2 }),
            ]
        );
        let lines = report(&files);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains("1 of 2 files"), "{lines:?}");
        assert!(lines[2].ends_with("3       2  src/Over.sol"), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("Ok.sol")), "{lines:?}");
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
        let d = std::env::temp_dir().join(format!("rainix-static-commentcap-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let e = scan(&d, &["src".into()]).unwrap_err();
        assert!(e.contains("git ls-files failed"), "{e}");
    }
}
