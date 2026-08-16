//! Shared engine for the byte-cap checks: charge a set of root files, follow
//! the references out of them, and report largest-first.
//!
//! `agent-context-cap` (rainlanguage/rainix#299) and `prompt-cap`
//! (rainlanguage/rainix#310) cap different things loaded by different loaders,
//! but the traversal is one algorithm: read a root, charge its bytes as the
//! loader sees them, find the file references in it, resolve them against the
//! file they were found in, charge those too, stop at a depth bound, and never
//! charge the same file twice however many ways it is reached.
//!
//! Reimplementing that is how a second check ends up subtly different from the
//! first: the resolve-against-the-importer rule, the code-span and fence
//! masking, and the cycle dedup are exactly the parts that look obvious and are
//! not. So they live here once, and what differs is a [`Charge`]: which
//! references count, how they resolve, and what the loader strips before the
//! text lands in the window.
//!
//! What is NOT shared is the cap. Each caller owns its own number and its own
//! failure prose, because where the number lives is the whole mechanism —
//! `agent_context_cap::CAP_BYTES` can be a compile-time ratchet precisely
//! because it lives in this repo, and `prompt-cap`'s number cannot, because
//! which files are prompts is per-repo.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One file that lands in the window, and what it costs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Contributor {
    pub(crate) bytes: u64,
    /// Path as reported to the user, plus why it is loaded.
    pub(crate) label: String,
}

/// What a caller charges and what it follows. The three things that differ
/// between the checks, and nothing else.
pub(crate) struct Charge {
    /// The reference tokens in a file's text. Given the RAW text, so a matcher
    /// decides for itself what masking means for its syntax ([`mask_code`] is
    /// here for the markdown answer). Deliberately liberal is safe: a token
    /// that does not resolve costs nothing.
    pub(crate) tokens: fn(&str) -> Vec<String>,
    /// The file a token names, or `None` when it names nothing chargeable.
    /// `from` is the file the token was found in — relative references resolve
    /// against it, not against the working directory.
    pub(crate) resolve: fn(root: &Path, from: &Path, token: &str) -> Option<PathBuf>,
    /// Text as the loader injects it. Bytes the loader drops before injection
    /// are not in the window, so charging them would bill for context that
    /// never loads.
    pub(crate) strip: fn(&str) -> String,
    /// How a followed file is labelled: "<path> (<verb> <parent>)".
    pub(crate) verb: &'static str,
    /// How many hops of references to follow. A bound belongs to the loader
    /// being modelled, so it is the caller's number.
    pub(crate) max_depth: usize,
}

/// A traversal in progress: what has been charged, and what has been seen so
/// it is not charged again.
pub(crate) struct Walk<'a> {
    root: &'a Path,
    charge: &'a Charge,
    seen: BTreeSet<PathBuf>,
    out: Vec<Contributor>,
}

impl<'a> Walk<'a> {
    pub(crate) fn new(root: &'a Path, charge: &'a Charge) -> Walk<'a> {
        Walk {
            root,
            charge,
            seen: BTreeSet::new(),
            out: Vec::new(),
        }
    }

    /// Content of a regular file that has not been charged yet, as it lands in
    /// context. `None` for anything absent, non-regular, unreadable or already
    /// counted — an absent file is a PASS, these checks cap what loads, they
    /// never require a file to exist.
    ///
    /// The `is_file` guard rejects non-regular paths — a directory, a fifo — up
    /// front rather than relying on the read to fail on them. Belt and braces:
    /// a directory would fail the read anyway, but a fifo would BLOCK it, and a
    /// CI job that hangs costs a runner for its whole timeout.
    ///
    /// Marks the file seen, so a caller that reads a file and then decides NOT
    /// to charge it has still spent it.
    pub(crate) fn take(&mut self, path: &Path) -> Option<String> {
        if !path.is_file() {
            return None;
        }
        // Canonicalize so the same file reached two ways is charged once (and
        // so a reference cycle terminates).
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !self.seen.insert(key) {
            return None;
        }
        std::fs::read_to_string(path)
            .ok()
            .map(|s| (self.charge.strip)(&s))
    }

    /// Charge bytes against a label already decided by the caller.
    pub(crate) fn push(&mut self, bytes: u64, label: String) {
        self.out.push(Contributor { bytes, label });
    }

    /// Charge `path` under `label`, then everything it references, transitively.
    /// A file already charged, absent or non-regular is a no-op.
    pub(crate) fn root_file(&mut self, path: &Path, label: &str) {
        let Some(text) = self.take(path) else {
            return;
        };
        self.push(text.len() as u64, label.to_string());
        self.follow(path, &text, label, 1);
    }

    /// Charge every reference reachable from `text`, depth-first, stopping at
    /// `max_depth` hops. A token that does not resolve to a chargeable file
    /// contributes nothing — it is not loaded.
    fn follow(&mut self, from: &Path, text: &str, from_label: &str, depth: usize) {
        if depth > self.charge.max_depth {
            return;
        }
        for token in (self.charge.tokens)(text) {
            let Some(target) = (self.charge.resolve)(self.root, from, &token) else {
                continue;
            };
            let Some(body) = self.take(&target) else {
                continue;
            };
            let label = format!(
                "{} ({} {from_label})",
                display_path(self.root, &target),
                self.charge.verb
            );
            self.push(body.len() as u64, label.clone());
            self.follow(&target, &body, &label, depth + 1);
        }
    }

    /// The contributors, largest first, so the biggest cut is the first line
    /// read. Ties break on label for a deterministic report.
    pub(crate) fn finish(self) -> Vec<Contributor> {
        let mut out = self.out;
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.label.cmp(&b.label)));
        out
    }
}

