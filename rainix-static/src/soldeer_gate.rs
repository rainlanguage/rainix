//! `soldeer-gate` — Soldeer registry-derived content gate.
//!
//! Compares the normalized content of what `forge soldeer push --dry-run` would
//! upload against the newest published revision, derives the publish version as
//! `max(patch_bump(newest published), newest next-v intent tag merged into
//! HEAD)` under semver ordering, and emits `changed` / `version`. foundry.toml
//! is never read for release metadata and never rewritten: the
//! `[external.package]` / legacy `[package]` section is excluded from the
//! content hash so carrying it, editing it, or deleting it is content-neutral.
//! Runs inside sol-shell; `forge`, `curl` and `git` are on PATH (the nix
//! package wraps the binary with pinned git + curl).

use crate::fail;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A file entry pulled from a package zip: (name, bytes).
type Entry = (String, Vec<u8>);

/// True if the line is a release-metadata section header: `[external.package]`,
/// or the legacy `[package]` form. Surrounding whitespace and a trailing
/// `# comment` are allowed (TOML permits both); `[package.metadata]` and other
/// dotted extensions are NOT release metadata and do not match.
fn is_metadata_header(line: &str) -> bool {
    let t = line.trim();
    for header in ["[external.package]", "[package]"] {
        if let Some(rest) = t.strip_prefix(header) {
            let r = rest.trim_start();
            if r.is_empty() || r.starts_with('#') {
                return true;
            }
        }
    }
    false
}

