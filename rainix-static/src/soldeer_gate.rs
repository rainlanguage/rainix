//! `soldeer-gate` — Soldeer next-version content gate.
//!
//! Compares the normalized content of what `forge soldeer push --dry-run` would
//! upload against the latest published revision, enforces the next-version
//! ahead-invariant, and emits `changed` / `version` / `next`. Runs inside
//! sol-shell, so `forge` and `curl` are on PATH.

use crate::fail;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A file entry pulled from a package zip: (name, bytes).
type Entry = (String, Vec<u8>);

/// A foundry.toml `[package].version` line starts with `version`, then optional
/// spaces/tabs, then `=`. Matches the old `^version[[:space:]]*=` sed anchor.
fn is_version_line(line: &str) -> bool {
    match line.strip_prefix("version") {
        Some(rest) => rest.trim_start_matches([' ', '\t']).starts_with('='),
        None => false,
    }
}

/// Blank foundry.toml's version line to `version = "0.0.0"` so a bump alone is
/// never seen as a content change. Every other line is preserved verbatim.
fn blank_foundry_version(content: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(content);
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, nl) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        if is_version_line(body) {
            out.push_str("version = \"0.0.0\"");
            out.push_str(nl);
        } else {
            out.push_str(line);
        }
    }
    out.into_bytes()
}

