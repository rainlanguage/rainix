//! Witness that the repo's codegen hooks still EMIT each committed generated
//! file, which re-running them and diffing cannot see.
//!
//! `rainix-copy-artifacts` currency-checks committed generated sources by
//! re-running every consumer codegen hook and then `git diff --exit-code`. That
//! method has one blind spot, and it is total: a generator that has STOPPED
//! emitting a file writes nothing, so the committed copy — already correct —
//! is left exactly as it is, nothing differs, and the job is green over a dead
//! emitter (rainlanguage/rain.factory.deploy#35, reproduced with a control: a
//! marker appended to the generated file survived the generator run once its
//! one emitting call was removed, with `git diff` clean throughout).
//!
//! The blind spot is structural. The generator's output is the check's only
//! oracle for what the committed files should contain, so a file the generator
//! never writes has no oracle at all. Seeing it requires an INDEPENDENT
//! statement of which committed files are generated, which is what
//! `script/codegen-manifest.txt` is. The check then has two halves: the diff
//! says the content is current, and this says something actually wrote it.
//!
//! ## What is witnessed
//!
//! `mark` records the mtime of every git-tracked file before the first codegen
//! hook runs; `verify` re-stats them after the last one and calls a file
//! WRITTEN when it exists now and its mtime moved. That is the property the
//! defect is about — `vm.writeFile` and friends rewrite unconditionally, so a
//! live emitter always moves the mtime even when the bytes are identical,
//! which is exactly the case the diff cannot distinguish from a dead one.
//!
//! Scope is git-tracked files: the currency check is about COMMITTED generated
//! sources, and restricting to them keeps `out/`, `cache/`, `broadcast/` and
//! `dependencies/` out of the witness whether or not a repo ignores them
//! properly.
//!
//! ## Why listed-must-be-written, and not set equality
//!
//! Set equality (every written file must be listed) would keep the manifest
//! self-maintaining, but it couples an org-wide gate to every incidental write
//! inside the window — `forge build` is in there, and the day it starts
//! rewriting a lock file it reddens every consumer at once. The claim worth
//! making is the one the defect is about: a path this repo DECLARES as
//! generated must have been written. Files written but not listed are printed
//! as a note instead, so a newly generated file is discoverable without being
//! able to break anyone.
//!
//! The residual gap is the mirror of that choice, and is named here rather than
//! papered over: a generated file nobody has listed yet is not protected. So is
//! a brand-new generated file that is never committed — `git diff --exit-code`
//! does not see untracked files either, which is a separate hole in the same
//! job.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::UNIX_EPOCH;

/// The committed declaration, beside the hooks it describes (`script/Build.sol`,
/// `script/build.sh`, ...). One fixed path: the workflow is consumed at `@main`
/// by every Rain repo and cannot go looking for a per-repo convention.
pub(crate) const MANIFEST_PATH: &str = "script/codegen-manifest.txt";

/// The consumer-supplied codegen hooks `rainix-copy-artifacts` runs. Presence of
/// any one of them is what makes a manifest mandatory: a repo with no codegen
/// has nothing to declare and must not be asked to declare it.
pub(crate) const HOOKS: [&str; 4] = [
    "script/build-meta.sh",
    "script/Build.sol",
    "script/CopyArtifacts.sol",
    "script/build.sh",
];

/// Header of a rendered manifest. Present so the file explains itself to
/// whoever opens it in a diff, and parsed back out as a comment.
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

/// Paths a manifest lists. Blank lines and `#` comments are ignored, and each
/// path is trimmed, so the file can carry its own explanation.
pub(crate) fn parse_manifest(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// A manifest file's content for `paths`: the header, then one path per line in
/// sorted order. What `verify` prints for a consumer to commit verbatim.
pub(crate) fn render_manifest(paths: &BTreeSet<String>) -> String {
    let mut out = String::from(HEADER);
    for path in paths {
        out.push_str(path);
        out.push('\n');
    }
    out
}

/// Offenders for a manifest that exists: every declared path nothing wrote.
///
/// Declared-but-absent is reported as the same offence — a path that is not
/// even on disk was certainly not written, and saying "no hook wrote it" of a
/// file that does not exist would send the reader looking for the wrong thing.
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

/// Repo-relative paths git tracks under `root`.
fn tracked(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        // -z: a path may contain anything but NUL, and git otherwise quotes the
        // awkward ones, which would not match the manifest.
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

/// Nanoseconds-since-epoch mtime of `root/path`, or `None` when it is not there
/// (git tracks a path the worktree may not currently hold) or its mtime cannot
/// be read at all.
fn mtime_nanos(root: &Path, path: &str) -> Option<u64> {
    let meta = std::fs::metadata(root.join(path)).ok()?;
    let since = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(since.as_nanos()).ok()
}

/// Record every tracked file's mtime into `state`, to be compared after the
/// codegen hooks have run. Returns how many files were marked.
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

/// The marked files that were WRITTEN since `mark`, and those that exist now.
///
/// Written means present now with a different mtime: a rewrite with identical
/// bytes still moves it, which is the whole point, while a file the hooks
/// DELETED is not a write (and `git diff` catches a deletion on its own).
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

/// `mark` as a subcommand: record and report, or fail loud.
pub(crate) fn run_mark(root: &Path, state: &Path) {
    match mark(root, state) {
        Err(e) => crate::fail(&format!("codegen-witness mark: {e}")),
        Ok(n) => println!(
            "codegen-witness: marked {n} tracked files in {}",
            root.display()
        ),
    }
}

/// `verify` as a subcommand: the manifest's declarations against what the hooks
/// actually wrote.
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

    // The manifest itself is written by hand and by this command's output, never
    // by a codegen hook, so it is never part of its own witness.
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
        // A note, never a failure: see the module doc on why an unlisted write
        // must not be able to redden an org-wide job.
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
        // Sorted, so the committed file does not churn on set ordering.
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

    // THE defect: the file is on disk and byte-identical to what the generator
    // would have written, so every content check is happy — and nothing wrote
    // it.
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

    // The deliberate asymmetry: an incidental write inside the window (forge
    // touching a lock file, say) must never redden a job every Rain repo runs.
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
