//! `prompt-cap` — byte cap on the prompt files a repo points this at
//! (rainlanguage/rainix#310).
//!
//! `agent-context-cap` caps what Claude Code loads before turn one. A runner
//! prompt is the same tax paid harder: a shell script reads it whole, so every
//! byte sits in the window for every turn of a run that can be hundreds of
//! turns long, and nothing else in CI measures it — to every other check a
//! prompt is inert data. So it is the same check, over the same traversal
//! ([`crate::context_bytes`]), with three things different.
//!
//! **Roots are a glob, and the cap is on their TOTAL.** Which files are prompts
//! is per-repo, so the caller names them; a glob rather than a list because a
//! per-file cap is evaded by splitting the file, and because a new prompt file
//! must be charged rather than escape by not being named.
//!
//! **Nothing is stripped.** A shell script reads the bytes on disk; there is no
//! loader dropping HTML comments the way Claude Code does.
//!
//! **A reference is any repo file the prompt names**, not declared syntax, and
//! it reaches ONE hop. Moving text into `docs/foo.md` and writing "read it
//! first" puts those bytes in the window exactly as `@docs/foo.md` does, so
//! they are charged. What `docs/foo.md` goes on to mention is not charged: an
//! import expands transitively because the LOADER expands it, and no loader is
//! involved here. Following further reads paths out of shell scripts, lock
//! files and logs and bills a prompt for the whole repo, which is a number
//! nobody can act on.
//!
//! Charging what a prompt names over-charges where `agent-context-cap` cannot:
//! a path in a prompt may be an instruction to read, an example, or somewhere
//! to WRITE. Three guards keep it honest — code spans and fences are masked, so
//! documenting a path costs nothing; a token that resolves to no file costs
//! nothing; and a file outside the repo (or generated at run time) is never
//! charged. The failure message says so, because "presumed loaded" is a claim
//! the reader must be able to check.
//!
//! The cap itself comes from the consuming repo, since the files and their
//! weights are per-repo. That repo can raise it — accepted: the raise is a line
//! in its own workflow call, in a PR diff, where a reviewer sees it. Floor-only
//! is stated intent here, not a mechanism. `agent_context_cap::CAP_BYTES` keeps
//! its compile-time ratchet, which works only because that number lives here.

use crate::context_bytes::{self, Charge, Walk};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One hop. A prompt telling the agent to read a file is why that file is in
/// the window; the paths THAT file happens to contain are nobody's instruction.
/// A repo that wants a whole directory charged widens the glob, which is the
/// honest way to say so.
const MAX_REFERENCE_DEPTH: usize = 1;

/// What a prompt costs and what it drags in: any named repo file, resolved
/// against the naming file and then the repo root, with nothing stripped.
fn charge() -> Charge {
    Charge {
        tokens: reference_tokens,
        resolve: resolve_reference,
        strip: |s| s.to_string(),
        verb: "referenced by",
        max_depth: MAX_REFERENCE_DEPTH,
    }
}

/// Split the `--paths` value into glob patterns: one per line, or comma
/// separated, so a YAML block scalar and a one-liner both pass through
/// unchanged. Blank lines and `#` comments are dropped.
pub(crate) fn parse_patterns(spec: &str) -> Vec<String> {
    spec.split(['\n', ','])
        .map(|p| p.trim().trim_start_matches("./").trim())
        .filter(|p| !p.is_empty() && !p.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// True when `name` matches one glob segment: `*` any run of characters, `?`
/// exactly one, everything else literal. Neither wildcard crosses a `/`, since
/// a segment never contains one — `**` is what spans directories.
pub(crate) fn segment_matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut i, mut j) = (0, 0);
    // Where to resume from if the current `*` turns out to have matched too
    // little: the classic backtracking match, linear in practice.
    let mut star: Option<usize> = None;
    let mut retry = 0;
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            i = s + 1;
            retry += 1;
            j = retry;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

/// Files under `root` matching any pattern, as paths relative to `root`, sorted
/// and each listed once however many patterns hit it.
///
/// `**` spans any number of directories (including none); a plain segment only
/// descends where it matches, so a pattern with no `**` never walks the tree.
/// `.git` is never entered: nothing in it is a prompt, and it is the one
/// directory big enough for the walk to be felt.
pub(crate) fn matching(root: &Path, patterns: &[String]) -> Vec<PathBuf> {
    let mut out = BTreeSet::new();
    for pattern in patterns {
        let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            continue;
        }
        expand(
            root,
            &segments,
            Path::new(""),
            &mut BTreeSet::new(),
            &mut out,
        );
    }
    out.into_iter().collect()
}

