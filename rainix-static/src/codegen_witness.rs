use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::UNIX_EPOCH;

pub(crate) const MANIFEST_PATH: &str = "script/codegen-manifest.txt";

pub(crate) const HOOKS: [&str; 4] = [
    "script/build-meta.sh",
    "script/Build.sol",
    "script/CopyArtifacts.sol",
    "script/build.sh",
];

const HEADER: &str = "\
# Committed files this repo's codegen hooks generate — one path per line.
#
# rainix-copy-artifacts re-runs the hooks and diffs, which proves the CONTENT is
# current but cannot see a generator that has stopped emitting a file: nothing
# is rewritten, so nothing differs and the job is green over a dead emitter.
# Every path listed here must be written on each run, so losing an emitter is a
# red job rather than silence.
#
# Add a path when the repo starts generating it; remove one only when the file
# genuinely stops being generated (and is deleted or hand-maintained from then
# on). Blank lines and # comments are ignored.
";

pub(crate) fn parse_manifest(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

pub(crate) fn render_manifest(paths: &BTreeSet<String>) -> String {
    let mut out = String::from(HEADER);
    for path in paths {
        out.push_str(path);
        out.push('\n');
    }
    out
}

pub(crate) fn offenders(
    listed: &BTreeSet<String>,
    written: &BTreeSet<String>,
    present: &BTreeSet<String>,
    manifest: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for path in listed.difference(written) {
        if present.contains(path) {
            out.push(format!(
                "ERROR: {manifest} declares {path} generated, but no codegen hook wrote it on \
                 this run. The committed file is left exactly as it was, so re-running the \
                 generators and diffing passes without checking it — its emitter is dead. \
                 Restore the emitter, or, if {path} is genuinely no longer generated, say so by \
                 removing the line (and the file, if nothing hand-maintains it)."
            ));
        } else {
            out.push(format!(
                "ERROR: {manifest} declares {path} generated, but no such file exists after \
                 running the codegen hooks. Either its emitter is dead, or the path in the \
                 manifest is wrong."
            ));
        }
    }
    out
}

fn tracked(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        // -z: git otherwise quotes awkward paths, which would not match the manifest.
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

pub(crate) fn mark(root: &Path, state: &Path) -> Result<usize, String> {
    let files = tracked(root)?;
    let mut map = serde_json::Map::new();
    for path in &files {
        let value = match mtime_nanos(root, path) {
            Some(nanos) => serde_json::Value::from(nanos),
            None => serde_json::Value::Null,
        };
        map.insert(path.clone(), value);
    }
    let doc = serde_json::json!({ "files": serde_json::Value::Object(map) });
    std::fs::write(state, doc.to_string())
        .map_err(|e| format!("failed to write {}: {e}", state.display()))?;
    Ok(files.len())
}

pub(crate) fn written_since(
    root: &Path,
    state: &Path,
) -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
    let text = std::fs::read_to_string(state).map_err(|e| {
        format!(
            "failed to read the mark state {}: {e} — `codegen-witness mark` must run before the \
             codegen steps, in the same job",
            state.display()
        )
    })?;
    let doc: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not valid JSON: {e}", state.display()))?;
    let files = doc
        .get("files")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format!("{} has no `files` object", state.display()))?;

    let mut written = BTreeSet::new();
    let mut present = BTreeSet::new();
    for (path, before) in files {
        let after = mtime_nanos(root, path);
        if after.is_some() {
            present.insert(path.clone());
            if after != before.as_u64() {
                written.insert(path.clone());
            }
        }
    }
    Ok((written, present))
}

pub(crate) fn run_mark(root: &Path, state: &Path) {
    match mark(root, state) {
        Err(e) => crate::fail(&format!("codegen-witness mark: {e}")),
        Ok(n) => println!(
            "codegen-witness: marked {n} tracked files in {}",
            root.display()
        ),
    }
}

