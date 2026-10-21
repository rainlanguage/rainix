//! `relative-src-imports` — fail if a published `src/` Solidity file imports
//! through a path rooted at the repo.

use std::path::Path;
use std::process::Command;

/// A soldeer package ships its `src/` without `remappings.txt`, and a consumer
/// compiles it from `dependencies/<package>-<version>/`. An import inside
/// `src/` whose path begins with one of the repo's own top-level entries
/// (`src/…`, `test/…`, `dependencies/…`) compiles here but resolves against
/// the CONSUMER's root downstream, so it breaks every consumer and nothing
/// local notices. Imports within `src/` are relative (`./`, `../`); a
/// dependency import names its versioned package (`forge-std-1.17.0/…`),
/// which is no entry of the repo root and passes.
///
/// The root's entries are read from disk, not the index, because
/// `dependencies/` is gitignored: run it after `forge soldeer install`.
///
/// Scans the tracked `src/**/*.sol` files. Returns the offender report lines,
/// empty meaning clean; `Err` when git fails or a tracked file is unreadable.
pub(crate) fn check(dir: &Path) -> Result<Vec<String>, String> {
    let mut offenders = Vec::new();
    for file in src_sol_files(dir)? {
        let text = std::fs::read_to_string(dir.join(&file))
            .map_err(|e| format!("cannot read {file}: {e}"))?;
        for path in import_paths(&text) {
            if is_repo_rooted(dir, &path) {
                offenders.push(format!("  {file}: \"{path}\""));
            }
        }
    }
    if !offenders.is_empty() {
        offenders.insert(
            0,
            "Repo-rooted imports in src/ resolve against the consumer's root downstream; import relatively:"
                .to_string(),
        );
    }
    Ok(offenders)
}

/// Whether `path`'s first segment names an entry of the repo root.
fn is_repo_rooted(dir: &Path, path: &str) -> bool {
    let first = path.split('/').next().unwrap_or("");
    !matches!(first, "" | "." | "..") && dir.join(first).symlink_metadata().is_ok()
}

/// The tracked `src/**/*.sol` files, relative to `dir`.
fn src_sol_files(dir: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "-z", "--", "src"])
        .output()
        .map_err(|e| format!("git ls-files failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out
        .stdout
        .split(|&c| c == 0)
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .filter(|n| n.ends_with(".sol"))
        .collect())
}

