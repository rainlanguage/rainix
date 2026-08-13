//! `agent-context-cap` — hard byte cap on the agent context a repo loads at the
//! start of EVERY session.
//!
//! Launch-loaded context is a tax on all work done in the repo rather than a
//! cost paid by the readers it helps: it is in the window on every turn whether
//! or not the turn needs a word of it. The org default is therefore "empty",
//! with the burden of proof on inclusion — a line stays only if a capable agent
//! looking at the repo would get it *wrong*, not merely take a moment to find
//! it (rainlanguage/rainix#298). Prose asking for restraint has no failure mode
//! and rots silently; a number CI checks does not. This is that number.
//!
//! The cap is on the TOTAL, not on one file, because Claude Code loads project
//! memory from several places at launch and a single-file cap is evaded by
//! moving text sideways. What counts (per the Claude Code memory docs):
//!
//! - `CLAUDE.md`, or `.claude/CLAUDE.md` — alternative locations for the same
//!   project memory.
//! - Everything those pull in via `@path` imports, transitively. Imports are
//!   expanded into context at launch, so splitting a file up organises it
//!   without reducing anything. Max 4 hops; relative paths resolve against the
//!   IMPORTING file; `@` inside a code span or fence is literal text, not an
//!   import.
//! - Every `.claude/rules/**/*.md` WITHOUT a `paths:` frontmatter key — those
//!   load at launch with the same priority as `.claude/CLAUDE.md`.
//!
//! What deliberately does NOT count, because it is not loaded at launch:
//!
//! - Rules WITH `paths:` frontmatter — they load only when a matching file is
//!   read. That is legitimate scoping, and the cap must reward it.
//! - `CLAUDE.md` in subdirectories — loaded on demand when that subtree is read.
//! - `CLAUDE.local.md` — a gitignored personal file, not repo policy.
//! - Block-level HTML comments, which are stripped before injection, so a
//!   maintainer note costs nothing and is not charged for.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
/// So: if a repo fails this check, the fix is to CUT THE CONTENT (or scope a
/// rule with `paths:` so it stops loading at launch). It is never to edit this
/// line upward. Lowering it is a one-line change that fans out across every
/// repo pinning the shared static job `@main`.
pub(crate) const CAP_BYTES: u64 = 4096;

/// The ratchet, enforced by the compiler rather than by hope: raising the cap
/// does not fail a test that could be deleted with the edit, it fails to BUILD.
/// Lowering it — the only edit intended — leaves this untouched, so winding the
/// ratchet down stays the one-line change the constant promises.
///
/// Every other assertion about the cap is written relative to `CAP_BYTES`, so
/// the value itself is pinned here or nowhere: mutation testing raised the cap
/// to 8192 and the entire suite still passed, which is exactly the hole this
/// closes.
const _: () = assert!(
    CAP_BYTES <= 4096,
    "the agent-context cap is a floor-only ratchet: it may only ever be LOWERED, never raised. \
     A repo over the cap cuts its context, or scopes a rule with `paths:`."
);

/// Claude Code stops expanding `@path` imports after this many hops, so
/// anything deeper is not in the launch context and must not be charged.
const MAX_IMPORT_DEPTH: usize = 4;

/// One file that lands in the launch context, and what it costs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Contributor {
    pub(crate) bytes: u64,
    /// Path as reported to the user, plus why it is loaded.
    pub(crate) label: String,
}