pub(crate) fn run_verify(root: &Path, state: &Path, manifest_rel: &str) {
    let (written, present) = match written_since(root, state) {
        Ok(sets) => sets,
        Err(e) => crate::fail(&format!("codegen-witness verify: {e}")),
    };

    let manifest_path = root.join(manifest_rel);
    let listed = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => Some(parse_manifest(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => crate::fail(&format!(
            "codegen-witness verify: failed to read {}: {e}",
            manifest_path.display()
        )),
    };

    let mut written: BTreeSet<String> = written;
    written.remove(manifest_rel);

    let Some(listed) = listed else {
        let hooks: Vec<&str> = HOOKS
            .iter()
            .copied()
            .filter(|h| root.join(h).exists())
            .collect();
        if hooks.is_empty() {
            println!(
                "codegen-witness: clean — no codegen hook and no {manifest_rel}; nothing declared \
                 generated here"
            );
            return;
        }
        eprintln!(
            "::error::codegen-witness: this repo runs codegen ({}) but has no {manifest_rel}, so \
             the currency check cannot tell a generated file that is still emitted from one whose \
             emitter has died — the committed copy is correct either way. Commit a manifest \
             declaring the committed files the hooks generate. What they wrote on this run, as a \
             starting point:",
            hooks.join(", ")
        );
        eprintln!("{}", render_manifest(&written));
        std::process::exit(1);
    };

    let offenders = offenders(&listed, &written, &present, manifest_rel);
    let unlisted: Vec<&String> = written.difference(&listed).collect();

    if offenders.is_empty() {
        println!(
            "codegen-witness: clean — {} declared generated files, each written this run",
            listed.len()
        );
        if !unlisted.is_empty() {
            println!(
                "codegen-witness: note — written but not declared in {manifest_rel}, so nothing \
                 will notice if their emitters die:"
            );
            for path in unlisted {
                println!("  {path}");
            }
        }
        return;
    }

    for line in &offenders {
        eprintln!("::error::{line}");
    }
    eprintln!("What the codegen hooks actually wrote on this run:");
    eprintln!("{}", render_manifest(&written));
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn manifest_ignores_comments_and_blanks() {
        let text = "# a comment\n\nsrc/a.sol\n  src/b.sol  \n#src/c.sol\n";
        assert_eq!(parse_manifest(text), set(&["src/a.sol", "src/b.sol"]));
    }

    #[test]
    fn rendered_manifest_round_trips_sorted() {
        let paths = set(&["src/b.sol", "src/a.sol"]);
        let rendered = render_manifest(&paths);
        assert!(rendered.starts_with('#'));
        let body: Vec<&str> = rendered
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .collect();
        assert_eq!(body, vec!["src/a.sol", "src/b.sol"]);
        assert_eq!(parse_manifest(&rendered), paths);
    }

    #[test]
    fn empty_manifest_renders_to_header_only() {
        let rendered = render_manifest(&BTreeSet::new());
        assert!(parse_manifest(&rendered).is_empty());
    }

    #[test]
    fn declared_but_unwritten_is_an_offence() {
        let listed = set(&["src/lib/LibReleasedSuites.sol", "src/generated/A.sol"]);
        let written = set(&["src/generated/A.sol"]);
        let present = set(&["src/lib/LibReleasedSuites.sol", "src/generated/A.sol"]);
        let off = offenders(&listed, &written, &present, MANIFEST_PATH);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("src/lib/LibReleasedSuites.sol"));
        assert!(off[0].contains("emitter is dead"));
    }

    #[test]
    fn declared_but_missing_from_disk_says_so() {
        let listed = set(&["src/generated/Gone.sol"]);
        let off = offenders(&listed, &BTreeSet::new(), &BTreeSet::new(), MANIFEST_PATH);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("no such file exists"));
    }

    #[test]
    fn every_declared_path_written_is_clean() {
        let all = set(&["src/a.sol", "src/b.sol"]);
        assert!(offenders(&all, &all, &all, MANIFEST_PATH).is_empty());
    }

    #[test]
    fn a_written_but_undeclared_path_is_not_an_offence() {
        let listed = set(&["src/a.sol"]);
        let written = set(&["src/a.sol", "soldeer.lock"]);
        assert!(offenders(&listed, &written, &written, MANIFEST_PATH).is_empty());
    }

    #[test]
    fn an_empty_manifest_declares_nothing_and_is_clean() {
        let written = set(&["src/a.sol"]);
        assert!(offenders(&BTreeSet::new(), &written, &written, MANIFEST_PATH).is_empty());
    }
}