/// The path of every `import` directive: the first string literal after the
/// `import` keyword and before its `;`, which covers `import "p";`,
/// `import "p" as X;`, `import * as X from "p";` and `import {A} from "p";`.
/// Comments are skipped and string literals are read whole, so neither can
/// open a directive.
fn import_paths(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut paths = Vec::new();
    let mut in_import = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
            }
            quote @ (b'"' | b'\'') => {
                let start = i + 1;
                i = start;
                while i < b.len() && b[i] != quote {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                if in_import && i < b.len() {
                    paths.push(text[start..i].to_string());
                    in_import = false;
                }
                i += 1;
            }
            b';' => {
                in_import = false;
                i += 1;
            }
            c if is_ident(c) => {
                let start = i;
                while i < b.len() && is_ident(b[i]) {
                    i += 1;
                }
                if &text[start..i] == "import" {
                    in_import = true;
                }
            }
            _ => i += 1,
        }
    }
    paths
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn git(d: &Path, args: &[&str]) {
        assert!(Command::new("git")
            .arg("-C")
            .arg(d)
            .args(args)
            .status()
            .unwrap()
            .success());
    }

    /// A git repo with the root entries a soldeer consumer has.
    fn tmp_repo() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-relimports-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(d.join("src/lib")).unwrap();
        std::fs::create_dir_all(d.join("test")).unwrap();
        std::fs::create_dir_all(d.join("dependencies/forge-std-1.17.0")).unwrap();
        git(&d, &["init", "-q"]);
        d
    }

    /// Write `body` to `rel` and track it.
    fn add(d: &Path, rel: &str, body: &str) {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
        git(d, &["add", rel]);
    }

    fn offenders(d: &Path) -> Vec<String> {
        check(d).unwrap()
    }

    #[test]
    fn relative_and_dependency_imports_pass() {
        let d = tmp_repo();
        add(
            &d,
            "src/lib/A.sol",
            "import {B} from \"../interface/B.sol\";\nimport \"./C.sol\";\nimport {Test} from \"forge-std-1.17.0/src/Test.sol\";\n",
        );
        assert_eq!(offenders(&d), Vec::<String>::new());
    }

    #[test]
    fn src_rooted_import_fails_and_is_named() {
        let d = tmp_repo();
        add(
            &d,
            "src/lib/A.sol",
            "import {B} from \"src/interface/B.sol\";\n",
        );
        assert_eq!(
            offenders(&d),
            [
                "Repo-rooted imports in src/ resolve against the consumer's root downstream; import relatively:",
                "  src/lib/A.sol: \"src/interface/B.sol\"",
            ]
        );
    }

    #[test]
    fn every_root_entry_counts_not_only_src() {
        let d = tmp_repo();
        std::fs::write(d.join("foundry.toml"), "").unwrap();
        add(
            &d,
            "src/A.sol",
            "import \"test/X.sol\";\nimport * as D from 'dependencies/forge-std-1.17.0/src/Test.sol';\nimport \"foundry.toml/x.sol\";\n",
        );
        assert_eq!(
            offenders(&d)[1..],
            [
                "  src/A.sol: \"test/X.sol\"",
                "  src/A.sol: \"dependencies/forge-std-1.17.0/src/Test.sol\"",
                "  src/A.sol: \"foundry.toml/x.sol\"",
            ]
        );
    }

    #[test]
    fn a_first_segment_is_rooted_only_when_the_root_has_it() {
        let d = tmp_repo();
        add(&d, "src/A.sol", "import \"lib/X.sol\";\n");
        assert_eq!(offenders(&d), Vec::<String>::new());
        std::fs::create_dir(d.join("lib")).unwrap();
        assert_eq!(offenders(&d).len(), 2);
    }

    #[test]
    fn a_dangling_symlink_is_a_root_entry() {
        let d = tmp_repo();
        std::os::unix::fs::symlink(d.join("missing"), d.join("lib")).unwrap();
        add(&d, "src/A.sol", "import \"lib/X.sol\";\n");
        assert_eq!(offenders(&d).len(), 2);
    }

    #[test]
    fn dot_segments_and_empty_paths_are_not_rooted() {
        let d = tmp_repo();
        add(
            &d,
            "src/A.sol",
            "import \".\";\nimport \"..\";\nimport \"/src/X.sol\";\nimport \"\";\n",
        );
        assert_eq!(offenders(&d), Vec::<String>::new());
    }

    #[test]
    fn every_import_form_is_read() {
        let d = tmp_repo();
        add(
            &d,
            "src/lib/A.sol",
            "import \"src/a.sol\";\nimport \"src/b.sol\" as B;\nimport * as C from \"src/c.sol\";\nimport {\n    D,\n    E\n} from \"src/d.sol\";\nimport{F}from'src/e.sol';",
        );
        assert_eq!(
            offenders(&d)[1..],
            [
                "  src/lib/A.sol: \"src/a.sol\"",
                "  src/lib/A.sol: \"src/b.sol\"",
                "  src/lib/A.sol: \"src/c.sol\"",
                "  src/lib/A.sol: \"src/d.sol\"",
                "  src/lib/A.sol: \"src/e.sol\"",
            ]
        );
    }

    #[test]
    fn only_the_first_literal_of_a_directive_is_its_path() {
        assert_eq!(
            import_paths("import \"./a.sol\" as \"src/x\";\n\"src/y\";"),
            ["./a.sol"]
        );
    }

    #[test]
    fn a_semicolon_ends_a_directive_with_no_path() {
        assert_eq!(
            import_paths("import;\n\"src/x.sol\";"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn comments_and_identifiers_are_not_imports() {
        let d = tmp_repo();
        add(
            &d,
            "src/lib/A.sol",
            "// import \"src/a.sol\";\n/* import {X} from \"src/b.sol\"; */\nfunction importer() {}\nuint256 reimport;\nuint256 import2 = \"src/f.sol\";\n",
        );
        assert_eq!(offenders(&d), Vec::<String>::new());
    }

    #[test]
    fn string_literals_are_not_imports() {
        assert_eq!(
            import_paths(
                "string constant S = \"import 'src/c.sol';\";\nstring constant T = 'import \"src/d.sol\";';\n"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_escaped_quote_does_not_end_a_literal() {
        assert_eq!(
            import_paths("string s = \"\\\" import 'src/a.sol';\";\nimport \"./b\\\"c.sol\";"),
            ["./b\\\"c.sol"]
        );
    }

    #[test]
    fn a_comment_ends_where_it_closes() {
        assert_eq!(
            import_paths("// x\nimport \"a\";/* import \"b\"; */import \"c\";/**/import \"d\";"),
            ["a", "c", "d"]
        );
    }

    #[test]
    fn a_block_comment_opener_does_not_close_it() {
        assert_eq!(import_paths("/*/ import \"src/a\"; */import \"b\";"), ["b"]);
    }

    #[test]
    fn code_resumes_right_after_a_block_comment_closes() {
        assert_eq!(import_paths("/* a *//* b */import \"a\";"), ["a"]);
    }

    #[test]
    fn identifiers_containing_import_are_not_the_keyword() {
        assert_eq!(
            import_paths(
                "uint256 reimport = \"src/a\";\nuint256 $import = \"src/b\";\nuint256 _import = \"src/c\";\n"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_comment_inside_a_directive_holds_no_path() {
        assert_eq!(
            import_paths("import /* \"src/x\" */ {A} // \"src/y\"\n from \"./a.sol\";"),
            ["./a.sol"]
        );
    }

    #[test]
    fn a_slash_alone_is_not_a_comment() {
        assert_eq!(import_paths("x = 1 / 2; import \"a\";"), ["a"]);
    }

    #[test]
    fn unterminated_input_yields_no_partial_path() {
        assert_eq!(import_paths("import \"src/x.sol"), Vec::<String>::new());
        assert_eq!(
            import_paths("import /* \"src/x.sol\";"),
            Vec::<String>::new()
        );
        assert_eq!(import_paths("import \"a\\"), Vec::<String>::new());
    }

    #[test]
    fn non_ascii_text_is_read() {
        assert_eq!(
            import_paths("// é\nstring s = unicode\"é\";\nimport \"./é.sol\";"),
            ["./é.sol"]
        );
    }

    #[test]
    fn files_outside_src_and_non_sol_files_are_not_scanned() {
        let d = tmp_repo();
        add(
            &d,
            "test/T.sol",
            "import {B} from \"src/interface/B.sol\";\n",
        );
        add(
            &d,
            "srcx/T.sol",
            "import {B} from \"src/interface/B.sol\";\n",
        );
        add(
            &d,
            "src/README.md",
            "import {B} from \"src/interface/B.sol\";\n",
        );
        assert_eq!(offenders(&d), Vec::<String>::new());
    }

    #[test]
    fn nested_src_files_are_scanned() {
        let d = tmp_repo();
        add(&d, "src/a/b/c/D.sol", "import \"src/x.sol\";\n");
        assert_eq!(offenders(&d)[1..], ["  src/a/b/c/D.sol: \"src/x.sol\""]);
    }

    #[test]
    fn untracked_src_file_is_skipped() {
        let d = tmp_repo();
        std::fs::write(d.join("src/lib/A.sol"), "import \"src/x.sol\";\n").unwrap();
        assert_eq!(offenders(&d), Vec::<String>::new());
        git(&d, &["add", "src/lib/A.sol"]);
        assert_eq!(offenders(&d).len(), 2);
    }

    #[test]
    fn a_repo_with_no_src_is_clean() {
        let d = tmp_repo();
        std::fs::remove_dir_all(d.join("src")).unwrap();
        assert_eq!(offenders(&d), Vec::<String>::new());
    }

    #[test]
    fn a_tracked_file_that_cannot_be_read_is_an_error() {
        let d = tmp_repo();
        add(&d, "src/A.sol", "import \"./x.sol\";\n");
        std::fs::remove_file(d.join("src/A.sol")).unwrap();
        let e = check(&d).unwrap_err();
        assert!(e.starts_with("cannot read src/A.sol: "), "{e}");
    }

    #[test]
    fn outside_a_git_repo_is_an_error() {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-relimports-nogit-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(d.join("src")).unwrap();
        let e = check(&d).unwrap_err();
        assert!(e.starts_with("git ls-files failed: "), "{e}");
    }
}