/// Walk `dir` for the remaining `segments`, accumulating matches as paths
/// relative to the search root. `guard` holds the canonical directories already
/// entered under a `**`, so a symlink loop terminates instead of hanging the
/// job until the runner timeout.
fn expand(
    dir: &Path,
    segments: &[&str],
    rel: &Path,
    guard: &mut BTreeSet<PathBuf>,
    out: &mut BTreeSet<PathBuf>,
) {
    let Some((head, tail)) = segments.split_first() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    if *head == "**" {
        // Zero directories consumed: the rest of the pattern may match here.
        expand(dir, tail, rel, guard, out);
    }
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let child_rel = rel.join(&name);
        if path.is_dir() {
            if name == ".git" {
                continue;
            }
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if *head == "**" {
                if guard.insert(key) {
                    expand(&path, segments, &child_rel, guard, out);
                }
            } else if !tail.is_empty() && segment_matches(head, &name) {
                expand(&path, tail, &child_rel, guard, out);
            }
            // A trailing `**` takes every file under here, by the same rule:
            // two stars match any name.
        } else if tail.is_empty() && segment_matches(head, &name) && path.is_file() {
            out.insert(child_rel);
        }
    }
}

/// Tokens in a prompt that might name a file, ignoring anything inside a code
/// span or fence — a path shown as an example is documentation, not a load.
///
/// A candidate must contain `.` or `/`: prose is full of bare words, and a
/// reference to a file all but always carries an extension or a directory. Over
/// -matching beyond that is safe, since a token resolving to nothing costs
/// nothing, so wrapping punctuation is trimmed rather than parsed.
pub(crate) fn reference_tokens(text: &str) -> Vec<String> {
    context_bytes::mask_code(text)
        .split_whitespace()
        .map(|t| {
            t.trim_matches(|c: char| {
                matches!(
                    c,
                    '(' | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | '"'
                        | '\''
                        | '`'
                        | '*'
                        | '_'
                        | ','
                        | ';'
                        | ':'
                        | '!'
                        | '?'
                )
            })
            .trim_end_matches('.')
        })
        .filter(|t| !t.is_empty() && t.contains(['.', '/']))
        .map(str::to_string)
        .collect()
}

/// The repo file a token names, or `None`. Tried against the directory of the
/// file the token was found in, then against the repo root — a prompt names
/// paths both ways, and the root is what a runner's working directory is.
///
/// Only files INSIDE the repo are charged. A path that escapes it (absolute,
/// `..`, a symlink out) is a file this check cannot measure or ask anyone to
/// cut, so it costs nothing here.
pub(crate) fn resolve_reference(root: &Path, from: &Path, token: &str) -> Option<PathBuf> {
    let bases = [from.parent()?.to_path_buf(), root.to_path_buf()];
    let canonical_root = std::fs::canonicalize(root).ok()?;
    bases.iter().find_map(|base| {
        let target = std::fs::canonicalize(base.join(token)).ok()?;
        (target.is_file() && target.starts_with(&canonical_root)).then_some(target)
    })
}