/// Total launch-loaded bytes and the per-file breakdown, largest first. Public
/// for tests; `check` is what the subcommand calls.
pub(crate) fn collect(dir: &Path) -> Vec<Contributor> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut out: Vec<Contributor> = Vec::new();

    // Project memory. Both locations are alternatives for the same thing; if a
    // repo somehow has both, both are loaded, so both are charged.
    for rel in ["CLAUDE.md", ".claude/CLAUDE.md"] {
        let p = dir.join(rel);
        if let Some(text) = read_loaded(&p, &mut seen) {
            out.push(Contributor {
                bytes: text.len() as u64,
                label: rel.to_string(),
            });
            expand_imports(dir, &p, &text, rel, 1, &mut seen, &mut out);
        }
    }

    // Unscoped rules load at launch with project-memory priority. Scoped ones
    // (`paths:` frontmatter) load only when a matching file is read.
    for p in rules_files(&dir.join(".claude/rules")) {
        let Some(text) = read_loaded(&p, &mut seen) else {
            continue;
        };
        let rel = display_path(dir, &p);
        if is_path_scoped(&text) {
            continue;
        }
        out.push(Contributor {
            bytes: text.len() as u64,
            label: format!(
                "{rel} (unscoped rule — add `paths:` frontmatter to make it load on demand)"
            ),
        });
    }

    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.label.cmp(&b.label)));
    out
}

/// Returns `(total_bytes, offenders)`. Offenders is empty when clean. A total
/// exactly AT the cap passes — the cap is the largest permitted size, so only
/// `>` fails.
pub(crate) fn check(dir: &Path) -> (u64, Vec<String>) {
    let contributors = collect(dir);
    let total: u64 = contributors.iter().map(|c| c.bytes).sum();
    if total <= CAP_BYTES {
        return (total, Vec::new());
    }
    let over = total - CAP_BYTES;
    let mut lines = vec![format!(
        "ERROR: this repo loads {total} bytes of agent context at the start of every session \
         — {over} over the {CAP_BYTES}-byte cap. Cut {over} bytes; do NOT raise the cap (it is \
         a floor-only ratchet). Every file below is in the context window on every turn, \
         whether or not the turn needs it:"
    )];
    for c in &contributors {
        lines.push(format!("  {:>7}  {}", c.bytes, c.label));
    }
    lines.push(
        "Cut by asking of each line: would a capable agent looking at this repo get this WRONG, \
         or merely take a moment to find it? Directory layouts, dependency lists, architecture \
         overviews, which command CI runs, where tests live are all discoverable — cut them \
         (`claude doctor` runs the same trim). A rule repeated often enough to write down is a \
         rule worth linting, with the reference in the tool's --help. What survives is \
         irreversible hazards and rulings whose rationale is not recoverable from the code. A \
         rule that only matters for some files belongs in .claude/rules/ WITH `paths:` \
         frontmatter, which loads on demand and is not charged here. \
         See rainlanguage/rainix#298."
            .to_string(),
    );
    (total, lines)
}

/// Content of a regular file that has not been counted yet, as it lands in
/// context: block-level HTML comments stripped, because Claude Code strips them
/// before injection. `None` for anything absent, non-regular, unreadable or
/// already counted — an absent file is a PASS, this check caps context, it
/// never requires any file to exist.
///
/// The `is_file` guard rejects non-regular paths — a directory, a fifo — up
/// front rather than relying on the read to fail on them. Belt and braces: a
/// directory would fail the read anyway, but a fifo would BLOCK it, and a CI
/// job that hangs costs a runner for its whole timeout.
fn read_loaded(path: &Path, seen: &mut BTreeSet<PathBuf>) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    // Canonicalize so the same file reached two ways is charged once (and so an
    // import cycle terminates).
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(key) {
        return None;
    }
    std::fs::read_to_string(path)
        .ok()
        .map(|s| strip_block_html_comments(&s))
}

/// Charge every `@path` import reachable from `text`, depth-first, stopping at
/// `MAX_IMPORT_DEPTH` hops. A path that does not resolve to a file contributes
/// nothing (it is not loaded); flagging it is the separate path-existence check.
fn expand_imports(
    root: &Path,
    from: &Path,
    text: &str,
    from_label: &str,
    depth: usize,
    seen: &mut BTreeSet<PathBuf>,
    out: &mut Vec<Contributor>,
) {
    if depth > MAX_IMPORT_DEPTH {
        return;
    }
    for token in import_tokens(text) {
        let Some(target) = resolve_import(from, &token) else {
            continue;
        };
        let Some(body) = read_loaded(&target, seen) else {
            continue;
        };
        let label = format!("{} (imported by {from_label})", display_path(root, &target));
        out.push(Contributor {
            bytes: body.len() as u64,
            label: label.clone(),
        });
        expand_imports(root, &target, &body, &label, depth + 1, seen, out);
    }
}