/// True if the line opens ANY toml table — where the next section starts.
fn is_any_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// True for a full-line `#` comment.
fn is_comment_line(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

/// Strip every release-metadata section from foundry.toml content: the
/// `[external.package]` (or legacy `[package]`) header, everything under it up
/// to the next section header or EOF, and the contiguous full-line comment
/// block sitting directly above the header (a comment documents the section
/// below it, so it leaves with the section). By the same rule, a comment block
/// sitting directly above the NEXT header belongs to that next section and
/// stays. Every kept byte is preserved verbatim, so hashing the stripped bytes
/// makes deleting, editing, or never having had the section (attached comment
/// included) content-neutral, while any other foundry.toml change stays
/// visible.
fn strip_release_metadata(content: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(content);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut keep = vec![true; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if !is_metadata_header(lines[i]) {
            i += 1;
            continue;
        }
        // Attached preceding comment block leaves with the section.
        let mut start = i;
        while start > 0 && is_comment_line(lines[start - 1]) {
            start -= 1;
        }
        // Section body runs to the next header (or EOF) …
        let mut next = i + 1;
        while next < lines.len() && !is_any_header(lines[next]) {
            next += 1;
        }
        // … minus the comment block attached to that next header.
        let mut end = next;
        if next < lines.len() {
            while end > i + 1 && is_comment_line(lines[end - 1]) {
                end -= 1;
            }
        }
        for k in keep.iter_mut().take(end).skip(start) {
            *k = false;
        }
        i = next;
    }
    lines
        .iter()
        .zip(&keep)
        .filter(|(_, &k)| k)
        .map(|(l, _)| *l)
        .collect::<String>()
        .into_bytes()
}

/// Normalized content hash of a package's files. Excludes everything under
/// `src/generated/` (per-release snapshots + generated aliasing libs — derived
/// from source, and a fresh `<tag>/` dir appears every release, so hashing it
/// would flag "changed" on every merge). Strips foundry.toml's release-metadata
/// section (see `strip_release_metadata`) so release metadata is never content.
/// Then hashes each remaining file as `name \0 content`, in byte-sorted name
/// order, through one SHA-256 — so identical source yields an identical digest
/// regardless of zip entry order.
fn norm_hash(entries: &mut Vec<Entry>) -> String {
    entries.retain(|(name, _)| !name.starts_with("src/generated/"));
    for (name, content) in entries.iter_mut() {
        if name == "foundry.toml" {
            *content = strip_release_metadata(content);
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

/// The highest `next-v<major.minor.patch>` intent version among git tag names
/// (one per line, as `git tag --merged HEAD` prints them), semver-ordered.
/// A tag without the `next-v` prefix is not an intent tag and is ignored; a
/// `next-v` tag whose remainder is not major.minor.patch is a loud error — a
/// typo'd intent must never be silently skipped (fail-safe, the same posture
/// as registry failures). Ok(None) when no intent tags exist.
fn max_intent_tag(tag_lines: &str) -> Result<Option<[u64; 3]>, String> {
    let mut max: Option<[u64; 3]> = None;
    for tag in tag_lines.lines().map(str::trim) {
        let Some(rest) = tag.strip_prefix("next-v") else {
            continue;
        };
        let v = parse_ver(rest).ok_or_else(|| {
            format!(
                "intent tag {tag} does not parse as next-v<major.minor.patch>; \
                 fix or delete the tag"
            )
        })?;
        if Some(v) > max {
            max = Some(v);
        }
    }
    Ok(max)
}

/// The version to publish. The registry is the authoritative version ledger:
/// its newest published revision, patch-bumped, is the baseline, and a next-v
/// intent tag merged into HEAD can only raise it — whichever is higher under
/// semver (numeric, not string) ordering wins, so consumed or stale intent
/// tags are inert. No published revision yet is a first publish, which
/// REQUIRES an intent tag as the explicit version seed. A published version
/// that cannot be ordered against must not be silently guessed past: loud
/// error.
fn publish_version(remote: Option<&str>, intent: Option<[u64; 3]>) -> Result<String, String> {
    let fmt = |v: [u64; 3]| format!("{}.{}.{}", v[0], v[1], v[2]);
    let Some(r) = remote else {
        let seed = intent.ok_or_else(|| {
            "nothing is published yet and no next-v intent tag is merged into HEAD; \
             a first publish requires an explicit version seed — tag the commit to \
             release (e.g. `git tag next-v0.1.0 && git push origin next-v0.1.0`) \
             and re-run"
                .to_string()
        })?;
        return Ok(fmt(seed));
    };
    let rv = parse_ver(r).ok_or_else(|| {
        format!(
            "published revision ({r}) is not a major.minor.patch version; \
             cannot derive the publish version from the registry"
        )
    })?;
    let patch = rv[2].checked_add(1).ok_or_else(|| {
        format!("published revision ({r}) has patch u64::MAX; cannot patch-bump past it")
    })?;
    let bumped = [rv[0], rv[1], patch];
    Ok(fmt(match intent {
        Some(i) if i > bumped => i,
        _ => bumped,
    }))
}

/// A shallow checkout cannot tell which tags are merged into HEAD — an intent
/// tag on an ancestor outside the shallow window is silently invisible, and
/// the gate would derive the wrong version. Refuse to run on one. Input is
/// `git rev-parse --is-shallow-repository` output.
fn require_full_history(is_shallow: &str) -> Result<(), String> {
    match is_shallow.trim() {
        "false" => Ok(()),
        "true" => Err(
            "checkout is shallow: `git tag --merged HEAD` cannot see intent tags on \
             commits outside the shallow window; fetch full history \
             (actions/checkout fetch-depth: 0) and re-run"
                .to_string(),
        ),
        other => Err(format!(
            "git rev-parse --is-shallow-repository printed {other:?}, expected true or false"
        )),
    }
}

/// The newest published revision on the Soldeer registry: version + zip url.
#[derive(Debug, PartialEq)]
struct Revision {
    version: String,
    url: String,
}

/// Decide what the registry said from the revision API's HTTP status + body.
/// Ok(Some(_)) is the newest published revision; Ok(None) is a genuine first
/// publish. Everything else — a non-registry HTTP status, unparseable JSON,
/// or a revision missing its version/url — is an error: a failed lookup must
/// never be mistaken for "nothing published yet". First publish has exactly
/// two shapes, both pinned against the live API: HTTP 404 carrying the
/// registry's own fail envelope ({"status":"fail"} — how it answers an
/// unknown project), and HTTP 200 with an explicitly empty data array (the
/// project exists with zero revisions).
fn registry_revision(status: u16, body: &str) -> Result<Option<Revision>, String> {
    if status != 200 && status != 404 {
        return Err(format!("soldeer registry returned HTTP {status}: {body}"));
    }
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        format!("soldeer registry returned HTTP {status} with unparseable JSON ({e}): {body}")
    })?;
    if status == 404 {
        return if v.get("status").and_then(|s| s.as_str()) == Some("fail") {
            Ok(None)
        } else {
            Err(format!(
                "soldeer registry returned HTTP 404 without the registry's fail envelope: {body}"
            ))
        };
    }
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| format!("soldeer registry response has no data array: {body}"))?;
    let Some(d0) = data.first() else {
        return Ok(None);
    };
    let field = |k: &str| {
        d0.get(k)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("soldeer registry revision has no {k}: {body}"))
    };
    Ok(Some(Revision {
        version: field("version")?,
        url: field("url")?,
    }))
}