/// Total charged bytes and the offender lines (empty when clean). A total
/// exactly AT the cap passes — the cap is the largest permitted size, so only
/// `>` fails. `Err` when the caller has named nothing measurable.
pub(crate) fn check(
    root: &Path,
    patterns: &[String],
    cap: u64,
) -> Result<(u64, Vec<String>), String> {
    if patterns.is_empty() {
        return Err("--paths named no globs — point this at the repo's prompt files".to_string());
    }
    let files = matching(root, patterns);
    if files.is_empty() {
        return Err(format!(
            "no file matched {} — fix the globs or drop the step; as written it caps nothing",
            patterns.join(" ")
        ));
    }

    let charge = charge();
    let mut walk = Walk::new(root, &charge);
    for rel in &files {
        walk.root_file(&root.join(rel), &rel.display().to_string());
    }
    let contributors = walk.finish();
    let total = context_bytes::total(&contributors);
    if total <= cap {
        return Ok((total, Vec::new()));
    }

    let over = total - cap;
    let mut lines = vec![format!(
        "ERROR: the prompt files matching {} load {total} bytes — {over} over the {cap}-byte cap. \
         A prompt is read whole at launch and re-read on every turn of the run, so a byte here is \
         paid once per turn, by every run. Cut {over} bytes:",
        patterns.join(" ")
    )];
    lines.extend(context_bytes::breakdown(&contributors));
    lines.push(
        "Any repo file the prompt NAMES is charged with it, because telling the agent to read a \
         file puts that file in the window as surely as pasting it in — one hop only, so what \
         that file goes on to mention is free. A path inside a code span or fence is not charged, \
         nor is one that resolves to no file, nor one outside the repo or written at run time — \
         so a reference that is documentation, an example, or an output path costs nothing. Cut \
         by asking what the run would get WRONG without each line. \
         Lowering the cap where the workflow sets it is the ratchet; raising it is a line in a \
         PR diff. See rainlanguage/rainix#310."
            .to_string(),
    );
    Ok((total, lines))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-promptcap-test-{}-{}",
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

    fn globs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Cap of 0 so any charge fails: what is charged is the question, not what
    /// the number is.
    fn total(dir: &Path, patterns: &[&str]) -> u64 {
        check(dir, &globs(patterns), 0).unwrap().0
    }

    fn matched(dir: &Path, patterns: &[&str]) -> Vec<String> {
        matching(dir, &globs(patterns))
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    // ---- globs -----------------------------------------------------------

    #[test]
    fn patterns_parse_from_lines_and_commas() {
        assert_eq!(
            parse_patterns("*.txt\nprompts/*.md"),
            globs(&["*.txt", "prompts/*.md"])
        );
        assert_eq!(parse_patterns("*.txt, *.md"), globs(&["*.txt", "*.md"]));
        assert_eq!(
            parse_patterns("  ./*.txt \n\n# a comment\n"),
            globs(&["*.txt"])
        );
        assert!(parse_patterns("\n#nothing\n").is_empty());
    }

    #[test]
    fn segment_wildcards() {
        assert!(segment_matches("*", "anything"));
        assert!(segment_matches("*prompt*.txt", "campaign-prompt.txt"));
        assert!(segment_matches("?.md", "a.md"));
        assert!(segment_matches("a*b*c", "azzbzzc"));
        assert!(segment_matches("prompt.txt", "prompt.txt"));
        assert!(!segment_matches("?.md", "ab.md"));
        assert!(!segment_matches("*.txt", "a.txtx"));
        assert!(!segment_matches("a*b*c", "azzbzz"));
        assert!(!segment_matches("prompt.txt", "Prompt.txt"), "case matters");
    }

    #[test]
    fn a_glob_matches_files_not_directories() {
        let d = tmp_dir();
        write(&d, "a-prompt.txt", "x");
        std::fs::create_dir_all(d.join("b-prompt.txt")).unwrap();
        assert_eq!(matched(&d, &["*prompt*"]), vec!["a-prompt.txt"]);
    }

    #[test]
    fn a_star_does_not_cross_a_directory_boundary() {
        let d = tmp_dir();
        write(&d, "a.txt", "x");
        write(&d, "sub/b.txt", "x");
        assert_eq!(matched(&d, &["*.txt"]), vec!["a.txt"]);
        assert_eq!(matched(&d, &["sub/*.txt"]), vec!["sub/b.txt"]);
    }

    #[test]
    fn double_star_spans_any_depth_including_none() {
        let d = tmp_dir();
        write(&d, "a.txt", "x");
        write(&d, "one/b.txt", "x");
        write(&d, "one/two/c.txt", "x");
        assert_eq!(
            matched(&d, &["**/*.txt"]),
            vec!["a.txt", "one/b.txt", "one/two/c.txt"]
        );
    }

    #[test]
    fn a_trailing_double_star_takes_every_file_under_it() {
        let d = tmp_dir();
        write(&d, "prompts/a.txt", "x");
        write(&d, "prompts/deep/b.md", "x");
        write(&d, "elsewhere.txt", "x");
        assert_eq!(
            matched(&d, &["prompts/**"]),
            vec!["prompts/a.txt", "prompts/deep/b.md"]
        );
    }

    #[test]
    fn a_file_matched_by_two_patterns_is_listed_once() {
        let d = tmp_dir();
        write(&d, "a.txt", "x");
        assert_eq!(matched(&d, &["*.txt", "a.*", "**/*.txt"]), vec!["a.txt"]);
    }

    #[test]
    fn the_git_directory_is_never_walked() {
        let d = tmp_dir();
        write(&d, ".git/objects/pack.txt", "x");
        write(&d, "a.txt", "x");
        assert_eq!(matched(&d, &["**/*.txt"]), vec!["a.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_cycle_terminates() {
        let d = tmp_dir();
        write(&d, "deep/a.txt", "x");
        std::os::unix::fs::symlink(d.join("deep"), d.join("deep/self")).unwrap();
        assert_eq!(matched(&d, &["**/*.txt"]), vec!["deep/a.txt"]);
    }

    // ---- what is charged --------------------------------------------------

    #[test]
    fn every_matched_file_is_charged_so_splitting_one_prompt_changes_nothing() {
        let d = tmp_dir();
        write(&d, "one-prompt.txt", &"x".repeat(300));
        assert_eq!(total(&d, &["*prompt*.txt"]), 300);
        // the same text, split three ways, still costs 300
        std::fs::remove_file(d.join("one-prompt.txt")).unwrap();
        write(&d, "a-prompt.txt", &"x".repeat(100));
        write(&d, "b-prompt.txt", &"x".repeat(100));
        write(&d, "c-prompt.txt", &"x".repeat(100));
        assert_eq!(total(&d, &["*prompt*.txt"]), 300);
    }

    #[test]
    fn a_file_the_prompt_names_is_charged_but_not_what_that_file_mentions() {
        let d = tmp_dir();
        write(&d, "prompt.txt", "First read docs/style.md then work.\n");
        write(
            &d,
            "docs/style.md",
            &format!("Also read deep.md\n{}", "s".repeat(1000)),
        );
        // Charging deep.md would bill the prompt for a path nobody instructed
        // anyone to read — that route ends at a lock file or a log.
        write(&d, "docs/deep.md", &"d".repeat(500));
        let body = std::fs::metadata(d.join("prompt.txt")).unwrap().len();
        assert_eq!(total(&d, &["prompt.txt"]), body + 1018);
    }

    #[test]
    fn a_reference_resolves_against_the_repo_root_too() {
        let d = tmp_dir();
        // `style.md` names the root file, not one beside the prompt.
        write(&d, "prompts/p.txt", "Read style.md\n");
        write(&d, "style.md", &"s".repeat(400));
        assert_eq!(total(&d, &["prompts/*.txt"]), 14 + 400);
    }

    #[test]
    fn a_quoted_path_is_documentation_and_costs_nothing() {
        let d = tmp_dir();
        let body = "Write results to `out.md`.\n\n```\nout.md\n```\n";
        write(&d, "prompt.txt", body);
        write(&d, "out.md", &"o".repeat(9999));
        assert_eq!(total(&d, &["prompt.txt"]), body.len() as u64);
    }

    #[test]
    fn a_bare_word_is_prose_even_when_a_file_shares_its_name() {
        let d = tmp_dir();
        let body = "Keep notes as you go\n";
        write(&d, "prompt.txt", body);
        write(&d, "notes", &"n".repeat(9999));
        assert_eq!(total(&d, &["prompt.txt"]), body.len() as u64);
    }

    #[test]
    fn a_path_that_resolves_to_nothing_costs_nothing() {
        let d = tmp_dir();
        let body = "Read docs/missing.md and {{WORK_DIR}}/notes.md\n";
        write(&d, "prompt.txt", body);
        assert_eq!(total(&d, &["prompt.txt"]), body.len() as u64);
    }

    #[test]
    fn a_file_outside_the_repo_is_not_charged() {
        let d = tmp_dir();
        let other = tmp_dir();
        write(&other, "outside.md", &"o".repeat(9999));
        let body = format!(
            "Read {} and ../{}/outside.md\n",
            other.join("outside.md").display(),
            other.file_name().unwrap().to_string_lossy()
        );
        write(&d, "prompt.txt", &body);
        assert_eq!(total(&d, &["prompt.txt"]), body.len() as u64);
    }

    #[test]
    fn a_directory_named_by_a_prompt_is_not_charged() {
        let d = tmp_dir();
        let body = "Work in docs/ then stop\n";
        write(&d, "prompt.txt", body);
        write(&d, "docs/a.md", &"a".repeat(9999));
        assert_eq!(total(&d, &["prompt.txt"]), body.len() as u64);
    }

    #[test]
    fn a_file_referenced_by_two_prompts_is_charged_once() {
        let d = tmp_dir();
        write(&d, "a-prompt.txt", "read shared.md\n");
        write(&d, "b-prompt.txt", "read shared.md\n");
        write(&d, "shared.md", &"s".repeat(1000));
        assert_eq!(total(&d, &["*prompt*.txt"]), 15 + 15 + 1000);
    }

    #[test]
    fn a_prompt_that_names_itself_is_charged_once() {
        let d = tmp_dir();
        write(&d, "prompt.txt", "see prompt.txt above\n");
        assert_eq!(total(&d, &["prompt.txt"]), 21);
    }

    #[test]
    fn a_chain_of_references_is_charged_one_hop_deep() {
        let d = tmp_dir();
        write(&d, "prompt.txt", "read l1.md\n");
        for i in 1..=4 {
            write(&d, &format!("l{i}.md"), &format!("read l{}.md\n", i + 1));
        }
        write(&d, "l5.md", &"z".repeat(9999));
        assert_eq!(total(&d, &["prompt.txt"]), 11 + 11);
    }

    #[test]
    fn nothing_is_stripped_from_a_prompt() {
        let d = tmp_dir();
        // No loader drops these bytes: a shell script reads the file whole.
        let body = "<!--\nnote to self\n-->\nkeep\n";
        write(&d, "prompt.txt", body);
        assert_eq!(total(&d, &["prompt.txt"]), body.len() as u64);
    }

    #[test]
    fn size_is_bytes_not_chars() {
        let d = tmp_dir();
        let body = "€".repeat(100);
        write(&d, "prompt.txt", &body);
        assert_eq!(total(&d, &["prompt.txt"]), 300);
    }

    // ---- the cap ----------------------------------------------------------

    #[test]
    fn under_and_exactly_at_cap_pass_and_one_byte_over_fails() {
        let d = tmp_dir();
        write(&d, "prompt.txt", &"x".repeat(100));
        assert!(check(&d, &globs(&["prompt.txt"]), 101)
            .unwrap()
            .1
            .is_empty());
        assert!(check(&d, &globs(&["prompt.txt"]), 100)
            .unwrap()
            .1
            .is_empty());
        assert!(!check(&d, &globs(&["prompt.txt"]), 99).unwrap().1.is_empty());
    }

    #[test]
    fn failure_names_total_cap_overage_and_every_contributor_largest_first() {
        let d = tmp_dir();
        write(&d, "prompt.txt", "read big.md\n");
        write(&d, "big.md", &"b".repeat(2000));
        let (total, off) = check(&d, &globs(&["prompt.txt"]), 500).unwrap();
        assert_eq!(total, 12 + 2000);
        let joined = off.join("\n");
        assert!(joined.contains("2012 bytes"), "{joined}");
        assert!(joined.contains("500-byte cap"), "{joined}");
        assert!(joined.contains("1512 over"), "{joined}");
        assert!(
            joined.contains("big.md (referenced by prompt.txt)"),
            "{joined}"
        );
        let big_at = joined.find("  2000  big.md").unwrap();
        let prompt_at = joined.find("  12  prompt.txt").unwrap();
        assert!(big_at < prompt_at, "largest first: {joined}");
    }

    #[test]
    fn the_failure_states_what_a_named_path_costs_and_what_it_does_not() {
        let d = tmp_dir();
        write(&d, "prompt.txt", "x");
        let off = check(&d, &globs(&["prompt.txt"]), 0).unwrap().1;
        let joined = off.join("\n");
        assert!(joined.contains("NAMES"), "{joined}");
        assert!(joined.contains("code span"), "{joined}");
        assert!(joined.contains("outside the repo"), "{joined}");
    }

    #[test]
    fn a_glob_matching_nothing_is_an_error_not_a_pass() {
        let d = tmp_dir();
        write(&d, "prompt.txt", "x");
        let e = check(&d, &globs(&["prompts/*.txt"]), 10).unwrap_err();
        assert!(e.contains("no file matched"), "{e}");
        assert!(e.contains("prompts/*.txt"), "{e}");
    }

    #[test]
    fn no_globs_at_all_is_an_error_not_a_pass() {
        let d = tmp_dir();
        let e = check(&d, &[], 10).unwrap_err();
        assert!(e.contains("named no globs"), "{e}");
    }
}
