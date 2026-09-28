use std::collections::BTreeSet;
use std::path::Path;

/// Line prefix a codegen hook prints to declare a path. `forge script` indents
/// `console.log` output under `== Logs ==`, so lines are matched trimmed.
pub(crate) const SENTINEL: &str = "rainix-codegen ";

#[derive(Default, PartialEq, Eq, Debug)]
pub(crate) struct Declaration {
    pub(crate) owns: BTreeSet<String>,
    pub(crate) wrote: BTreeSet<String>,
    pub(crate) malformed: Vec<String>,
}

pub(crate) fn parse(log: &str) -> Declaration {
    let mut out = Declaration::default();
    for line in log.lines() {
        let Some(rest) = line.trim().strip_prefix(SENTINEL) else {
            continue;
        };
        match rest.split_once(char::is_whitespace) {
            Some(("owns", path)) if !path.trim().is_empty() => {
                out.owns.insert(path.trim().to_string());
            }
            Some(("wrote", path)) if !path.trim().is_empty() => {
                out.wrote.insert(path.trim().to_string());
            }
            // Fail-closed: a verb this check does not know is a declaration it
            // is silently not making, which is the defect one level up.
            _ => out.malformed.push(line.trim().to_string()),
        }
    }
    out
}

pub(crate) fn offences(
    owns: &BTreeSet<String>,
    wrote: &BTreeSet<String>,
    present: &BTreeSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for path in owns {
        match (wrote.contains(path), present.contains(path)) {
            (false, true) => out.push(format!(
                "the codegen hooks declare {path} generated, but nothing wrote it on this run. \
                 The committed copy is left exactly as it was, so regenerating and diffing \
                 passes without ever checking it — its emitter is dead. Restore the emitter, or \
                 stop declaring the path if it is genuinely no longer generated."
            )),
            (false, false) => out.push(format!(
                "the codegen hooks declare {path} generated, but nothing wrote it and no such \
                 file exists after the run."
            )),
            (true, false) => out.push(format!(
                "a codegen hook reported writing {path}, but no such file exists after the run."
            )),
            (true, true) => {}
        }
    }
    out
}

pub(crate) fn run(root: &Path, log: &Path) {
    let text = match std::fs::read_to_string(log) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => crate::fail(&format!(
            "codegen-declaration: failed to read {}: {e}",
            log.display()
        )),
    };
    let declaration = parse(&text);

    if !declaration.malformed.is_empty() {
        for line in &declaration.malformed {
            eprintln!("::error::codegen-declaration: unreadable declaration: {line}");
        }
        std::process::exit(1);
    }

    if declaration.owns.is_empty() {
        println!(
            "codegen-declaration: clean — this repo's codegen declares no generated paths, so \
             nothing here can tell a live emitter from one that has died"
        );
        return;
    }

    let present: BTreeSet<String> = declaration
        .owns
        .union(&declaration.wrote)
        .filter(|path| root.join(path).exists())
        .cloned()
        .collect();
    let offences = offences(&declaration.owns, &declaration.wrote, &present);

    if !offences.is_empty() {
        for line in &offences {
            eprintln!("::error::codegen-declaration: {line}");
        }
        std::process::exit(1);
    }

    println!(
        "codegen-declaration: clean — {} declared generated paths, each written this run",
        declaration.owns.len()
    );
    for path in declaration.wrote.difference(&declaration.owns) {
        println!("codegen-declaration: note — {path} was written but not declared, so nothing will notice if its emitter dies");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn forge_indented_lines_parse_into_both_sets() {
        let log = "== Logs ==\n  rainix-codegen owns src/a.sol\n  rainix-codegen wrote src/a.sol\n";
        let d = parse(log);
        assert_eq!(d.owns, set(&["src/a.sol"]));
        assert_eq!(d.wrote, set(&["src/a.sol"]));
        assert!(d.malformed.is_empty());
    }

    #[test]
    fn a_line_that_merely_mentions_the_sentinel_is_not_a_declaration() {
        let d = parse("error: expected `rainix-codegen owns src/a.sol`\n");
        assert_eq!(d, Declaration::default());
    }

    #[test]
    fn repeated_declarations_of_one_path_are_one_path() {
        let d = parse("rainix-codegen wrote src/a.sol\nrainix-codegen wrote src/a.sol\n");
        assert_eq!(d.wrote, set(&["src/a.sol"]));
    }

    #[test]
    fn an_unknown_verb_is_malformed_rather_than_ignored() {
        let d = parse("rainix-codegen skipped src/a.sol\n");
        assert_eq!(d.malformed, vec!["rainix-codegen skipped src/a.sol"]);
    }

    #[test]
    fn a_verb_with_no_path_is_malformed() {
        let d = parse("rainix-codegen owns\nrainix-codegen wrote   \n");
        assert_eq!(d.malformed.len(), 2);
        assert!(d.owns.is_empty());
        assert!(d.wrote.is_empty());
    }

    #[test]
    fn a_declared_path_nothing_wrote_is_an_offence_though_it_is_on_disk() {
        let owns = set(&["src/lib/LibReleasedSuites.sol", "src/generated/A.sol"]);
        let wrote = set(&["src/generated/A.sol"]);
        let off = offences(&owns, &wrote, &owns);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("src/lib/LibReleasedSuites.sol"));
        assert!(off[0].contains("emitter is dead"));
    }

    #[test]
    fn a_declared_path_that_is_not_on_disk_says_so_instead() {
        let owns = set(&["src/generated/Gone.sol"]);
        let off = offences(&owns, &BTreeSet::new(), &BTreeSet::new());
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("no such file exists"));
    }

    #[test]
    fn a_path_reported_written_that_is_not_on_disk_is_an_offence() {
        let all = set(&["src/a.sol"]);
        let off = offences(&all, &all, &BTreeSet::new());
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("reported writing"));
    }

    #[test]
    fn every_declared_path_written_is_clean() {
        let all = set(&["src/a.sol", "src/b.sol"]);
        assert!(offences(&all, &all, &all).is_empty());
    }

    #[test]
    fn a_written_but_undeclared_path_is_not_an_offence() {
        let owns = set(&["src/a.sol"]);
        let wrote = set(&["src/a.sol", "soldeer.lock"]);
        assert!(offences(&owns, &wrote, &wrote).is_empty());
    }

    #[test]
    fn declaring_nothing_is_not_an_offence() {
        let wrote = set(&["src/a.sol"]);
        assert!(offences(&BTreeSet::new(), &wrote, &wrote).is_empty());
    }
}