/// Total charged bytes.
pub(crate) fn total(contributors: &[Contributor]) -> u64 {
    contributors.iter().map(|c| c.bytes).sum()
}

/// The per-file breakdown lines, in the order given.
pub(crate) fn breakdown(contributors: &[Contributor]) -> Vec<String> {
    contributors
        .iter()
        .map(|c| format!("  {:>7}  {}", c.bytes, c.label))
        .collect()
}

/// Path relative to the repo root when possible, so reports are readable and
/// stable across checkouts.
pub(crate) fn display_path(root: &Path, p: &Path) -> String {
    let rooted = std::fs::canonicalize(root);
    let target = std::fs::canonicalize(p);
    if let (Ok(r), Ok(t)) = (rooted, target) {
        if let Ok(rel) = t.strip_prefix(&r) {
            return rel.display().to_string();
        }
    }
    p.display().to_string()
}

/// Blank out fenced code blocks and inline code spans so a *quoted* path is not
/// read as a reference. Offsets are not preserved — only reference scanning
/// uses this, never the byte measurement.
pub(crate) fn mask_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<String> = None;
    for line in text.lines() {
        let marker = fence_marker(line.trim_start());
        match (&fence, &marker) {
            // Inside a fence: a run of the same char, at least as long, closes.
            (Some(open), Some(m)) if m.starts_with(&open[..1]) && m.len() >= open.len() => {
                fence = None;
            }
            (Some(_), _) => {}
            (None, Some(m)) => fence = Some(m.clone()),
            (None, None) => {
                out.push_str(&mask_spans(line));
            }
        }
        out.push('\n');
    }
    out
}

/// The opening/closing run of a fence line (3+ backticks or tildes), if any.
fn fence_marker(trimmed: &str) -> Option<String> {
    let c = trimmed.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let run: String = trimmed.chars().take_while(|&x| x == c).collect();
    (run.len() >= 3).then_some(run)
}