/// Run the Soldeer content gate for `pkg` and emit changed / version.
pub(crate) fn run(pkg: &str, gh_out: Option<&str>) {
    // Version intent: next-v tags merged into HEAD. git supplies the data;
    // which tags are intent tags and which intent wins are decisions in
    // tested pure functions (max_intent_tag / publish_version). Reachability
    // needs full history, so a shallow checkout is refused up front rather
    // than silently hiding a real intent tag.
    let shallow = capture_stdout(
        Command::new("git").args(["rev-parse", "--is-shallow-repository"]),
        "git rev-parse --is-shallow-repository",
    );
    require_full_history(&shallow).unwrap_or_else(|e| fail(&e));
    let tags = capture_stdout(
        Command::new("git").args(["tag", "--merged", "HEAD"]),
        "git tag --merged HEAD",
    );
    let intent = max_intent_tag(&tags).unwrap_or_else(|e| fail(&e));

    // Newest published revision (version + zip url) from the registry. A
    // transport failure, non-registry HTTP status, or malformed response is a
    // loud gate error — never a first publish (which would derive an
    // already-published version and attempt an invalid upload).
    let (status, body) = curl_status_body(&format!(
        "https://api.soldeer.xyz/api/v1/revision?project_name={pkg}&offset=0&limit=1"
    ))
    .unwrap_or_else(|e| fail(&e));
    let remote = registry_revision(status, &body).unwrap_or_else(|e| fail(&e));

    // The registry patch-bump is the baseline; an intent tag can only raise
    // it. A first publish requires an intent tag as the explicit seed.
    let publish = publish_version(remote.as_ref().map(|r| r.version.as_str()), intent)
        .unwrap_or_else(|e| fail(&e));

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
    let old_hash = match &remote {
        Some(rev) => {
            let tmp = std::env::temp_dir().join("soldeer_pub.zip");
            run_cmd(
                Command::new("curl")
                    .args(["-fsSL", &rev.url, "-o"])
                    .arg(&tmp),
                "curl published zip",
            );
            let mut pub_entries = read_zip(&tmp);
            let _ = std::fs::remove_file(&tmp);
            norm_hash(&mut pub_entries)
        }
        None => "none".to_string(),
    };

    let changed = old_hash != new_hash;
    eprintln!(
        "soldeer gate: remote={} intent={} publish={publish} OLD={old_hash} NEW={new_hash}",
        remote
            .as_ref()
            .map(|r| r.version.as_str())
            .unwrap_or("none"),
        intent
            .map(|v| format!("{}.{}.{}", v[0], v[1], v[2]))
            .unwrap_or_else(|| "none".to_string()),
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

/// Run a subprocess and return its stdout; fail loud (with its stderr) on
/// spawn error or nonzero exit.
fn capture_stdout(cmd: &mut Command, what: &str) -> String {
    let out = cmd
        .output()
        .unwrap_or_else(|e| fail(&format!("{what}: failed to spawn: {e}")));
    if !out.status.success() {
        fail(&format!(
            "{what}: {} ({})",
            String::from_utf8_lossy(&out.stderr).trim(),
            out.status
        ));
    }
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// GET a URL with curl, returning (HTTP status, body). Deliberately NOT `-f`:
/// `-f` swallows the status and body of an HTTP-level failure, and the
/// registry answers an unknown project with a 404 whose body the caller must
/// see. `-w` appends the status code after the body on its own line.
/// Transport failures (spawn, DNS, connect, TLS) are errors.
fn curl_status_body(url: &str) -> Result<(u16, String), String> {
    let out = Command::new("curl")
        .args(["-sSL", "-w", "\n%{http_code}", url])
        .output()
        .map_err(|e| format!("curl {url}: failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl {url}: {} ({})",
            String::from_utf8_lossy(&out.stderr).trim(),
            out.status
        ));
    }
    split_status_body(&String::from_utf8_lossy(&out.stdout))
}

/// Split curl `-w '\n%{http_code}'` stdout into (status, body): everything
/// after the LAST newline is the status code, everything before it the body.
fn split_status_body(stdout: &str) -> Result<(u16, String), String> {
    let (body, code) = stdout
        .rsplit_once('\n')
        .ok_or_else(|| format!("curl output has no status-code line: {stdout}"))?;
    let status = code
        .trim()
        .parse()
        .map_err(|_| format!("curl status-code line ({code}) is not a number"))?;
    Ok((status, body.to_string()))
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

    #[test]
    fn metadata_header_detection() {
        assert!(is_metadata_header("[external.package]"));
        assert!(is_metadata_header("[package]"));
        assert!(is_metadata_header("  [package]  "));
        assert!(is_metadata_header("[external.package] # release metadata"));
        assert!(!is_metadata_header("[package] name = \"x\""));
        assert!(!is_metadata_header("[package.metadata]"));
        assert!(!is_metadata_header("[external.package.extra]"));
        assert!(!is_metadata_header("[profile.default]"));
        assert!(!is_metadata_header("# [package]"));
        assert!(!is_metadata_header("[packagex]"));
        assert!(!is_metadata_header("name = \"[package]\""));
    }

    /// The real consumer shape (rain.datacontract): a stale comment block
    /// directly above [external.package]. The ruled consumer sweep deletes the
    /// section AND that comment; both sides must normalize identically.
    #[test]
    fn strip_removes_external_package_section_and_attached_comment() {
        let with = b"# Release metadata, not foundry config: autopublish reads `version`.\n\
                     # `[external.*]` is the section foundry reserves for another tool.\n\
                     [external.package]\n\
                     name = \"rain-datacontract\"\n\
                     version = \"0.1.2\"\n\
                     \n\
                     [profile.default]\n\
                     libs = [\"dependencies\"]\n";
        let swept = b"[profile.default]\nlibs = [\"dependencies\"]\n";
        assert_eq!(strip_release_metadata(with), strip_release_metadata(swept));
        assert_eq!(strip_release_metadata(swept), swept.to_vec());
    }

    #[test]
    fn strip_removes_legacy_package_section() {
        let with = b"[package]\n\
                     name = \"rain-math-float\"\n\
                     version = \"0.1.7\"\n\
                     \n\
                     [profile.default]\n\
                     src = 'src'\n";
        let swept = b"[profile.default]\nsrc = 'src'\n";
        assert_eq!(strip_release_metadata(with), strip_release_metadata(swept));
    }

    #[test]
    fn strip_is_identity_without_metadata_section() {
        let src = b"[profile.default]\nsolc = \"0.8.25\"\n\n[fuzz]\nruns = 1024\n";
        assert_eq!(strip_release_metadata(src), src.to_vec());
    }

    #[test]
    fn strip_makes_section_edits_neutral() {
        let a = b"[external.package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[fuzz]\nruns = 1\n";
        let b = b"[external.package]\nname = \"y\"\nversion = \"9.9.9\"\nextra = 1\n\n[fuzz]\nruns = 1\n";
        assert_eq!(strip_release_metadata(a), strip_release_metadata(b));
    }

    /// A comment block directly above the NEXT header documents that next
    /// section — it survives the strip on both the section-carrying and the
    /// swept file, so the sweep stays neutral around it.
    #[test]
    fn strip_keeps_comment_attached_to_next_header() {
        let with = b"[package]\n\
                     name = \"x\"\n\
                     version = \"1.0.0\"\n\
                     \n\
                     # Fuzz runs tuned for CI wall-clock.\n\
                     [fuzz]\n\
                     runs = 1024\n";
        let swept = b"# Fuzz runs tuned for CI wall-clock.\n[fuzz]\nruns = 1024\n";
        assert_eq!(strip_release_metadata(with), strip_release_metadata(swept));
        assert_eq!(strip_release_metadata(with), swept.to_vec());
    }

    #[test]
    fn strip_removes_section_at_eof() {
        let with = b"[profile.default]\nsolc = \"0.8.25\"\n\n# meta\n[package]\nname = \"x\"\nversion = \"1.0.0\"\n";
        let swept = b"[profile.default]\nsolc = \"0.8.25\"\n\n";
        assert_eq!(strip_release_metadata(with), strip_release_metadata(swept));
    }

    #[test]
    fn strip_removes_both_sections_when_present() {
        let with = b"[package]\nname = \"x\"\n\n[external.package]\nname = \"x\"\nversion = \"1.0.0\"\n\n[fuzz]\nruns = 1\n";
        let swept = b"[fuzz]\nruns = 1\n";
        assert_eq!(strip_release_metadata(with), strip_release_metadata(swept));
    }

    #[test]
    fn norm_hash_ignores_release_metadata_section() {
        let body = |toml: &[u8]| {
            vec![
                ("foundry.toml".to_string(), toml.to_vec()),
                ("src/A.sol".to_string(), b"contract A {}".to_vec()),
            ]
        };
        let mut carrying = body(
            b"# stale release-metadata comment\n[external.package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[profile.default]\nsolc = \"0.8.25\"\n",
        );
        let mut edited = body(
            b"# stale release-metadata comment\n[external.package]\nname = \"x\"\nversion = \"0.9.9\"\n\n[profile.default]\nsolc = \"0.8.25\"\n",
        );
        let mut swept = body(b"[profile.default]\nsolc = \"0.8.25\"\n");
        let h = norm_hash(&mut carrying);
        assert_eq!(h, norm_hash(&mut edited));
        assert_eq!(h, norm_hash(&mut swept));
    }

    #[test]
    fn norm_hash_detects_non_metadata_foundry_change() {
        let mut a = vec![(
            "foundry.toml".to_string(),
            b"[external.package]\nversion = \"0.1.0\"\n\n[profile.default]\nsolc = \"0.8.25\"\n"
                .to_vec(),
        )];
        let mut b = vec![(
            "foundry.toml".to_string(),
            b"[external.package]\nversion = \"0.1.0\"\n\n[profile.default]\nsolc = \"0.8.26\"\n"
                .to_vec(),
        )];
        assert_ne!(norm_hash(&mut a), norm_hash(&mut b));
    }

    #[test]
    fn norm_hash_excludes_generated() {
        let base = ("src/A.sol".to_string(), b"contract A {}".to_vec());
        let mut without = vec![base.clone()];
        let mut with_gen = vec![
            base,
            (
                "src/generated/0_1_0/A.sol".to_string(),
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
    fn intent_tags_non_matching_ignored() {
        // Ordinary tags are not intent tags: no error, no intent.
        assert_eq!(
            max_intent_tag("v1.0.0\nsol-v0.1.2\nrelease-2024\nnpm-1.2.3\n").unwrap(),
            None
        );
        assert_eq!(max_intent_tag("").unwrap(), None);
        assert_eq!(max_intent_tag("\n\n").unwrap(), None);
    }

    #[test]
    fn intent_tag_parses_next_v() {
        assert_eq!(max_intent_tag("next-v0.2.0\n").unwrap(), Some([0, 2, 0]));
        // Mixed with non-matching tags, which stay ignored.
        assert_eq!(
            max_intent_tag("sol-v0.1.9\nnext-v0.2.0\nv3.0.0\n").unwrap(),
            Some([0, 2, 0])
        );
    }

    #[test]
    fn intent_tags_max_is_semver_not_string_or_date_order() {
        // Multiple intent tags: the max under NUMERIC semver ordering wins
        // (string ordering would put 0.1.9 above 0.1.10), regardless of the
        // order git prints them in.
        assert_eq!(
            max_intent_tag("next-v0.1.10\nnext-v0.1.9\nnext-v0.0.2\n").unwrap(),
            Some([0, 1, 10])
        );
        assert_eq!(
            max_intent_tag("next-v0.1.9\nnext-v0.1.10\n").unwrap(),
            Some([0, 1, 10])
        );
        assert_eq!(
            max_intent_tag("next-v1.0.0\nnext-v0.99.99\n").unwrap(),
            Some([1, 0, 0])
        );
    }

    #[test]
    fn intent_tag_malformed_is_loud_error() {
        for bad in [
            "next-v1.2",
            "next-v1.2.3.4",
            "next-vX",
            "next-viking",
            "next-v",
        ] {
            let e = max_intent_tag(bad).unwrap_err();
            assert!(e.contains(bad), "{e}");
            assert!(e.contains("next-v<major.minor.patch>"), "{e}");
        }
        // A valid intent tag does not excuse a malformed one alongside it.
        assert!(max_intent_tag("next-v0.2.0\nnext-v1.2\n").is_err());
    }

    #[test]
    fn publish_version_first_publish_requires_intent_tag() {
        let e = publish_version(None, None).unwrap_err();
        // The error names the fix: create a next-v tag.
        assert!(e.contains("next-v"), "{e}");
        assert!(e.contains("git tag next-v"), "{e}");
    }

    #[test]
    fn publish_version_first_publish_uses_intent_tag() {
        assert_eq!(publish_version(None, Some([0, 2, 0])).unwrap(), "0.2.0");
        assert_eq!(publish_version(None, Some([2, 3, 4])).unwrap(), "2.3.4");
    }

    #[test]
    fn publish_version_steady_state_patch_bumps_published() {
        // No intent tags (or only consumed ones): the registry drives the bump.
        assert_eq!(publish_version(Some("0.1.2"), None).unwrap(), "0.1.3");
    }

    #[test]
    fn publish_version_stale_intent_tag_is_inert() {
        // A consumed/stale intent tag at or below the bump changes nothing.
        assert_eq!(
            publish_version(Some("0.4.7"), Some([0, 2, 0])).unwrap(),
            "0.4.8"
        );
        assert_eq!(
            publish_version(Some("0.1.5"), Some([0, 1, 5])).unwrap(),
            "0.1.6"
        );
    }

    #[test]
    fn publish_version_intent_tag_above_bump_wins() {
        // A deliberate minor/major jump is expressed as a next-v tag.
        assert_eq!(
            publish_version(Some("0.1.9"), Some([0, 2, 0])).unwrap(),
            "0.2.0"
        );
        assert_eq!(
            publish_version(Some("0.9.9"), Some([1, 0, 0])).unwrap(),
            "1.0.0"
        );
    }

    #[test]
    fn publish_version_intent_equal_to_bump_is_the_bump() {
        assert_eq!(
            publish_version(Some("0.1.2"), Some([0, 1, 3])).unwrap(),
            "0.1.3"
        );
    }

    #[test]
    fn publish_version_orders_semver_not_strings() {
        // 0.1.9 patch-bumps to 0.1.10, which orders ABOVE 0.1.9 numerically
        // (a string compare would order "0.1.10" below "0.1.9").
        assert_eq!(publish_version(Some("0.1.9"), None).unwrap(), "0.1.10");
        assert_eq!(
            publish_version(Some("0.1.9"), Some([0, 1, 9])).unwrap(),
            "0.1.10"
        );
        // An intent of 0.1.10 ties the bumped 0.1.10 exactly.
        assert_eq!(
            publish_version(Some("0.1.9"), Some([0, 1, 10])).unwrap(),
            "0.1.10"
        );
        assert_eq!(publish_version(Some("1.0.9"), None).unwrap(), "1.0.10");
    }

    #[test]
    fn publish_version_errors_on_patch_overflow() {
        // A published patch of u64::MAX cannot be bumped: loud error, never a
        // wraparound or panic — regardless of how high the intent tag is.
        let e = publish_version(Some("1.2.18446744073709551615"), None).unwrap_err();
        assert!(e.contains("18446744073709551615"), "{e}");
        assert!(e.contains("patch-bump"), "{e}");
        assert!(publish_version(Some("1.2.18446744073709551615"), Some([9, 9, 9])).is_err());
        // One below the boundary still bumps normally.
        assert_eq!(
            publish_version(Some("1.2.18446744073709551614"), None).unwrap(),
            "1.2.18446744073709551615"
        );
    }

    #[test]
    fn publish_version_rejects_unparseable_remote() {
        assert!(publish_version(Some("garbage"), None).is_err());
        assert!(publish_version(Some("garbage"), Some([1, 0, 0])).is_err());
        assert!(publish_version(Some("0.1"), Some([1, 0, 0])).is_err());
    }

    #[test]
    fn shallow_checkout_is_refused() {
        assert!(require_full_history("false\n").is_ok());
        let e = require_full_history("true\n").unwrap_err();
        assert!(e.contains("shallow"), "{e}");
        assert!(e.contains("fetch-depth: 0"), "{e}");
        // Unexpected output is an error, never treated as "not shallow".
        assert!(require_full_history("").is_err());
        assert!(require_full_history("maybe").is_err());
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
    fn registry_newest_revision_extracted() {
        // Live API shape for a published project (extra fields present).
        let rev = registry_revision(
            200,
            r#"{"data":[{"version":"1.2.3","url":"http://x/z.zip","deleted":false,"downloads":7}],"status":"success"}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            rev,
            Revision {
                version: "1.2.3".to_string(),
                url: "http://x/z.zip".to_string()
            }
        );
    }

    #[test]
    fn registry_empty_revision_list_is_first_publish() {
        // Project exists with zero revisions: the explicit empty data array.
        assert_eq!(
            registry_revision(200, r#"{"data":[],"status":"success"}"#).unwrap(),
            None
        );
    }

    #[test]
    fn registry_unknown_project_404_is_first_publish() {
        // Live API shape for a never-published project: HTTP 404 carrying the
        // registry's own fail envelope.
        assert_eq!(
            registry_revision(
                404,
                r#"{"message":"Project not found or access denied","status":"fail"}"#
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn registry_404_without_fail_envelope_is_an_error() {
        // A 404 that is not the registry's own answer (an outage page, a
        // proxy) must not be mistaken for "nothing published yet".
        assert!(registry_revision(404, "<html>not the registry</html>").is_err());
        assert!(registry_revision(404, r#"{"data":[]}"#).is_err());
        assert!(registry_revision(404, r#"{"status":"success"}"#).is_err());
    }

    #[test]
    fn registry_http_error_status_is_an_error() {
        assert!(registry_revision(500, r#"{"status":"fail"}"#).is_err());
        assert!(registry_revision(502, "<html>bad gateway</html>").is_err());
        assert!(registry_revision(503, "").is_err());
        // A success-shaped body on an error status must not be trusted —
        // neither as a revision nor as a first publish.
        assert!(registry_revision(
            500,
            r#"{"data":[{"version":"1.2.3","url":"http://x/z.zip"}],"status":"success"}"#
        )
        .is_err());
        assert!(registry_revision(500, r#"{"data":[],"status":"success"}"#).is_err());
    }

    #[test]
    fn registry_malformed_response_is_an_error() {
        assert!(registry_revision(200, "not json").is_err());
        assert!(registry_revision(200, "{}").is_err()); // no data array
        assert!(registry_revision(200, r#"{"data":"x"}"#).is_err()); // data not an array
    }

    #[test]
    fn registry_revision_missing_fields_is_an_error() {
        assert!(registry_revision(200, r#"{"data":[{"url":"http://x/z.zip"}]}"#).is_err());
        assert!(registry_revision(200, r#"{"data":[{"version":"1.2.3"}]}"#).is_err());
        assert!(
            registry_revision(200, r#"{"data":[{"version":"","url":"http://x/z.zip"}]}"#).is_err()
        );
        assert!(registry_revision(200, r#"{"data":[{"version":"1.2.3","url":""}]}"#).is_err());
    }

    #[test]
    fn curl_output_splits_into_status_and_body() {
        assert_eq!(
            split_status_body("body\n200").unwrap(),
            (200, "body".to_string())
        );
        // The LAST newline splits: bodies may contain newlines of their own.
        assert_eq!(
            split_status_body("{\"a\":1}\nmore\n404").unwrap(),
            (404, "{\"a\":1}\nmore".to_string())
        );
        assert_eq!(split_status_body("\n404").unwrap(), (404, String::new()));
        assert!(split_status_body("no-newline").is_err());
        assert!(split_status_body("body\nnot-a-number").is_err());
    }
}
