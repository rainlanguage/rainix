//! `relative-src-imports` — fail if a published `src/` Solidity file imports
//! through a path rooted at the repo.

use std::path::{Path, PathBuf};
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
/// Scans the tracked `src/**/*.sol` files (every `src/**/*.sol` outside a git
/// repo). Returns the offender report lines; empty means clean.
pub(crate) fn check(dir: &Path) -> Vec<String> {
    let mut offenders = Vec::new();
    for file in src_sol_files(dir) {
        let Ok(text) = std::fs::read_to_string(dir.join(&file)) else {
            continue;
        };
        for path in import_paths(&strip_comments(&text)) {
            if is_repo_rooted(dir, &path) {
                offenders.push(format!("  {}: \"{path}\"", file.display()));
            }
        }
    }
    if !offenders.is_empty() {
        offenders.insert(
            0,
            "Repo-rooted imports in src/ — they resolve against the consumer's root downstream; import relatively:"
                .to_string(),
        );
    }
    offenders
}

/// Whether `path`'s first segment names an entry of the repo root.
fn is_repo_rooted(dir: &Path, path: &str) -> bool {
    if path.starts_with("./") || path.starts_with("../") {
        return false;
    }
    match path.split('/').next() {
        Some(first) if !first.is_empty() => dir.join(first).exists(),
        _ => false,
    }
}

/// The tracked `src/**/*.sol` files, relative to `dir`; every such file on
/// disk when `dir` is not a git repo.
fn src_sol_files(dir: &Path) -> Vec<PathBuf> {
    let tracked = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "-z", "--", "src"])
        .output();
    if let Ok(out) = tracked {
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout)
                .split('\0')
                .filter(|p| p.ends_with(".sol"))
                .map(PathBuf::from)
                .collect();
        }
    }
    let mut files = Vec::new();
    walk(dir, Path::new("src"), &mut files);
    files
}

fn walk(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root.join(rel)) else {
        return;
    };
    for entry in entries.flatten() {
        let child = rel.join(entry.file_name());
        if entry.path().is_dir() {
            walk(root, &child, out);
        } else if child.extension().is_some_and(|e| e == "sol") {
            out.push(child);
        }
    }
}

/// `text` with `//` and `/* */` comments blanked, string literals kept.
fn strip_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                let quote = b[i];
                let start = i;
                i += 1;
                while i < b.len() && b[i] != quote {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(b.len());
                out.push_str(&text[start..i]);
            }
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
                i = (i + 2).min(b.len());
                out.push(' ');
            }
            _ => {
                let ch = text[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// The path of every `import` directive: the first string literal between the
/// `import` keyword and its `;`, which covers `import "p";`,
/// `import "p" as X;`, `import * as X from "p";` and `import {A} from "p";`.
fn import_paths(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut paths = Vec::new();
    let mut from = 0;
    while let Some(off) = text[from..].find("import") {
        let at = from + off;
        from = at + "import".len();
        let before_ok = at == 0 || !is_ident(b[at - 1]);
        let after_ok = b.get(from).map_or(true, |c| !is_ident(*c));
        if !(before_ok && after_ok) {
            continue;
        }
        let end = text[from..].find(';').map_or(text.len(), |e| from + e);
        let directive = &text[from..end];
        if let Some(q) = directive.find(['"', '\'']) {
            let quote = directive.as_bytes()[q] as char;
            if let Some(len) = directive[q + 1..].find(quote) {
                paths.push(directive[q + 1..q + 1 + len].to_string());
            }
        }
        from = end;
    }
    paths
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_repo() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-relimports-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(d.join("src/lib")).unwrap();
        std::fs::create_dir_all(d.join("test")).unwrap();
        std::fs::create_dir_all(d.join("dependencies")).unwrap();
        d
    }

    fn write(d: &Path, rel: &str, body: &str) {
        std::fs::write(d.join(rel), body).unwrap();
    }

    #[test]
    fn relative_and_dependency_imports_pass() {
        let d = tmp_repo();
        write(
            &d,
            "src/lib/A.sol",
            "import {B} from \"../interface/B.sol\";\nimport \"./C.sol\";\nimport {Test} from \"forge-std-1.17.0/src/Test.sol\";\n",
        );
        assert!(check(&d).is_empty(), "{:?}", check(&d));
    }

    #[test]
    fn src_rooted_import_fails_and_is_named() {
        let d = tmp_repo();
        write(&d, "src/lib/A.sol", "import {B} from \"src/interface/B.sol\";\n");
        let off = check(&d);
        assert_eq!(off.len(), 2, "{off:?}");
        assert!(off[1].contains("src/lib/A.sol") && off[1].contains("\"src/interface/B.sol\""), "{off:?}");
    }

    #[test]
    fn every_root_entry_counts_not_only_src() {
        let d = tmp_repo();
        write(
            &d,
            "src/lib/A.sol",
            "import \"test/X.sol\";\nimport * as D from 'dependencies/forge-std-1.17.0/src/Test.sol';\n",
        );
        let off = check(&d);
        assert_eq!(off.len(), 3, "{off:?}");
    }

    #[test]
    fn every_import_form_is_read() {
        let d = tmp_repo();
        write(
            &d,
            "src/lib/A.sol",
            "import \"src/a.sol\";\nimport \"src/b.sol\" as B;\nimport * as C from \"src/c.sol\";\nimport {\n    D,\n    E\n} from \"src/d.sol\";\n",
        );
        assert_eq!(check(&d).len(), 5, "{:?}", check(&d));
    }

    #[test]
    fn comments_and_identifiers_are_not_imports() {
        let d = tmp_repo();
        write(
            &d,
            "src/lib/A.sol",
            "// import \"src/a.sol\";\n/* import {X} from \"src/b.sol\"; */\nfunction importer() {}\nuint256 reimport;\nstring constant S = \"import src/c.sol\";\n",
        );
        assert!(check(&d).is_empty(), "{:?}", check(&d));
    }

    #[test]
    fn files_outside_src_are_not_scanned() {
        let d = tmp_repo();
        write(&d, "test/T.sol", "import {B} from \"src/interface/B.sol\";\n");
        assert!(check(&d).is_empty());
    }

    #[test]
    fn untracked_src_file_is_skipped_in_a_git_repo() {
        let d = tmp_repo();
        assert!(Command::new("git").arg("-C").arg(&d).args(["init", "-q"]).status().unwrap().success());
        write(&d, "src/lib/A.sol", "import {B} from \"src/interface/B.sol\";\n");
        assert!(check(&d).is_empty());
        assert!(Command::new("git").arg("-C").arg(&d).args(["add", "src/lib/A.sol"]).status().unwrap().success());
        assert_eq!(check(&d).len(), 2);
    }
}
