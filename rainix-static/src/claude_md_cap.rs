//! `claude-md-cap` — hard byte cap on a repo's root `CLAUDE.md`.
//!
//! `CLAUDE.md` is loaded into context on EVERY turn of EVERY session in the
//! repo, whether or not the session needs a word of it, so its size is a tax on
//! all work done there rather than a cost paid by the readers it helps. The org
//! default is therefore "empty", with the burden of proof on inclusion: a line
//! stays only if a capable agent looking at the repo would get it *wrong*, not
//! merely take a moment to find it (rainlanguage/rainix#298).
//!
//! Prose cannot enforce that — a paragraph asking for restraint has no failure
//! mode and rots silently. A number that CI checks does. This is that number.

use std::path::Path;

/// The cap, in bytes. **FLOOR-ONLY RATCHET: this value may only ever be
/// LOWERED, never raised.**
///
/// That is the whole mechanism, not a nicety. A cap that can move up is not a
/// cap: the first PR that finds it inconvenient raises it by exactly enough to
/// pass, every later PR points at that precedent, and the check degrades into a
/// ceremony that measures nothing. A cap that can only move down makes regrowth
/// structurally impossible and lets the existing fat be wound out gradually
/// instead of demanding every repo be rewritten in one sitting.
///
/// So: if a repo's `CLAUDE.md` fails this check, the fix is to CUT THE FILE.
/// It is never to edit this line upward. Lowering it is a one-line change that
/// fans out across every repo pinning the shared static job `@main`.
pub(crate) const CAP_BYTES: u64 = 4096;

/// The capped file, relative to the repo root. Sibling agent-context files
/// (`AGENTS.md`, `.cursorrules`) are deliberately NOT covered yet — see #298.
pub(crate) const FILE: &str = "CLAUDE.md";

/// Byte size of `<dir>/CLAUDE.md`, or `None` when there is no such regular
/// file. An absent `CLAUDE.md` is a PASS: this check caps the file, it never
/// requires it to exist. The `is_file` guard matters — a directory named
/// `CLAUDE.md` reports its own inode size (4096 on ext4, i.e. exactly at the
/// cap), which would silently mean "clean" for the wrong reason.
fn size_bytes(dir: &Path) -> Option<u64> {
    match std::fs::metadata(dir.join(FILE)) {
        Ok(m) if m.is_file() => Some(m.len()),
        _ => None,
    }
}

/// Returns the offender report lines; empty means clean. A file exactly AT the
/// cap passes — the cap is the largest permitted size, so `>` is the failure.
pub(crate) fn check(dir: &Path) -> Vec<String> {
    let size = match size_bytes(dir) {
        Some(s) => s,
        None => return Vec::new(),
    };
    if size <= CAP_BYTES {
        return Vec::new();
    }
    let over = size - CAP_BYTES;
    vec![format!(
        "ERROR: {} is {size} bytes — {over} over the {CAP_BYTES}-byte cap. Cut {over} bytes; \
         do NOT raise the cap (it is a floor-only ratchet). Cut by asking of each line: would \
         a capable agent looking at this repo get this WRONG, or merely take a moment to find \
         it? Directory layouts, what a thing does, which command CI runs, where tests live are \
         all discoverable — cut them. A rule repeated often enough to write down is a rule \
         worth linting, with the reference in the tool's --help. What survives is irreversible \
         hazards and rulings whose rationale is not recoverable from the code. \
         See rainlanguage/rainix#298.",
        dir.join(FILE).display()
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-claudecap-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Write a `CLAUDE.md` of exactly `n` bytes and return its dir.
    fn with_claude_md(n: usize) -> std::path::PathBuf {
        let d = tmp_dir();
        std::fs::write(d.join(FILE), "x".repeat(n)).unwrap();
        d
    }

    #[test]
    fn absent_file_passes() {
        let d = tmp_dir();
        std::fs::write(d.join("README.md"), "x".repeat(CAP_BYTES as usize * 2)).unwrap();
        assert!(check(&d).is_empty());
    }

    #[test]
    fn under_cap_passes() {
        assert!(check(&with_claude_md(CAP_BYTES as usize - 1)).is_empty());
    }

    #[test]
    fn exactly_at_cap_passes() {
        assert!(check(&with_claude_md(CAP_BYTES as usize)).is_empty());
    }

    #[test]
    fn one_byte_over_cap_fails() {
        let d = with_claude_md(CAP_BYTES as usize + 1);
        let off = check(&d);
        assert_eq!(off.len(), 1, "{off:?}");
    }

    #[test]
    fn failure_names_file_size_cap_and_overage() {
        let d = with_claude_md(CAP_BYTES as usize + 500);
        let off = check(&d);
        assert_eq!(off.len(), 1, "{off:?}");
        let msg = &off[0];
        // the file, by path
        assert!(msg.contains(&d.join(FILE).display().to_string()), "{msg}");
        // its actual size
        assert!(
            msg.contains(&format!("{} bytes", CAP_BYTES + 500)),
            "{msg}"
        );
        // the cap
        assert!(msg.contains(&format!("{CAP_BYTES}-byte cap")), "{msg}");
        // the overage
        assert!(msg.contains("500 over"), "{msg}");
    }

    /// A *directory* named `CLAUDE.md` is not a `CLAUDE.md`. Without the
    /// `is_file` guard this passes for the wrong reason (dir len == 4096 on
    /// ext4, exactly at the cap) and would keep passing were the cap lowered.
    #[test]
    fn directory_named_claude_md_is_not_a_file() {
        let d = tmp_dir();
        std::fs::create_dir_all(d.join(FILE)).unwrap();
        assert!(size_bytes(&d).is_none());
        assert!(check(&d).is_empty());
    }

    /// Size is measured in BYTES, not chars: a file whose char count is under
    /// the cap but whose UTF-8 encoding is over it must fail. Context is
    /// charged by bytes-to-tokens, not by code points.
    #[test]
    fn multibyte_content_is_counted_by_bytes() {
        let d = tmp_dir();
        // 3 bytes each in UTF-8, so CAP_BYTES/2 chars is 1.5x the cap.
        let chars = CAP_BYTES as usize / 2;
        let body = "é€".repeat(chars / 2);
        assert!(body.chars().count() < CAP_BYTES as usize);
        std::fs::write(d.join(FILE), &body).unwrap();
        assert_eq!(size_bytes(&d), Some(body.len() as u64));
        assert!(!check(&d).is_empty());
    }
}
