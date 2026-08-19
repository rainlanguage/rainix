//! `soldeer-gate` — Soldeer registry-derived content gate.
//!
//! Compares the normalized content of what `forge soldeer push --dry-run` would
//! upload against the newest published revision, derives the publish version
//! from the registry (`max(patch_bump(newest published), local floor)` under
//! semver ordering), and emits `changed` / `version`. Runs inside sol-shell,
//! so `forge` and `curl` are on PATH.
//!
//! Also home to `soldeer-set-version`, which rewrites foundry.toml's version
//! line in the CI checkout so the published zip carries the version it is
//! published under; the workflow never commits or pushes that rewrite.

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

/// The version to publish. The registry is the authoritative version ledger:
/// its newest published revision, patch-bumped, is the baseline, and the
/// repo's `[package].version` is only a FLOOR — whichever is higher under
/// semver (numeric, not string) ordering wins. No published revision yet
/// means a first publish, which uses the local version as-is. A version that
/// does not parse as major.minor.patch is an error on either side: the local
/// floor is meaningless unless it can be ordered, and a published version
/// that cannot be ordered against must not be silently guessed past.
fn publish_version(local: &str, remote: Option<&str>) -> Result<String, String> {
    let l = parse_ver(local).ok_or_else(|| {
        format!("foundry.toml [package].version ({local}) is not a major.minor.patch version")
    })?;
    let Some(r) = remote else {
        return Ok(local.to_string());
    };
    let rv = parse_ver(r).ok_or_else(|| {
        format!(
            "published revision ({r}) is not a major.minor.patch version; \
             cannot derive the publish version from the registry"
        )
    })?;
    let bumped = [rv[0], rv[1], rv[2] + 1];
    Ok(if l > bumped {
        local.to_string()
    } else {
        format!("{}.{}.{}", bumped[0], bumped[1], bumped[2])
    })
}

/// Rewrite the FIRST `[package].version` line (same first-match anchor as
/// `read_local_version`) to `version`, preserving every other byte. Errors on
/// a non-major.minor.patch `version` or when no version line exists.
fn set_first_version_line(content: &str, version: &str) -> Result<String, String> {
    if parse_ver(version).is_none() {
        return Err(format!(
            "version ({version}) is not a major.minor.patch version"
        ));
    }
    let mut out = String::with_capacity(content.len());
    let mut done = false;
    for line in content.split_inclusive('\n') {
        let (body, nl) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        if !done && is_version_line(body) {
            out.push_str(&format!("version = \"{version}\""));
            out.push_str(nl);
            done = true;
        } else {
            out.push_str(line);
        }
    }
    if !done {
        return Err("foundry.toml has no [package].version line".to_string());
    }
    Ok(out)
}