/// Normalized content hash of a package's files. Excludes everything under
/// `src/generated/` (per-release snapshots + generated aliasing libs — derived
/// from source, and a fresh `<tag>/` dir appears every release, so hashing it
/// would flag "changed" on every merge). Blanks foundry.toml's version line.
/// Then hashes each remaining file as `name \0 content`, in byte-sorted name
/// order, through one SHA-256 — so identical source yields an identical digest
/// regardless of zip entry order.
fn norm_hash(entries: &mut Vec<Entry>) -> String {
    entries.retain(|(name, _)| !name.starts_with("src/generated/"));
    for (name, content) in entries.iter_mut() {
        if name == "foundry.toml" {
            *content = blank_foundry_version(content);
        }
    }
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut h = Sha256::new();
    for (name, content) in entries.iter() {
        h.update(name.as_bytes());
        h.update([0u8]);
        h.update(content);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Read a zip into (name, bytes) entries, skipping directory entries.
fn read_zip(path: &Path) -> Vec<Entry> {
    let file = std::fs::File::open(path)
        .unwrap_or_else(|e| fail(&format!("open {}: {e}", path.display())));
    let mut archive = zip::ZipArchive::new(file)
        .unwrap_or_else(|e| fail(&format!("read zip {}: {e}", path.display())));
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .unwrap_or_else(|e| fail(&format!("zip entry {i}: {e}")));
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .unwrap_or_else(|e| fail(&format!("read zip entry {name}: {e}")));
        out.push((name, buf));
    }
    out
}

/// Parse a "major.minor.patch" version into three numbers. None if it is not
/// exactly three numeric dot-separated components.
fn parse_ver(v: &str) -> Option<[u64; 3]> {
    let mut it = v.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some([a, b, c])
}

/// True iff `a` is strictly ahead of `b` in version order. Fail-closed: an
/// unparseable operand is treated as NOT ahead.
fn ver_gt(a: &str, b: &str) -> bool {
    match (parse_ver(a), parse_ver(b)) {
        (Some(x), Some(y)) => x > y,
        _ => false,
    }
}

/// The next unpublished version: bump `v`'s patch component.
fn bump_patch(v: &str) -> String {
    let p = parse_ver(v).unwrap_or([0, 0, 0]);
    format!("{}.{}.{}", p[0], p[1], p[2] + 1)
}

/// Extract (latest published version, its zip url) from the Soldeer revision
/// API response. Either is None when absent.
fn parse_registry(json: &str) -> (Option<String>, Option<String>) {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
    let d0 = v.get("data").and_then(|d| d.get(0));
    let ver = d0
        .and_then(|x| x.get("version"))
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let url = d0
        .and_then(|x| x.get("url"))
        .and_then(|x| x.as_str())
        .map(str::to_string);
    (ver, url)
}

/// First `[package].version` value in foundry.toml (the in-dev, unpublished
/// version). Reads the value between the first pair of quotes on that line.
fn read_local_version(dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(dir.join("foundry.toml")).ok()?;
    for line in content.lines() {
        if is_version_line(line) {
            let q1 = line.find('"')?;
            let rest = &line[q1 + 1..];
            let q2 = rest.find('"')?;
            return Some(rest[..q2].to_string());
        }
    }
    None
}

/// Run the Soldeer content gate for `pkg` and emit changed / version / next.
pub(crate) fn run(pkg: &str, gh_out: Option<&str>) {
    let dir = Path::new(".");
    let local =
        read_local_version(dir).unwrap_or_else(|| fail("foundry.toml has no [package].version"));
    if parse_ver(&local).is_none() {
        fail(&format!(
            "foundry.toml [package].version ({local}) is not a major.minor.patch version"
        ));
    }

    // Latest published revision (version + zip url); {} on any fetch failure.
    let json = curl_stdout(&format!(
        "https://api.soldeer.xyz/api/v1/revision?project_name={pkg}&offset=0&limit=1"
    ))
    .unwrap_or_else(|| "{}".to_string());
    let (remote, url) = parse_registry(&json);

    // Next-version invariant: the in-dev version must be AHEAD of what is
    // published. If a prior run published but its bump-commit push failed,
    // local would equal remote — fail loud with a clear action, not a silent
    // re-publish / mis-bump.
    if let Some(r) = &remote {
        if !ver_gt(&local, r) {
            fail(&format!(
                "foundry.toml [package].version ({local}) is not ahead of the published revision ({r}); \
                 the next-version lifecycle needs it to be the next UNPUBLISHED version — bump [package].version above {r}."
            ));
        }
    }

    // Local package content: `forge soldeer push --dry-run` writes
    // <cwd-basename>.zip into the cwd.
    remove_cwd_zips();
    let spec = format!("{pkg}~{local}");
    run_cmd(
        Command::new("forge").args(["soldeer", "push", &spec, "--dry-run"]),
        "forge soldeer push --dry-run",
    );
    let local_zip = newest_cwd_zip().unwrap_or_else(|| fail("forge dry-run produced no .zip"));
    let mut local_entries = read_zip(&local_zip);
    let new_hash = norm_hash(&mut local_entries);
    remove_cwd_zips();

    // Published content, hashed the same way; "none" when nothing is published.
    let old_hash = match (&remote, url.as_deref()) {
        (Some(_), Some(u)) if !u.is_empty() => {
            let tmp = std::env::temp_dir().join("soldeer_pub.zip");
            run_cmd(
                Command::new("curl").args(["-fsSL", u, "-o"]).arg(&tmp),
                "curl published zip",
            );
            let mut pub_entries = read_zip(&tmp);
            let _ = std::fs::remove_file(&tmp);
            norm_hash(&mut pub_entries)
        }
        _ => "none".to_string(),
    };

    let changed = old_hash != new_hash;
    let next = bump_patch(&local);
    eprintln!(
        "soldeer gate: remote={} publish={local} next={next} OLD={old_hash} NEW={new_hash}",
        remote.as_deref().unwrap_or("none")
    );

    emit(
        gh_out,
        &format!("changed={changed}\nversion={local}\nnext={next}\n"),
    );
}

/// Write key=value output lines to --github-output, or stdout when absent.
fn emit(gh_out: Option<&str>, lines: &str) {
    match gh_out {
        Some(path) => {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)
                .unwrap_or_else(|e| fail(&format!("open {path}: {e}")));
            f.write_all(lines.as_bytes())
                .unwrap_or_else(|e| fail(&format!("write {path}: {e}")));
        }
        None => print!("{lines}"),
    }
}

/// Run a subprocess, inheriting stdio; fail loud on spawn error or nonzero exit.
fn run_cmd(cmd: &mut Command, what: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| fail(&format!("{what}: failed to spawn: {e}")));
    if !status.success() {
        fail(&format!("{what}: exited with {status}"));
    }
}

/// GET a URL with curl, returning its body on success.
fn curl_stdout(url: &str) -> Option<String> {
    let out = Command::new("curl").args(["-fsSL", url]).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// Paths of `*.zip` files in the cwd.
fn cwd_zips() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(".") {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "zip") {
                v.push(p);
            }
        }
    }
    v
}

fn remove_cwd_zips() {
    for p in cwd_zips() {
        let _ = std::fs::remove_file(p);
    }
}

