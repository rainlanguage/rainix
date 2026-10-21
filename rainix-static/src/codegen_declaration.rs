use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::time::UNIX_EPOCH;

/// Line prefix a codegen hook prints to declare a path. `forge script` indents
/// `console.log` output under `== Logs ==`, so lines are matched trimmed.
pub(crate) const SENTINEL: &str = "rainix-codegen ";

#[derive(Default, PartialEq, Eq, Debug)]
pub(crate) struct Declaration {
    pub(crate) owns: BTreeSet<String>,
    pub(crate) malformed: Vec<String>,
}

pub(crate) fn parse(log: &str) -> Declaration {
    let mut out = Declaration::default();
    for line in log.lines() {
        let Some(rest) = line.trim().strip_prefix(SENTINEL) else {
            continue;
        };
        match rest.split_once(char::is_whitespace) {
            Some(("owns", path)) if is_repo_relative(path.trim()) => {
                out.owns.insert(path.trim().to_string());
            }
            // Fail-closed: a verb this check does not know is a declaration it
            // is silently not making. That includes `wrote`: what was written is
            // observed here, never taken from the generator.
            _ => out.malformed.push(line.trim().to_string()),
        }
    }
    out
}

// Absolute or `..` paths would let `root.join` check a file outside the repo;
// `.` and empty segments would key one file under two spellings.
fn is_repo_relative(path: &str) -> bool {
    path.split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

pub(crate) type Mtimes = BTreeMap<String, Option<u64>>;

/// A path was written when it exists now with an mtime other than the one
/// marked. A path not marked at all (untracked before the run) counts as
/// written once it exists.
pub(crate) fn written(path: &str, before: &Mtimes, after: Option<u64>) -> bool {
    after.is_some() && before.get(path).copied().flatten() != after
}

pub(crate) fn offences(
    owns: &BTreeSet<String>,
    written: &BTreeSet<String>,
    present: &BTreeSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for path in owns.difference(written) {
        if present.contains(path) {
            out.push(format!(
                "the codegen hooks declare {path} generated, but nothing wrote it on this run. \
                 The committed copy is left exactly as it was, so regenerating and diffing \
                 passes without ever checking it — its emitter is dead. Restore the emitter, or \
                 stop declaring the path if it is genuinely no longer generated."
            ));
        } else {
            out.push(format!(
                "the codegen hooks declare {path} generated, but nothing wrote it and no such \
                 file exists after the run."
            ));
        }
    }
    out
}

fn tracked(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| format!("failed to run git ls-files: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files in {} failed: {}",
            root.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn mtime_nanos(root: &Path, path: &str) -> Option<u64> {
    let meta = std::fs::metadata(root.join(path)).ok()?;
    let since = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(since.as_nanos()).ok()
}

pub(crate) fn run_mark(root: &Path, state: &Path) {
    let files =
        tracked(root).unwrap_or_else(|e| crate::fail(&format!("codegen-declaration mark: {e}")));
    let marked: Mtimes = files
        .iter()
        .map(|path| (path.clone(), mtime_nanos(root, path)))
        .collect();
    let text = serde_json::to_string(&marked).expect("a map of strings to integers serialises");
    std::fs::write(state, text).unwrap_or_else(|e| {
        crate::fail(&format!(
            "codegen-declaration mark: failed to write {}: {e}",
            state.display()
        ))
    });
    println!("codegen-declaration: marked {} tracked files", marked.len());
}

fn read_marked(state: &Path) -> Mtimes {
    let text = std::fs::read_to_string(state).unwrap_or_else(|e| {
        crate::fail(&format!(
            "codegen-declaration verify: failed to read {}: {e} — `mark` must run before the \
             codegen hooks, in the same job",
            state.display()
        ))
    });
    serde_json::from_str(&text).unwrap_or_else(|e| {
        crate::fail(&format!(
            "codegen-declaration verify: {} is not a mark state: {e}",
            state.display()
        ))
    })
}

pub(crate) fn run_verify(root: &Path, state: &Path, log: &Path) {
    let before = read_marked(state);
    let text = match std::fs::read_to_string(log) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => crate::fail(&format!(
            "codegen-declaration: failed to read {}: {e}",
            log.display()
        )),
    };
    let declaration = parse(&text);

    if !declaration.malformed.is_empty() {
        for line in &declaration.malformed {
            eprintln!("::error::codegen-declaration: unreadable declaration: {line}");
        }
        std::process::exit(1);
    }

    if declaration.owns.is_empty() {
        println!(
            "codegen-declaration: clean — this repo's codegen declares no generated paths, so \
             nothing here can tell a live emitter from one that has died"
        );
        return;
    }

    let mut present = BTreeSet::new();
    let mut wrote = BTreeSet::new();
    for path in declaration.owns.iter().chain(before.keys()) {
        let after = mtime_nanos(root, path);
        if after.is_some() {
            present.insert(path.clone());
        }
        if written(path, &before, after) {
            wrote.insert(path.clone());
        }
    }
    let offences = offences(&declaration.owns, &wrote, &present);

    if !offences.is_empty() {
        for line in &offences {
            eprintln!("::error::codegen-declaration: {line}");
        }
        std::process::exit(1);
    }

    println!(
        "codegen-declaration: clean — {} declared generated paths, each written this run",
        declaration.owns.len()
    );
    for path in wrote.difference(&declaration.owns) {
        println!("codegen-declaration: note — {path} was written but not declared, so nothing will notice if its emitter dies");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn forge_indented_lines_parse() {
        let d = parse("== Logs ==\n  rainix-codegen owns src/a.sol\n");
        assert_eq!(d.owns, set(&["src/a.sol"]));
        assert!(d.malformed.is_empty());
    }

    #[test]
    fn a_line_that_merely_mentions_the_sentinel_is_not_a_declaration() {
        let d = parse("error: expected `rainix-codegen owns src/a.sol`\n");
        assert_eq!(d, Declaration::default());
    }

    #[test]
    fn repeated_declarations_of_one_path_are_one_path() {
        let d = parse("rainix-codegen owns src/a.sol\nrainix-codegen owns src/a.sol\n");
        assert_eq!(d.owns, set(&["src/a.sol"]));
    }

    #[test]
    fn a_generator_cannot_report_its_own_writes() {
        let d = parse("rainix-codegen wrote src/a.sol\n");
        assert_eq!(d.malformed, vec!["rainix-codegen wrote src/a.sol"]);
    }

    #[test]
    fn an_unknown_verb_is_malformed_rather_than_ignored() {
        let d = parse("rainix-codegen skipped src/a.sol\n");
        assert_eq!(d.malformed, vec!["rainix-codegen skipped src/a.sol"]);
    }

    #[test]
    fn a_verb_with_no_path_is_malformed() {
        let d = parse("rainix-codegen owns\nrainix-codegen owns   \n");
        assert_eq!(d.malformed.len(), 2);
        assert!(d.owns.is_empty());
    }

    #[test]
    fn a_path_that_is_not_plainly_inside_the_repo_is_malformed() {
        for path in [
            "/etc/passwd",
            "../outside.sol",
            "src/../../outside.sol",
            "./src/a.sol",
            "src/./a.sol",
            "src//a.sol",
            "src/generated/",
        ] {
            let d = parse(&format!("rainix-codegen owns {path}\n"));
            assert_eq!(d.malformed.len(), 1, "{path}");
            assert!(d.owns.is_empty(), "{path}");
        }
    }

    #[test]
    fn a_moved_mtime_is_a_write() {
        let before: Mtimes = [("src/a.sol".to_string(), Some(1))].into();
        assert!(written("src/a.sol", &before, Some(2)));
    }

    #[test]
    fn an_unmoved_mtime_is_not_a_write() {
        let before: Mtimes = [("src/a.sol".to_string(), Some(1))].into();
        assert!(!written("src/a.sol", &before, Some(1)));
    }

    #[test]
    fn a_file_absent_after_the_run_was_not_written() {
        let before: Mtimes = [("src/a.sol".to_string(), Some(1))].into();
        assert!(!written("src/a.sol", &before, None));
    }

    #[test]
    fn a_file_created_by_the_run_was_written() {
        assert!(written("src/new.sol", &Mtimes::new(), Some(1)));
        let before: Mtimes = [("src/new.sol".to_string(), None)].into();
        assert!(written("src/new.sol", &before, Some(1)));
    }

    #[test]
    fn a_declared_path_nothing_wrote_is_an_offence_though_it_is_on_disk() {
        let owns = set(&["src/lib/LibReleasedSuites.sol", "src/generated/A.sol"]);
        let wrote = set(&["src/generated/A.sol"]);
        let off = offences(&owns, &wrote, &owns);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("src/lib/LibReleasedSuites.sol"));
        assert!(off[0].contains("emitter is dead"));
    }

    #[test]
    fn a_declared_path_that_is_not_on_disk_says_so_instead() {
        let owns = set(&["src/generated/Gone.sol"]);
        let off = offences(&owns, &BTreeSet::new(), &BTreeSet::new());
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("no such file exists"));
    }

    #[test]
    fn every_declared_path_written_is_clean() {
        let all = set(&["src/a.sol", "src/b.sol"]);
        assert!(offences(&all, &all, &all).is_empty());
    }

    #[test]
    fn a_written_but_undeclared_path_is_not_an_offence() {
        let owns = set(&["src/a.sol"]);
        let wrote = set(&["src/a.sol", "soldeer.lock"]);
        assert!(offences(&owns, &wrote, &wrote).is_empty());
    }

    #[test]
    fn declaring_nothing_is_not_an_offence() {
        let wrote = set(&["src/a.sol"]);
        assert!(offences(&BTreeSet::new(), &wrote, &wrote).is_empty());
    }
}
