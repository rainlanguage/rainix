//! `codegen-fixed-point` — run a repo's regeneration pipeline until the working
//! tree stops changing, under a bound.
//!
//! Generated Solidity feeds back into its own inputs: a pointer table is
//! imported by the contract whose codehash that same table records, so one pass
//! of the pipeline is not a fixed point — it is one application of a function
//! whose output is part of its next input. A pipeline run once and then diffed
//! reports "stale" for three different states, and a developer told to
//! regenerate can only distinguish them by hand: a tree that is one pass behind,
//! a tree that is several passes behind, and a pipeline that will never settle.
//!
//! Iterating here collapses the first two into a pass and separates the third
//! out with its own error. The bound is what makes non-convergence reportable at
//! all — an unbounded loop on an oscillating pipeline is a hung job, which is
//! the same non-diagnosis as the single pass, paid for in runner minutes.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Why the loop stopped.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The pipeline ran `passes` times and the last one changed nothing. The
    /// working tree holds the fixed point. Whether it MATCHES what is committed
    /// is a separate question, answered by the caller's currency check: a repo
    /// already at its fixed point converges in 1, a repo whose committed
    /// artifacts are one or more passes behind converges in more than 1 and
    /// leaves a tree that differs from `HEAD`.
    Converged { passes: u32 },
    /// The tree was still moving when the bound was spent, so no fixed point was
    /// observed and none can be reported to exist.
    NotConverged { passes: u32 },
}

/// Run `command` (via `bash -c`, in `root`) until two consecutive observations
/// of the working tree agree, or until `max_passes` is spent.
///
/// The first comparison is against the tree as it was BEFORE any pass, so an
/// already-current repo costs exactly one pass — the same pipeline cost it paid
/// when the pipeline ran once with no loop around it.
///
/// Pollution already present in the checkout is in that first observation, so it
/// is not mistaken for something a pass emitted.
pub(crate) fn run(root: &Path, max_passes: u32, command: &str) -> Result<Outcome, String> {
    if max_passes == 0 {
        return Err("--max-passes must be at least 1".to_string());
    }
    let index = index_path(root)?;
    let result = iterate(root, max_passes, command, &index);
    let _ = std::fs::remove_file(&index);
    result
}

fn iterate(root: &Path, max_passes: u32, command: &str, index: &Path) -> Result<Outcome, String> {
    let mut previous = snapshot(root, index)?;
    for pass in 1..=max_passes {
        run_pipeline(root, command, pass, max_passes)?;
        let current = snapshot(root, index)?;
        if current == previous {
            return Ok(Outcome::Converged { passes: pass });
        }
        previous = current;
    }
    Ok(Outcome::NotConverged { passes: max_passes })
}

/// One pass of the consumer's pipeline. A pass that fails is not a
/// non-convergence: the pipeline itself is broken and its own error is the one
/// worth reporting, so it stops the loop rather than burning the bound.
fn run_pipeline(root: &Path, command: &str, pass: u32, max_passes: u32) -> Result<(), String> {
    println!("codegen-fixed-point: pass {pass} of at most {max_passes}");
    let status = Command::new("bash")
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .status()
        .map_err(|e| format!("failed to run the regeneration command: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "the regeneration command failed on pass {pass} ({status})"
        ))
    }
}

/// Content hash of the whole working tree, as git's own tree object id.
///
/// Built in a scratch index so the repo's real index is never written: the
/// currency check that runs after this one stages the tree itself, and a loop
/// that had already staged everything would leave it nothing to find.
///
/// Reading through git rather than walking the filesystem is what makes
/// `.gitignore` apply for free, so `out/`, `cache/` and `dependencies/` — which
/// every pass rewrites and no repo commits — do not read as a tree that never
/// settles. It also makes the observation content-addressed, so a pass that
/// rewrites a file with the same bytes is correctly seen as no change, and one
/// that oscillates between two contents of the same path is correctly seen as a
/// change.
fn snapshot(root: &Path, index: &Path) -> Result<String, String> {
    // The scratch index carries over between observations, which `git add --all`
    // is defined to reconcile: it updates entries whose content moved and drops
    // entries whose file is gone, so the tree it writes describes the working
    // tree as it is now and not the union of every pass so far.
    git(root, index, &["add", "--all"])?;
    git(root, index, &["write-tree"])
}

