//! `env-ignored` — fail unless the repo's own committed `.gitignore` ignores
//! `.env` and no file named `.env` is tracked.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn git(dir: &Path, args: &[&str], stdin: &[u8], ok: &[i32]) -> Result<Vec<u8>, String> {
    let err = |e: String| format!("git {}: {e}", args.join(" "));
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| err(e.to_string()))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin)
        .map_err(|e| err(e.to_string()))?;
    let out = child.wait_with_output().map_err(|e| err(e.to_string()))?;
    match out.status.code() {
        Some(c) if c == 0 || ok.contains(&c) => Ok(out.stdout),
        _ => Err(err(String::from_utf8_lossy(&out.stderr).trim().to_string())),
    }
}

/// The global excludes file is disabled and a match from `.git/info/exclude`
/// does not count, so only patterns the repo itself carries can pass. The
/// deciding `.gitignore` must be in the index: an untracked one does not reach
/// a fresh clone.
fn ignored_by_committed_gitignore(dir: &Path) -> Result<bool, String> {
    let out = git(
        dir,
        &[
            "-c",
            "core.excludesFile=/dev/null",
            "check-ignore",
            "--no-index",
            "--stdin",
            "-v",
            "-z",
        ],
        b".env\0",
        &[1],
    )?;
    let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    let (source, pattern) = match (fields.first(), fields.get(2)) {
        (Some(s), Some(p)) if !s.is_empty() => (*s, *p),
        _ => return Ok(false),
    };
    let source = String::from_utf8_lossy(source);
    if pattern.starts_with(b"!")
        || Path::new(source.as_ref()).file_name() != Some(".gitignore".as_ref())
    {
        return Ok(false);
    }
    let tracked = git(dir, &["ls-files", "-z", "--", source.as_ref()], b"", &[])?;
    Ok(!tracked.is_empty())
}

fn tracked_env_files(dir: &Path) -> Result<Vec<String>, String> {
    let out = git(dir, &["ls-files", "-z"], b"", &[])?;
    Ok(out
        .split(|b| *b == 0)
        .map(String::from_utf8_lossy)
        .filter(|p| p.rsplit('/').next() == Some(".env"))
        .map(|p| p.into_owned())
        .collect())
}

/// Returns the offender report lines; empty means clean.
pub(crate) fn check(dir: &Path) -> Result<Vec<String>, String> {
    git(dir, &["rev-parse", "--git-dir"], b"", &[])?;
    let mut offenders = Vec::new();
    if !ignored_by_committed_gitignore(dir)? {
        offenders.push(
            "The repo's committed .gitignore does not ignore .env — add a line `.env` to .gitignore and commit it (a global or .git/info/exclude ignore does not count)."
                .to_string(),
        );
    }
    let tracked = tracked_env_files(dir)?;
    if !tracked.is_empty() {
        offenders.push(
            "Tracked .env files found — remove them with `git rm --cached <path>` and rotate every secret they held:"
                .to_string(),
        );
        for p in tracked {
            offenders.push(format!("  {p}"));
        }
    }
    Ok(offenders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn run(d: &Path, args: &[&str]) {
        assert!(Command::new("git")
            .arg("-C")
            .arg(d)
            .args(args)
            .status()
            .unwrap()
            .success());
    }

    fn tmp_repo() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-env-ignored-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        run(&d, &["init", "-q"]);
        d
    }

    fn committed_gitignore(d: &Path, body: &str) {
        std::fs::write(d.join(".gitignore"), body).unwrap();
        run(d, &["add", ".gitignore"]);
    }

    #[test]
    fn ignored_and_untracked_passes() {
        let d = tmp_repo();
        committed_gitignore(&d, "out\n.env\n");
        std::fs::write(d.join(".env"), "SECRET=1\n").unwrap();
        assert_eq!(check(&d).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn glob_patterns_count() {
        for pat in ["*.env", ".env*", "/.env", "**/.env"] {
            let d = tmp_repo();
            committed_gitignore(&d, &format!("{pat}\n"));
            assert!(check(&d).unwrap().is_empty(), "{pat}");
        }
    }

    #[test]
    fn missing_ignore_fails() {
        let d = tmp_repo();
        committed_gitignore(&d, "out\n");
        let off = check(&d).unwrap();
        assert_eq!(off.len(), 1, "{off:?}");
        assert!(
            off[0].contains("add a line `.env` to .gitignore"),
            "{off:?}"
        );
    }

    #[test]
    fn no_gitignore_fails() {
        let d = tmp_repo();
        assert_eq!(check(&d).unwrap().len(), 1);
    }

    #[test]
    fn negated_ignore_fails() {
        let d = tmp_repo();
        committed_gitignore(&d, ".env*\n!.env\n");
        assert_eq!(check(&d).unwrap().len(), 1);
    }

    #[test]
    fn untracked_gitignore_fails() {
        let d = tmp_repo();
        std::fs::write(d.join(".gitignore"), ".env\n").unwrap();
        assert_eq!(check(&d).unwrap().len(), 1);
    }

    #[test]
    fn info_exclude_only_fails() {
        let d = tmp_repo();
        std::fs::write(d.join(".git/info/exclude"), ".env\n").unwrap();
        assert_eq!(check(&d).unwrap().len(), 1);
    }

    #[test]
    fn global_excludes_only_fails() {
        let d = tmp_repo();
        let global = d.join("global-ignore");
        std::fs::write(&global, ".env\n").unwrap();
        run(
            &d,
            &["config", "core.excludesFile", global.to_str().unwrap()],
        );
        assert_eq!(check(&d).unwrap().len(), 1);
    }

    #[test]
    fn tracked_env_fails_even_when_ignored() {
        let d = tmp_repo();
        committed_gitignore(&d, ".env\n");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join(".env"), "SECRET=1\n").unwrap();
        std::fs::write(d.join("sub/.env"), "SECRET=1\n").unwrap();
        run(&d, &["add", "-f", ".env", "sub/.env"]);
        let off = check(&d).unwrap();
        assert!(off.iter().any(|l| l == "  .env"), "{off:?}");
        assert!(off.iter().any(|l| l == "  sub/.env"), "{off:?}");
        assert!(
            !off.iter().any(|l| l.contains("does not ignore")),
            "{off:?}"
        );
    }

    #[test]
    fn tracked_env_example_allowed() {
        let d = tmp_repo();
        committed_gitignore(&d, ".env\n");
        std::fs::write(d.join(".env.example"), "SECRET=\n").unwrap();
        run(&d, &["add", ".env.example"]);
        assert!(check(&d).unwrap().is_empty());
    }

    #[test]
    fn non_git_dir_is_an_error() {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-env-ignored-nogit-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        assert!(check(&d).is_err());
    }
}
