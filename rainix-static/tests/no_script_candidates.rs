// Fixture build-info: in tests/fixtures/no-script-candidates, per case
// `forge build script/<Case>.s.sol --force --build-info --build-info-path build-info/<case>`.

use std::path::{Path, PathBuf};
use std::process::Output;

fn fixture(case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/no-script-candidates/build-info")
        .join(case)
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("no-script-candidates")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_rainix-static"))
        .arg("no-script-candidates")
        .arg(dir)
        .output()
        .unwrap()
}

fn assert_offenders(dir: &Path, expected: &[&str]) {
    let out = run(dir);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(!lines[0].starts_with("  "), "{stdout}");
    let mut actual: Vec<&str> = lines[1..]
        .iter()
        .map(|l| l.strip_prefix("  ").unwrap())
        .collect();
    actual.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{stdout}");
}

fn assert_error(dir: &Path, message: &str) {
    let out = run(dir);
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(out.stdout.is_empty());
    assert!(
        stderr.contains(&format!(
            "no-script-candidates: {message} {}",
            dir.display()
        )),
        "{stderr}"
    );
}

#[test]
fn direct_candidate_use_is_reported_at_its_byte_offset_line() {
    assert_offenders(
        &fixture("direct"),
        &["script/Direct.s.sol:16: TIMELOCK_CANDIDATE"],
    );
}

#[test]
fn import_aliases_resolve_to_the_candidate() {
    assert_offenders(
        &fixture("alias"),
        &[
            "script/Alias.s.sol:6: GOVERNANCE_CANDIDATE",
            "script/Alias.s.sol:10: TIMELOCK_CANDIDATE",
            "script/Alias.s.sol:14: GOVERNANCE_CANDIDATE",
        ],
    );
}

#[test]
fn library_function_reaching_a_candidate_through_calls_is_reported() {
    assert_offenders(&fixture("transitive"), &["script/Transitive.s.sol:9: plan"]);
}

#[test]
fn constant_holding_a_candidate_is_reported() {
    assert_offenders(
        &fixture("constant"),
        &["script/Constant.s.sol:9: FACET_IMPLEMENTATION"],
    );
}

#[test]
fn src_wiring_to_candidates_and_new_or_type_of_it_pass() {
    let dir = fixture("src-only");
    let info = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap();
    let text = std::fs::read_to_string(info.path()).unwrap();
    assert!(text.contains("\"src/Beacon.sol\"") && text.contains("TIMELOCK_CANDIDATE"));
    let out = run(&dir);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(stdout, "no-script-candidates: clean\n");
}

#[test]
fn every_build_info_file_in_the_dir_is_read() {
    let dir = scratch("several");
    for case in ["direct", "constant"] {
        for f in std::fs::read_dir(fixture(case)).unwrap() {
            let f = f.unwrap();
            std::fs::copy(f.path(), dir.join(f.file_name())).unwrap();
        }
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    assert_offenders(
        &dir,
        &[
            "script/Direct.s.sol:16: TIMELOCK_CANDIDATE",
            "script/Constant.s.sol:9: FACET_IMPLEMENTATION",
        ],
    );
}

#[test]
fn empty_build_info_dir_errors() {
    assert_error(&scratch("empty"), "no build-info in");
}

#[test]
fn missing_build_info_dir_errors() {
    let dir = scratch("missing").join("build-info");
    assert_error(&dir, "cannot read");
}