/// `@path` tokens in `text`, ignoring anything inside a code span or fence
/// (a backticked `@README` is literal text, not an import).
///
/// Deliberately liberal: a token that does not resolve to an existing file
/// costs nothing, so over-matching is harmless while under-matching would leave
/// the cap evadable.
fn import_tokens(text: &str) -> Vec<String> {
    let masked = mask_code(text);
    let chars: Vec<char> = masked.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // An import starts a token: preceded by start-of-input or whitespace,
        // so an email address or a `foo@bar` suffix is not one.
        if chars[i] != '@' || (i > 0 && !chars[i - 1].is_whitespace()) {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }
        let token: String = chars[start..end].iter().collect();
        // Trailing sentence punctuation is prose, not path ("see @docs/x.md.").
        let token = token.trim_end_matches([',', ';', ':', ')', ']', '}', '"', '\'', '.']);
        if !token.is_empty() {
            out.push(token.to_string());
        }
        i = end;
    }
    out
}

/// Absolute path an import token names, or `None` when it cannot be resolved.
/// Relative paths resolve against the IMPORTING file's directory, not the
/// working directory.
fn resolve_import(from: &Path, token: &str) -> Option<PathBuf> {
    if let Some(rest) = token.strip_prefix("~/") {
        return Some(PathBuf::from(std::env::var("HOME").ok()?).join(rest));
    }
    let p = Path::new(token);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    Some(from.parent()?.join(p))
}

/// Blank out fenced code blocks and inline code spans so a *quoted* `@path` is
/// not read as an import. Offsets are not preserved — only import scanning uses
/// this, never the byte measurement.
fn mask_code(text: &str) -> String {
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

/// Drop HTML comments that occupy whole lines — Claude Code strips block-level
/// comments before injection, so charging for them would bill a repo for
/// context it never loads. A comment with prose on its line is left in place:
/// inline, it is not block-level.
fn strip_block_html_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(open) = rest.find("<!--") else {
            out.push_str(rest);
            return out;
        };
        let Some(close_rel) = rest[open..].find("-->") else {
            out.push_str(rest);
            return out;
        };
        let close = open + close_rel + "-->".len();
        let line_start = rest[..open].rfind('\n').map_or(0, |i| i + 1);
        let line_end = rest[close..].find('\n').map_or(rest.len(), |i| close + i);
        let block =
            rest[line_start..open].trim().is_empty() && rest[close..line_end].trim().is_empty();
        if block {
            out.push_str(&rest[..line_start]);
            rest = rest.get(line_end + 1..).unwrap_or("");
        } else {
            out.push_str(&rest[..close]);
            rest = &rest[close..];
        }
        if rest.is_empty() {
            return out;
        }
    }
}

/// Every `.md` under `.claude/rules/`, recursively, sorted so the report is
/// deterministic. Empty when the directory does not exist.
fn rules_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// True when YAML frontmatter declares a top-level `paths:` key — the rule is
/// conditional, loads only when a matching file is read, and is NOT launch
/// context. Frontmatter must be a `---` delimited block at the very top.
fn is_path_scoped(text: &str) -> bool {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    let mut found = false;
    for line in lines {
        if line.trim() == "---" {
            return found;
        }
        // Top-level keys are unindented; `paths:` nested under something else
        // is a different key.
        if !line.starts_with([' ', '\t', '-'])
            && line
                .split_once(':')
                .is_some_and(|(k, _)| k.trim() == "paths")
        {
            found = true;
        }
    }
    // No closing delimiter: not frontmatter at all.
    false
}