/// Blank inline code spans within one line. An unmatched backtick run is
/// literal, per CommonMark, so it is left alone.
fn mask_spans(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let open_start = i;
        while i < chars.len() && chars[i] == '`' {
            i += 1;
        }
        let n = i - open_start;
        let mut j = i;
        let mut close = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let run_start = j;
                while j < chars.len() && chars[j] == '`' {
                    j += 1;
                }
                if j - run_start == n {
                    close = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => {
                for _ in open_start..end {
                    out.push(' ');
                }
                i = end;
            }
            None => {
                for _ in 0..n {
                    out.push('`');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-ctxbytes-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// A minimal caller: every whitespace-delimited token is a reference, paths
    /// resolve against the file they were found in, nothing is stripped.
    fn plain() -> Charge {
        fn tokens(text: &str) -> Vec<String> {
            mask_code(text)
                .split_whitespace()
                .map(str::to_string)
                .collect()
        }
        fn resolve(_root: &Path, from: &Path, token: &str) -> Option<PathBuf> {
            Some(from.parent()?.join(token))
        }
        Charge {
            tokens,
            resolve,
            strip: |s| s.to_string(),
            verb: "referenced by",
            max_depth: 2,
        }
    }

    fn walk_root(dir: &Path, rel: &str) -> Vec<Contributor> {
        let charge = plain();
        let mut w = Walk::new(dir, &charge);
        w.root_file(&dir.join(rel), rel);
        w.finish()
    }

    #[test]
    fn references_are_charged_transitively_within_the_depth_bound() {
        let d = tmp_dir();
        write(&d, "root.txt", "a.txt");
        write(&d, "a.txt", "b.txt");
        write(&d, "b.txt", "c.txt");
        write(&d, "c.txt", &"z".repeat(9999));
        // depth 2: root -> a (hop 1) -> b (hop 2), and c is a hop too far.
        assert_eq!(total(&walk_root(&d, "root.txt")), 5 + 5 + 5);
    }

    #[test]
    fn references_resolve_against_the_file_they_are_found_in() {
        let d = tmp_dir();
        write(&d, "root.txt", "docs/a.txt");
        // `b.txt` inside docs/a.txt means docs/b.txt, NOT ./b.txt
        write(&d, "docs/a.txt", "b.txt");
        write(&d, "docs/b.txt", &"b".repeat(500));
        write(&d, "b.txt", &"w".repeat(9999));
        assert_eq!(total(&walk_root(&d, "root.txt")), 10 + 5 + 500);
    }

    #[test]
    fn a_cycle_terminates_and_each_file_is_charged_once() {
        let d = tmp_dir();
        write(&d, "root.txt", "a.txt");
        write(&d, "a.txt", "root.txt a.txt");
        assert_eq!(total(&walk_root(&d, "root.txt")), 5 + 14);
    }

    #[test]
    fn an_unresolvable_reference_costs_nothing() {
        let d = tmp_dir();
        write(&d, "root.txt", "nope/gone.txt");
        assert_eq!(total(&walk_root(&d, "root.txt")), 13);
    }

    #[test]
    fn a_directory_is_not_a_file_and_is_not_charged() {
        let d = tmp_dir();
        write(&d, "root.txt", "sub");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        assert_eq!(total(&walk_root(&d, "root.txt")), 3);
    }

    #[test]
    fn contributors_are_largest_first_and_say_what_pulled_them_in() {
        let d = tmp_dir();
        write(&d, "root.txt", "a.txt");
        write(&d, "a.txt", &"x".repeat(100));
        let c = walk_root(&d, "root.txt");
        assert_eq!(c[0].bytes, 100);
        assert_eq!(c[0].label, "a.txt (referenced by root.txt)");
        assert_eq!(c[1].label, "root.txt");
        assert_eq!(
            breakdown(&c),
            vec![
                "      100  a.txt (referenced by root.txt)".to_string(),
                "        5  root.txt".to_string()
            ]
        );
    }

    #[test]
    fn strip_decides_what_is_measured() {
        fn tokens(_: &str) -> Vec<String> {
            Vec::new()
        }
        fn resolve(_: &Path, _: &Path, _: &str) -> Option<PathBuf> {
            None
        }
        let d = tmp_dir();
        write(&d, "root.txt", "keep DROP keep");
        let charge = Charge {
            tokens,
            resolve,
            strip: |s| s.replace("DROP", ""),
            verb: "referenced by",
            max_depth: 1,
        };
        let mut w = Walk::new(&d, &charge);
        w.root_file(&d.join("root.txt"), "root.txt");
        assert_eq!(total(&w.finish()), 10);
    }

    #[test]
    fn take_spends_a_file_even_when_the_caller_does_not_charge_it() {
        let d = tmp_dir();
        write(&d, "a.txt", "body");
        let charge = plain();
        let mut w = Walk::new(&d, &charge);
        assert_eq!(w.take(&d.join("a.txt")).as_deref(), Some("body"));
        assert!(
            w.take(&d.join("a.txt")).is_none(),
            "a second read of the same file must not be charged twice"
        );
    }

    #[test]
    fn masking_hides_quoted_paths_from_the_matcher() {
        let text = "~~~\na.md\n~~~\n``b.md``\nc.md\n";
        let masked = mask_code(text);
        assert!(!masked.contains("a.md"), "fenced: {masked:?}");
        assert!(!masked.contains("b.md"), "code span: {masked:?}");
        assert!(masked.contains("c.md"), "prose: {masked:?}");
    }

    #[test]
    fn an_unmatched_backtick_run_is_literal() {
        assert_eq!(mask_code("a ` b.md\n"), "a ` b.md\n");
    }

    #[test]
    fn display_path_is_relative_to_the_root_when_it_can_be() {
        let d = tmp_dir();
        write(&d, "docs/a.md", "x");
        assert_eq!(display_path(&d, &d.join("docs/a.md")), "docs/a.md");
        let other = tmp_dir();
        write(&other, "b.md", "x");
        assert_eq!(
            display_path(&d, &other.join("b.md")),
            other.join("b.md").display().to_string(),
            "outside the root there is nothing to relativize against"
        );
    }
}