/// `soldeer-set-version`: rewrite `dir`/foundry.toml's version line in place.
/// Errors are returned, not exited on, so the caller (main) owns the process
/// exit and the unit tests stay a plain in-process assertion.
pub(crate) fn set_version(dir: &Path, version: &str) -> Result<(), String> {
    let path = dir.join("foundry.toml");
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let out = set_first_version_line(&content, version)?;
    std::fs::write(&path, out).map_err(|e| format!("write {}: {e}", path.display()))
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

/// First `[package].version` value in foundry.toml (the version FLOOR).
/// Reads the value between the first pair of quotes on that line.
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

/// Run the Soldeer content gate for `pkg` and emit changed / version.
pub(crate) fn run(pkg: &str, gh_out: Option<&str>) {
    let dir = Path::new(".");
    let local =
        read_local_version(dir).unwrap_or_else(|| fail("foundry.toml has no [package].version"));

    // Newest published revision (version + zip url); {} on any fetch failure.
    let json = curl_stdout(&format!(
        "https://api.soldeer.xyz/api/v1/revision?project_name={pkg}&offset=0&limit=1"
    ))
    .unwrap_or_else(|| "{}".to_string());
    let (remote, url) = parse_registry(&json);

    // The registry derives the publish version; the local version line is
    // only a floor. local == published is the normal steady state (nothing
    // ever writes the version line back to the branch).
    let publish = publish_version(&local, remote.as_deref()).unwrap_or_else(|e| fail(&e));

    // Local package content: `forge soldeer push --dry-run` writes
    // <cwd-basename>.zip into the cwd.
    remove_cwd_zips();
    let spec = format!("{pkg}~{publish}");
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
    eprintln!(
        "soldeer gate: remote={} local={local} publish={publish} OLD={old_hash} NEW={new_hash}",
        remote.as_deref().unwrap_or("none")
    );

    emit(gh_out, &gate_output(changed, &publish));
}

/// The gate's machine output: the changed verdict and the derived publish
/// version, as GitHub-output key=value lines.
fn gate_output(changed: bool, publish: &str) -> String {
    format!("changed={changed}\nversion={publish}\n")
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
    fn version_parse() {
        assert_eq!(parse_ver("1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse_ver("1.2"), None);
        assert_eq!(parse_ver("1.2.3.4"), None);
        assert_eq!(parse_ver("1.2.x"), None);
    }

    #[test]
    fn publish_version_first_publish_uses_local() {
        assert_eq!(publish_version("0.1.0", None).unwrap(), "0.1.0");
        assert_eq!(publish_version("2.3.4", None).unwrap(), "2.3.4");
    }

    #[test]
    fn publish_version_steady_state_patch_bumps_published() {
        // local == newest published is the normal steady state (the workflow
        // never writes the version line back); the registry drives the bump.
        assert_eq!(publish_version("0.1.2", Some("0.1.2")).unwrap(), "0.1.3");
    }

    #[test]
    fn publish_version_stale_low_local_is_ignored() {
        // The version line is only a floor; the registry has moved past it.
        assert_eq!(publish_version("0.1.0", Some("0.4.7")).unwrap(), "0.4.8");
        // Even a local BEHIND the published version is harmless.
        assert_eq!(publish_version("0.1.0", Some("0.1.5")).unwrap(), "0.1.6");
    }

    #[test]
    fn publish_version_local_ahead_wins_as_floor() {
        // A deliberate minor/major jump in the repo outruns the registry bump.
        assert_eq!(publish_version("0.2.0", Some("0.1.9")).unwrap(), "0.2.0");
        assert_eq!(publish_version("1.0.0", Some("0.9.9")).unwrap(), "1.0.0");
    }

    #[test]
    fn publish_version_local_equal_to_bump_is_the_bump() {
        assert_eq!(publish_version("0.1.3", Some("0.1.2")).unwrap(), "0.1.3");
    }

    #[test]
    fn publish_version_orders_semver_not_strings() {
        // 0.1.9 patch-bumps to 0.1.10, which orders ABOVE 0.1.9 numerically
        // (a string compare would order "0.1.10" below "0.1.9").
        assert_eq!(publish_version("0.1.0", Some("0.1.9")).unwrap(), "0.1.10");
        assert_eq!(publish_version("0.1.9", Some("0.1.9")).unwrap(), "0.1.10");
        // A local floor of 0.1.10 beats a bumped 0.1.10 tie exactly.
        assert_eq!(publish_version("0.1.10", Some("0.1.9")).unwrap(), "0.1.10");
        assert_eq!(publish_version("1.0.9", Some("1.0.9")).unwrap(), "1.0.10");
    }

    #[test]
    fn publish_version_rejects_unparseable() {
        assert!(publish_version("0.1.0", Some("garbage")).is_err());
        assert!(publish_version("garbage", None).is_err());
        assert!(publish_version("garbage", Some("0.1.0")).is_err());
        assert!(publish_version("0.1", Some("0.1.0")).is_err());
    }

    #[test]
    fn set_version_line_rewrites_first_match_only() {
        let src = "version = \"0.1.0\"\nversion = \"0.2.0\"\n";
        let out = set_first_version_line(src, "0.4.2").unwrap();
        assert_eq!(out, "version = \"0.4.2\"\nversion = \"0.2.0\"\n");
    }

    #[test]
    fn set_version_line_preserves_everything_else() {
        let src = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n# version = \"9\"\n";
        let out = set_first_version_line(src, "0.9.9").unwrap();
        assert_eq!(
            out,
            "[package]\nname = \"x\"\nversion = \"0.9.9\"\n# version = \"9\"\n"
        );
    }

    #[test]
    fn set_version_line_keeps_missing_trailing_newline() {
        let out = set_first_version_line("version = \"1.0.0\"", "2.0.0").unwrap();
        assert_eq!(out, "version = \"2.0.0\"");
    }

    #[test]
    fn set_version_line_errors_without_version_line() {
        assert!(set_first_version_line("[package]\nname = \"x\"\n", "1.0.0").is_err());
    }

    #[test]
    fn set_version_line_rejects_non_semver() {
        assert!(set_first_version_line("version = \"1.0.0\"\n", "not-a-version").is_err());
        assert!(set_first_version_line("version = \"1.0.0\"\n", "1.0").is_err());
    }

    #[test]
    fn gate_output_emits_changed_and_publish_version() {
        assert_eq!(gate_output(true, "0.1.3"), "changed=true\nversion=0.1.3\n");
        assert_eq!(
            gate_output(false, "0.4.8"),
            "changed=false\nversion=0.4.8\n"
        );
    }

    #[test]
    fn set_version_writes_foundry_toml() {
        let d = tmp_dir();
        std::fs::write(d.join("foundry.toml"), "[package]\nversion = \"0.1.0\"\n").unwrap();
        set_version(&d, "0.9.9").unwrap();
        assert_eq!(read_local_version(&d).as_deref(), Some("0.9.9"));
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