/// Path relative to the repo root when possible, so reports are readable and
/// stable across checkouts.
fn display_path(root: &Path, p: &Path) -> String {
    let rooted = std::fs::canonicalize(root);
    let target = std::fs::canonicalize(p);
    if let (Ok(r), Ok(t)) = (rooted, target) {
        if let Ok(rel) = t.strip_prefix(&r) {
            return rel.display().to_string();
        }
    }
    p.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-agentctx-test-{}-{}",
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

    fn total(dir: &Path) -> u64 {
        check(dir).0
    }

    fn offenders(dir: &Path) -> Vec<String> {
        check(dir).1
    }

    // ---- the four cases the cap exists for -------------------------------

    #[test]
    fn absent_claude_md_passes() {
        let d = tmp_dir();
        write(&d, "README.md", &"x".repeat(CAP_BYTES as usize * 2));
        assert_eq!(total(&d), 0);
        assert!(offenders(&d).is_empty());
    }

    #[test]
    fn under_cap_passes() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", &"x".repeat(CAP_BYTES as usize - 1));
        assert!(offenders(&d).is_empty());
    }

    #[test]
    fn exactly_at_cap_passes() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", &"x".repeat(CAP_BYTES as usize));
        assert_eq!(total(&d), CAP_BYTES);
        assert!(offenders(&d).is_empty());
    }

    #[test]
    fn one_byte_over_cap_fails() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", &"x".repeat(CAP_BYTES as usize + 1));
        assert!(!offenders(&d).is_empty());
    }

    #[test]
    fn failure_names_total_cap_overage_and_every_contributor() {
        // Sizes are stated relative to CAP_BYTES, never as literals, so winding
        // the ratchet down stays the one-line change the constant promises.
        let d = tmp_dir();
        let import = "@big.md\n";
        let memory = CAP_BYTES as usize;
        let imported = 2000;
        write(&d, "CLAUDE.md", &format!("{import}{}", "x".repeat(memory)));
        write(&d, "big.md", &"y".repeat(imported));
        let memory_bytes = import.len() + memory;
        let total_bytes = (memory_bytes + imported) as u64;
        let over = total_bytes - CAP_BYTES;
        let (t, off) = check(&d);
        assert_eq!(t, total_bytes);
        let joined = off.join("\n");
        assert!(joined.contains(&format!("{total_bytes} bytes")), "{joined}");
        assert!(
            joined.contains(&format!("{CAP_BYTES}-byte cap")),
            "{joined}"
        );
        assert!(joined.contains(&format!("{over} over")), "{joined}");
        assert!(joined.contains("CLAUDE.md"), "{joined}");
        assert!(
            joined.contains("big.md (imported by CLAUDE.md)"),
            "{joined}"
        );
        // breakdown is largest-first so the fix is obvious
        let claude_at = joined
            .find(&format!("  {memory_bytes}  CLAUDE.md"))
            .unwrap();
        let big_at = joined.find(&format!("  {imported}  big.md")).unwrap();
        assert!(claude_at < big_at, "{joined}");
    }

    // ---- imports: the hole a file-size check leaves ----------------------

    #[test]
    fn imports_are_charged_transitively() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "@docs/a.md");
        write(&d, "docs/a.md", &format!("@b.md\n{}", "a".repeat(1000)));
        write(&d, "docs/b.md", &"b".repeat(2000));
        // 10 + 1006 + 2000
        assert_eq!(total(&d), 3016);
    }

    #[test]
    fn a_small_file_importing_a_large_one_does_not_pass() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "@docs/conventions.md\n");
        write(&d, "docs/conventions.md", &"x".repeat(CAP_BYTES as usize));
        assert!(!offenders(&d).is_empty());
    }

    #[test]
    fn relative_imports_resolve_against_the_importing_file() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "@docs/a.md");
        // `@b.md` inside docs/a.md means docs/b.md, NOT ./b.md
        write(&d, "docs/a.md", "@b.md");
        write(&d, "docs/b.md", &"b".repeat(500));
        write(&d, "b.md", &"w".repeat(9999));
        assert_eq!(total(&d), 10 + 5 + 500);
    }

    #[test]
    fn absolute_imports_resolve_as_given() {
        let d = tmp_dir();
        let other = tmp_dir();
        write(&other, "shared.md", &"s".repeat(700));
        write(
            &d,
            "CLAUDE.md",
            &format!("@{}", other.join("shared.md").display()),
        );
        assert_eq!(
            total(&d),
            700 + std::fs::metadata(d.join("CLAUDE.md")).unwrap().len()
        );
    }

    #[test]
    fn imports_stop_after_four_hops() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "@l1.md");
        for i in 1..=4 {
            write(&d, &format!("l{i}.md"), &format!("@l{}.md", i + 1));
        }
        // l5.md is the 5th hop: past the limit, so not loaded and not charged.
        write(&d, "l5.md", &"z".repeat(9999));
        assert_eq!(total(&d), 6 + 6 * 4);
    }

    #[test]
    fn an_import_cycle_terminates_and_charges_each_file_once() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "@a.md");
        write(&d, "a.md", "@CLAUDE.md\n@a.md");
        assert_eq!(total(&d), 5 + 16);
    }

    #[test]
    fn a_missing_import_target_costs_nothing() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "@nope/gone.md");
        assert_eq!(total(&d), 13);
        assert!(offenders(&d).is_empty());
    }

    #[test]
    fn quoted_at_paths_are_not_imports() {
        // code span, fenced block, and an email — none of these load anything
        let body = "`@big.md` is literal\n\n```\n@big.md\n```\n\nmail me@big.md\n";
        let d = tmp_dir();
        write(&d, "CLAUDE.md", body);
        write(&d, "big.md", &"x".repeat(9999));
        assert_eq!(total(&d), body.len() as u64);
        assert!(offenders(&d).is_empty());
    }

    #[test]
    fn tilde_fences_and_multi_backtick_spans_also_mask() {
        let text = "~~~\n@a.md\n~~~\n``@b.md``\n@c.md\n";
        assert_eq!(import_tokens(text), vec!["c.md".to_string()]);
    }

    #[test]
    fn trailing_prose_punctuation_is_not_part_of_the_path() {
        assert_eq!(
            import_tokens("see @docs/x.md, and @y.md.\n"),
            vec!["docs/x.md".to_string(), "y.md".to_string()]
        );
    }

    // ---- .claude/rules: the other hole -----------------------------------

    #[test]
    fn unscoped_rules_are_charged() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "hi\n");
        write(&d, ".claude/rules/style.md", &"r".repeat(2000));
        write(&d, ".claude/rules/deep/more.md", &"m".repeat(2100));
        let (t, off) = check(&d);
        assert_eq!(t, 3 + 2000 + 2100);
        assert!(!off.is_empty());
        assert!(
            off.join("\n").contains(".claude/rules/deep/more.md"),
            "{off:?}"
        );
    }

    #[test]
    fn path_scoped_rules_are_not_charged() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "hi\n");
        write(
            &d,
            ".claude/rules/sol.md",
            &format!("---\npaths:\n  - \"**/*.sol\"\n---\n{}", "r".repeat(9999)),
        );
        assert_eq!(total(&d), 3);
        assert!(offenders(&d).is_empty());
    }

    #[test]
    fn frontmatter_without_paths_is_still_launch_context() {
        let d = tmp_dir();
        write(
            &d,
            ".claude/rules/x.md",
            &format!("---\ndescription: x\n---\n{}", "r".repeat(5000)),
        );
        assert!(!offenders(&d).is_empty());
    }

    #[test]
    fn paths_key_detection() {
        assert!(is_path_scoped("---\npaths:\n  - \"**/*.sol\"\n---\nbody"));
        assert!(is_path_scoped(
            "---\ndescription: x\npaths: \"src/**\"\n---\n"
        ));
        // no frontmatter at all
        assert!(!is_path_scoped("paths: src/**\nbody"));
        // frontmatter that never closes is not frontmatter
        assert!(!is_path_scoped("---\npaths: src/**\nbody"));
        // nested under another key, not a top-level scope declaration
        assert!(!is_path_scoped("---\nmeta:\n  paths: src/**\n---\n"));
        assert!(!is_path_scoped("---\ndescription: x\n---\nbody"));
    }

    #[test]
    fn non_md_files_under_rules_are_ignored() {
        let d = tmp_dir();
        write(&d, ".claude/rules/notes.txt", &"t".repeat(9999));
        assert_eq!(total(&d), 0);
    }

    // ---- what must NOT be charged ---------------------------------------

    #[test]
    fn nested_claude_md_is_on_demand_not_launch_context() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", "hi\n");
        write(&d, "packages/app/CLAUDE.md", &"n".repeat(9999));
        assert_eq!(total(&d), 3);
    }

    #[test]
    fn claude_local_md_is_personal_and_not_charged() {
        let d = tmp_dir();
        write(&d, "CLAUDE.local.md", &"l".repeat(9999));
        assert_eq!(total(&d), 0);
    }

    #[test]
    fn block_html_comments_are_stripped_because_they_never_load() {
        let d = tmp_dir();
        let comment = format!("<!--\n{}\n-->\n", "c".repeat(9999));
        write(&d, "CLAUDE.md", &format!("{comment}kept\n"));
        assert_eq!(total(&d), 5);
        assert!(offenders(&d).is_empty());
    }

    #[test]
    fn inline_html_comments_are_not_block_level_and_still_count() {
        let text = "before <!-- x --> after\n";
        assert_eq!(strip_block_html_comments(text), text);
    }

    #[test]
    fn an_unterminated_html_comment_is_left_alone() {
        let text = "<!-- never closed\nbody\n";
        assert_eq!(strip_block_html_comments(text), text);
    }

    // ---- both memory locations -------------------------------------------

    #[test]
    fn dot_claude_claude_md_is_the_other_memory_location() {
        let d = tmp_dir();
        write(&d, ".claude/CLAUDE.md", &"x".repeat(CAP_BYTES as usize + 1));
        assert!(!offenders(&d).is_empty());
    }

    #[test]
    fn both_memory_locations_are_charged_together() {
        let d = tmp_dir();
        write(&d, "CLAUDE.md", &"x".repeat(2500));
        write(&d, ".claude/CLAUDE.md", &"y".repeat(2500));
        assert_eq!(total(&d), 5000);
        assert!(!offenders(&d).is_empty());
    }

    // ---- measurement ------------------------------------------------------

    #[test]
    fn a_directory_named_claude_md_is_not_a_file() {
        let d = tmp_dir();
        std::fs::create_dir_all(d.join("CLAUDE.md")).unwrap();
        assert_eq!(total(&d), 0);
        assert!(offenders(&d).is_empty());
    }

    #[test]
    fn size_is_bytes_not_chars() {
        let d = tmp_dir();
        // 3 UTF-8 bytes per char, so under the cap in chars but over it in bytes
        let body = "€".repeat(CAP_BYTES as usize / 2);
        assert!(body.chars().count() < CAP_BYTES as usize);
        write(&d, "CLAUDE.md", &body);
        assert_eq!(total(&d), body.len() as u64);
        assert!(!offenders(&d).is_empty());
    }

    /// The depth limit is Claude Code's, not ours: charging deeper would bill
    /// for context that never loads, charging shallower would leave a hole.
    #[test]
    fn import_depth_matches_claude_code() {
        assert_eq!(MAX_IMPORT_DEPTH, 4);
    }
}