fn git(root: &Path, index: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .output()
        .map_err(|e| format!("failed to run git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Absolute path for the scratch index, inside the repo's own git dir so it
/// shares the repo's filesystem and is invisible to every path the checks read.
fn index_path(root: &Path) -> Result<PathBuf, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .map_err(|e| format!("failed to run git rev-parse: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{} is not a git repository: {}",
            root.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(Path::new(String::from_utf8_lossy(&out.stdout).trim())
        .join("rainix-codegen-fixed-point.index"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    /// A consumer checkout: a git repo with one committed generated artifact and
    /// a `.gitignore` covering the directories forge writes to.
    struct Fixture {
        dir: PathBuf,
        repo: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "rainix-static-fixedpoint-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            let repo = dir.join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            for args in [
                vec!["init", "-q", "-b", "main"],
                vec!["config", "user.email", "rainix@example.com"],
                vec!["config", "user.name", "rainix"],
            ] {
                assert!(Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(&args)
                    .status()
                    .unwrap()
                    .success());
            }
            std::fs::write(repo.join(".gitignore"), "out/\ncache/\n").unwrap();
            std::fs::create_dir_all(repo.join("src/generated")).unwrap();
            std::fs::write(repo.join("src/generated/A.sol"), "pass 0\n").unwrap();
            assert!(Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["add", "--all"])
                .status()
                .unwrap()
                .success());
            assert!(Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["commit", "-qm", "committed artifacts"])
                .status()
                .unwrap()
                .success());
            Fixture { dir, repo }
        }

        /// Path OUTSIDE the repo holding the pass counter, so counting does not
        /// itself perturb the tree the loop is observing.
        fn counter(&self) -> PathBuf {
            self.dir.join("passes")
        }

        fn passes_run(&self) -> u32 {
            std::fs::read_to_string(self.counter())
                .map(|s| s.trim().len() as u32)
                .unwrap_or(0)
        }

        /// A pipeline that bumps the out-of-tree counter, then writes whatever
        /// `body` computes into the committed artifact. `$n` is the pass number.
        fn pipeline(&self, body: &str) -> String {
            format!(
                "printf x >> {counter}; n=$(wc -c < {counter} | tr -d ' '); {body}",
                counter = self.counter().display(),
            )
        }

        fn artifact(&self) -> String {
            std::fs::read_to_string(self.repo.join("src/generated/A.sol")).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_repo_already_at_its_fixed_point_costs_one_pass() {
        let f = Fixture::new();
        let cmd = f.pipeline("printf 'pass 0\\n' > src/generated/A.sol");

        assert_eq!(run(&f.repo, 5, &cmd), Ok(Outcome::Converged { passes: 1 }));
        assert_eq!(
            f.passes_run(),
            1,
            "an unchanged repo must not pay a second pass"
        );
    }

    #[test]
    fn a_stale_repo_converges_and_leaves_the_regenerated_tree() {
        let f = Fixture::new();
        let cmd = f.pipeline("printf 'regenerated\\n' > src/generated/A.sol");

        assert_eq!(run(&f.repo, 5, &cmd), Ok(Outcome::Converged { passes: 2 }));
        assert_eq!(f.artifact(), "regenerated\n");
    }

    #[test]
    fn generation_that_settles_only_after_several_passes_still_converges() {
        let f = Fixture::new();
        // Each pass copies the previous pass's number, so the artifact chases
        // the counter and settles once the counter stops being read fresh —
        // here, a value that stops moving at pass 3.
        let cmd = f.pipeline(
            "if [ \"$n\" -lt 3 ]; then printf 'pass %s\\n' \"$n\" > src/generated/A.sol; fi",
        );

        assert_eq!(run(&f.repo, 5, &cmd), Ok(Outcome::Converged { passes: 3 }));
    }

    #[test]
    fn oscillating_generation_is_reported_as_not_converged() {
        let f = Fixture::new();
        let cmd = f.pipeline("printf 'pass %s\\n' \"$((n % 2))\" > src/generated/A.sol");

        assert_eq!(
            run(&f.repo, 5, &cmd),
            Ok(Outcome::NotConverged { passes: 5 })
        );
        assert_eq!(
            f.passes_run(),
            5,
            "the bound is what stops it, so it is spent in full"
        );
    }

    #[test]
    fn the_bound_is_the_bound() {
        let f = Fixture::new();
        let cmd = f.pipeline(
            "if [ \"$n\" -lt 3 ]; then printf 'pass %s\\n' \"$n\" > src/generated/A.sol; fi",
        );

        // The same pipeline that converges in 3 is not converged in 2.
        assert_eq!(
            run(&f.repo, 2, &cmd),
            Ok(Outcome::NotConverged { passes: 2 })
        );
    }

    #[test]
    fn a_file_no_pass_has_committed_yet_counts_as_a_change() {
        let f = Fixture::new();
        // Nothing tracked ever changes, so `git diff` sees nothing at all here.
        let cmd = f.pipeline("printf 'renamed\\n' > src/generated/B.sol");

        assert_eq!(run(&f.repo, 5, &cmd), Ok(Outcome::Converged { passes: 2 }));
    }

    #[test]
    fn gitignored_build_output_does_not_look_like_a_moving_tree() {
        let f = Fixture::new();
        // Rewriting out/ every pass is what forge does; it must not read as a
        // pipeline that never settles.
        let cmd = f.pipeline(
            "mkdir -p out cache; printf '%s' \"$n\" > out/A.json; printf '%s' \"$n\" > cache/x",
        );

        assert_eq!(run(&f.repo, 5, &cmd), Ok(Outcome::Converged { passes: 1 }));
    }

    #[test]
    fn the_repos_own_index_is_left_for_the_currency_check_to_stage() {
        let f = Fixture::new();
        let cmd = f.pipeline("printf 'regenerated\\n' > src/generated/A.sol");

        assert_eq!(run(&f.repo, 5, &cmd), Ok(Outcome::Converged { passes: 2 }));

        let status = Command::new("git")
            .arg("-C")
            .arg(&f.repo)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        // Trimmed, an unstaged modification's " M path" is "M path"; a staged
        // one would read "M  path" instead.
        assert_eq!(
            String::from_utf8_lossy(&status.stdout).trim(),
            "M src/generated/A.sol",
            "the regenerated file must still be unstaged"
        );
    }

    #[test]
    fn a_scratch_index_is_not_left_behind() {
        let f = Fixture::new();
        let cmd = f.pipeline("printf 'regenerated\\n' > src/generated/A.sol");

        run(&f.repo, 5, &cmd).unwrap();

        assert!(!f
            .repo
            .join(".git/rainix-codegen-fixed-point.index")
            .exists());
    }

    #[test]
    fn a_failing_pipeline_stops_the_loop_and_reports_itself() {
        let f = Fixture::new();
        let cmd = f.pipeline("printf 'regenerated\\n' > src/generated/A.sol; exit 3");

        let err = run(&f.repo, 5, &cmd).unwrap_err();

        assert!(err.contains("regeneration command failed"), "{err}");
        assert!(err.contains("pass 1"), "{err}");
        assert_eq!(f.passes_run(), 1, "a broken pipeline must not be retried");
    }

    #[test]
    fn a_bound_of_zero_is_rejected_rather_than_passing_without_running() {
        let f = Fixture::new();
        let cmd = f.pipeline("printf 'regenerated\\n' > src/generated/A.sol");

        let err = run(&f.repo, 0, &cmd).unwrap_err();

        assert!(err.contains("at least 1"), "{err}");
        assert_eq!(f.passes_run(), 0);
    }

    #[test]
    fn a_directory_that_is_not_a_repo_is_an_error_not_a_pass() {
        let dir = std::env::temp_dir().join(format!(
            "rainix-static-fixedpoint-norepo-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let err = run(&dir, 5, "true").unwrap_err();

        assert!(err.contains("not a git repository"), "{err}");
        // git's own message names no path, so an operator who pointed the loop
        // at the wrong directory learns which one only if this says so.
        assert!(err.contains(&dir.display().to_string()), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