/// Most recently modified `*.zip` in the cwd (the dry-run output).
fn newest_cwd_zip() -> Option<PathBuf> {
    cwd_zips()
        .into_iter()
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-soldeer-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn version_line_detection() {
        assert!(is_version_line("version = \"0.1.2\""));
        assert!(is_version_line("version=\"0.1.2\""));
        assert!(is_version_line("version\t = \"0.1.2\""));
        assert!(!is_version_line("  version = \"0.1.2\"")); // leading ws => not the [package] anchor
        assert!(!is_version_line("versionx = 1"));
        assert!(!is_version_line("# version = 1"));
    }

    #[test]
    fn blank_only_the_version_line() {
        let src = b"[package]\nname = \"rain-erc\"\nversion = \"9.9.9\"\ndescription = \"v\"\n";
        let out = blank_foundry_version(src);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("version = \"0.0.0\""));
        assert!(!s.contains("9.9.9"));
        assert!(s.contains("name = \"rain-erc\"")); // untouched
        assert!(s.contains("description = \"v\""));
    }

    #[test]
    fn norm_hash_ignores_version_bump() {
        let mut a = vec![
            (
                "foundry.toml".to_string(),
                b"[package]\nversion = \"0.1.0\"\n".to_vec(),
            ),
            ("src/A.sol".to_string(), b"contract A {}".to_vec()),
        ];
        let mut b = vec![
            (
                "foundry.toml".to_string(),
                b"[package]\nversion = \"0.9.9\"\n".to_vec(),
            ),
            ("src/A.sol".to_string(), b"contract A {}".to_vec()),
        ];
        assert_eq!(norm_hash(&mut a), norm_hash(&mut b));
    }

    #[test]
    fn norm_hash_excludes_generated() {
        let base = ("src/A.sol".to_string(), b"contract A {}".to_vec());
        let mut without = vec![base.clone()];
        let mut with_gen = vec![
            base,
            (
                "src/generated/0.1.0/A.pointers.sol".to_string(),
                b"address constant X = 1;".to_vec(),
            ),
        ];
        assert_eq!(norm_hash(&mut without), norm_hash(&mut with_gen));
    }

    #[test]
    fn norm_hash_detects_source_change() {
        let mut a = vec![("src/A.sol".to_string(), b"contract A {}".to_vec())];
        let mut b = vec![("src/A.sol".to_string(), b"contract B {}".to_vec())];
        assert_ne!(norm_hash(&mut a), norm_hash(&mut b));
    }

    #[test]
    fn norm_hash_is_order_independent() {
        let mut a = vec![
            ("src/A.sol".to_string(), b"a".to_vec()),
            ("src/B.sol".to_string(), b"b".to_vec()),
        ];
        let mut b = vec![
            ("src/B.sol".to_string(), b"b".to_vec()),
            ("src/A.sol".to_string(), b"a".to_vec()),
        ];
        assert_eq!(norm_hash(&mut a), norm_hash(&mut b));
    }

    #[test]
    fn version_parse_compare_bump() {
        assert_eq!(parse_ver("1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse_ver("1.2"), None);
        assert_eq!(parse_ver("1.2.3.4"), None);
        assert_eq!(parse_ver("1.2.x"), None);
        assert!(ver_gt("0.1.2", "0.1.1"));
        assert!(ver_gt("0.2.0", "0.1.9"));
        assert!(!ver_gt("0.1.1", "0.1.1")); // equal is not ahead
        assert!(!ver_gt("0.1.0", "0.1.1"));
        assert!(!ver_gt("bad", "0.1.1")); // fail-closed
        assert_eq!(bump_patch("0.1.2"), "0.1.3");
        assert_eq!(bump_patch("1.0.9"), "1.0.10");
    }

    #[test]
    fn registry_parse() {
        let (v, u) = parse_registry(r#"{"data":[{"version":"1.2.3","url":"http://x/z.zip"}]}"#);
        assert_eq!(v.as_deref(), Some("1.2.3"));
        assert_eq!(u.as_deref(), Some("http://x/z.zip"));
        assert_eq!(parse_registry("{}"), (None, None));
        assert_eq!(parse_registry(r#"{"data":[]}"#), (None, None));
        assert_eq!(parse_registry("not json"), (None, None));
    }

    #[test]
    fn local_version_read() {
        let d = tmp_dir();
        std::fs::write(
            d.join("foundry.toml"),
            "[package]\nname = \"x\"\nversion = \"0.4.2\"\n",
        )
        .unwrap();
        assert_eq!(read_local_version(&d).as_deref(), Some("0.4.2"));
        let e = tmp_dir();
        std::fs::write(e.join("foundry.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert_eq!(read_local_version(&e), None);
    }
}
