//! `ci-gate` — publish gate on the gated commit's own CI.
//!
//! rainix-autopublish runs concurrently with the caller repo's test workflows
//! (both trigger on the same push), so without a gate a red commit publishes an
//! immutable registry revision while — or before — its own CI reports. This
//! subcommand polls the repository's workflow runs for `GITHUB_SHA`, excluding
//! every run of the release workflow itself (resolved from `GITHUB_RUN_ID`, so
//! the gate never waits on itself, its re-run attempts, or a concurrent
//! dispatch of the same release workflow), and exits 0 only when every other
//! run on the commit has completed green. A failed, cancelled or timed-out run
//! is a loud immediate error naming it; a commit with NO other workflow runs
//! after a grace period is a loud error too (fail-closed: every rainix
//! consumer runs push-triggered CI, so "nothing else ran" means nothing tested
//! the commit, not that there was nothing to wait for). Transient API failures
//! (5xx, rate limits, transport) retry until the deadline; a token that cannot
//! read Actions runs is a fatal error naming the `actions: read` grant the
//! caller must carry.
//!
//! Auth comes from `GITHUB_TOKEN` and reaches curl via a config file on stdin,
//! never argv (argv is world-readable in /proc while curl runs).

use crate::fail;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// One workflow run on the gated commit, as the Actions API reports it.
#[derive(Debug, Clone, PartialEq)]
struct Run {
    workflow_id: u64,
    name: String,
    path: String,
    status: String,
    conclusion: Option<String>,
    url: String,
}

/// How a single run counts toward the gate.
#[derive(Debug, PartialEq)]
enum RunState {
    Green,
    Red,
    Pending,
}

/// Classify one run. Anything not `completed` is pending. Completed runs:
/// `success` passes; `skipped` passes (the whole workflow was skipped by
/// job-level `if`s — observed conclusion for such runs — so it deliberately
/// did not apply to this commit); `neutral` passes (GitHub's own required-check
/// logic treats it as passing). `failure`, `cancelled`, `timed_out`,
/// `action_required`, `stale` and `startup_failure` fail. A conclusion this
/// gate does not recognize is a loud error, never a silent pass — fail-closed
/// against GitHub growing new conclusion values.
fn classify(status: &str, conclusion: Option<&str>) -> Result<RunState, String> {
    if status != "completed" {
        return Ok(RunState::Pending);
    }
    match conclusion {
        Some("success") | Some("skipped") | Some("neutral") => Ok(RunState::Green),
        Some("failure")
        | Some("cancelled")
        | Some("timed_out")
        | Some("action_required")
        | Some("stale")
        | Some("startup_failure") => Ok(RunState::Red),
        other => Err(format!(
            "workflow run completed with unrecognized conclusion {other:?}; \
             refusing to treat it as passing"
        )),
    }
}

/// The gate's decision over one snapshot of the commit's runs.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// Every other-workflow run on the commit completed green.
    Pass { green: usize },
    /// At least one completed red — the strings name them. Red wins over
    /// pending: one failed run already forbids the publish, so the gate does
    /// not wait for the rest.
    Red(Vec<String>),
    /// Still waiting on these runs.
    Wait(Vec<String>),
    /// No runs besides the release workflow's own exist (yet).
    NoOtherCi,
}

/// Decide the gate verdict from a snapshot of the commit's runs. Runs of the
/// release workflow itself (`own_workflow_id`) are invisible to the gate.
fn verdict(runs: &[Run], own_workflow_id: u64) -> Result<Verdict, String> {
    let mut green = 0usize;
    let mut red = Vec::new();
    let mut pending = Vec::new();
    for r in runs.iter().filter(|r| r.workflow_id != own_workflow_id) {
        match classify(&r.status, r.conclusion.as_deref())? {
            RunState::Green => green += 1,
            RunState::Red => red.push(format!(
                "{} ({}) concluded {}: {}",
                r.name,
                r.path,
                r.conclusion.as_deref().unwrap_or("<none>"),
                r.url
            )),
            RunState::Pending => pending.push(format!("{} ({}) is {}", r.name, r.path, r.status)),
        }
    }
    Ok(if !red.is_empty() {
        Verdict::Red(red)
    } else if !pending.is_empty() {
        Verdict::Wait(pending)
    } else if green == 0 {
        Verdict::NoOtherCi
    } else {
        Verdict::Pass { green }
    })
}

/// Parse the workflow-runs list response into runs + the API's total count
/// (the pagination loop's termination signal). A missing or malformed field is
/// an error, never a skipped run — a run the gate cannot read must not become
/// a run the gate does not wait for.
fn parse_runs(body: &str) -> Result<(Vec<Run>, u64), String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("workflow-runs response is not JSON ({e}): {body}"))?;
    let total = v
        .get("total_count")
        .and_then(|t| t.as_u64())
        .ok_or_else(|| format!("workflow-runs response has no numeric total_count: {body}"))?;
    let arr = v
        .get("workflow_runs")
        .and_then(|w| w.as_array())
        .ok_or_else(|| format!("workflow-runs response has no workflow_runs array: {body}"))?;
    let mut runs = Vec::new();
    for r in arr {
        let u64_field = |k: &str| {
            r.get(k)
                .and_then(|x| x.as_u64())
                .ok_or_else(|| format!("workflow run has no numeric {k}: {r}"))
        };
        let str_field = |k: &str| {
            r.get(k)
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("workflow run has no {k}: {r}"))
        };
        let conclusion = match r.get("conclusion") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            Some(other) => return Err(format!("workflow run conclusion is not a string: {other}")),
        };
        runs.push(Run {
            workflow_id: u64_field("workflow_id")?,
            name: str_field("name")?,
            path: str_field("path")?,
            status: str_field("status")?,
            conclusion,
            url: str_field("html_url")?,
        });
    }
    Ok((runs, total))
}

/// Parse the single-run lookup response into its workflow_id — which workflow
/// file the release run belongs to.
fn parse_workflow_id(body: &str) -> Result<u64, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("run-lookup response is not JSON ({e}): {body}"))?;
    v.get("workflow_id")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| format!("run-lookup response has no numeric workflow_id: {body}"))
}

/// An API failure, split by what the poll loop should do with it.
#[derive(Debug, PartialEq)]
enum ApiFailure {
    /// Retry on the next poll tick until the deadline: outages, 5xx, rate
    /// limits. A gate that can wait hours for CI must not die to one blip.
    Transient(String),
    /// Stop now: bad token, missing permission, malformed response. Waiting
    /// cannot fix these.
    Fatal(String),
}

/// Map an HTTP status to Ok (caller parses the body) or a failure. 403 is
/// BOTH GitHub's permission refusal and its rate-limit status; the rate-limit
/// bodies say so, so that text routes to Transient and every other 403 (and
/// 404, which is how the API hides resources the token cannot see) is the
/// caller-permissions error, with the fix in the message.
fn api_status(status: u16, body: &str, what: &str) -> Result<(), ApiFailure> {
    match status {
        200 => Ok(()),
        401 => Err(ApiFailure::Fatal(format!(
            "{what}: GitHub API returned 401 — GITHUB_TOKEN is missing or invalid: {body}"
        ))),
        429 => Err(ApiFailure::Transient(format!(
            "{what}: GitHub API rate-limited (HTTP 429): {body}"
        ))),
        403 if body.contains("rate limit") => Err(ApiFailure::Transient(format!(
            "{what}: GitHub API rate-limited (HTTP 403): {body}"
        ))),
        403 | 404 => Err(ApiFailure::Fatal(format!(
            "{what}: GitHub API returned HTTP {status} — the workflow token cannot read \
             Actions runs. The rainix-autopublish job requests `actions: read`; a caller \
             job that sets an explicit `permissions:` block on the job that `uses:` \
             rainix-autopublish must include `actions: read` in that block (a called \
             workflow can only narrow the caller's grant, never widen it): {body}"
        ))),
        500..=599 => Err(ApiFailure::Transient(format!(
            "{what}: GitHub API returned HTTP {status}: {body}"
        ))),
        other => Err(ApiFailure::Fatal(format!(
            "{what}: GitHub API returned unexpected HTTP {other}: {body}"
        ))),
    }
}

/// curl config lines carrying the auth + protocol headers. The token travels
/// on curl's stdin via this config, never argv. A token that cannot be quoted
/// into the config safely (curl's double-quoted values take backslash
/// escapes) is refused rather than escaped — real GITHUB_TOKENs are plain
/// ASCII, so anything else is not a token.
fn curl_config(token: &str) -> Result<String, String> {
    if token.is_empty() {
        return Err("GITHUB_TOKEN is empty".to_string());
    }
    if !token
        .bytes()
        .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
    {
        return Err(
            "GITHUB_TOKEN contains whitespace, quotes, or non-ASCII bytes; refusing to \
             pass it to curl"
                .to_string(),
        );
    }
    Ok(format!(
        "header = \"Authorization: Bearer {token}\"\n\
         header = \"Accept: application/vnd.github+json\"\n\
         header = \"X-GitHub-Api-Version: 2022-11-28\"\n\
         header = \"User-Agent: rainix-autopublish (+https://github.com/rainlanguage/rainix)\"\n"
    ))
}

/// `owner/repo`, both segments limited to GitHub's name alphabet — anything
/// else could smuggle URL structure into the API path.
fn validate_repo(repo: &str) -> Result<(), String> {
    let ok_seg = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    };
    match repo.split_once('/') {
        Some((owner, name)) if ok_seg(owner) && ok_seg(name) => Ok(()),
        _ => Err(format!(
            "GITHUB_REPOSITORY ({repo:?}) is not an owner/repo name"
        )),
    }
}

/// A full 40-hex commit sha, as GITHUB_SHA always is.
fn validate_sha(sha: &str) -> Result<(), String> {
    if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("GITHUB_SHA ({sha:?}) is not a 40-hex commit sha"))
    }
}

/// A numeric run id, as GITHUB_RUN_ID always is.
fn validate_run_id(id: &str) -> Result<(), String> {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(format!("GITHUB_RUN_ID ({id:?}) is not a run id"))
    }
}

/// GET an API URL with the token, returning (status, body). Transport
/// failures (spawn, DNS, connect, TLS) are strings for the caller to treat as
/// transient.
fn curl_api(url: &str, token: &str) -> Result<(u16, String), String> {
    let cfg = curl_config(token)?;
    let mut child = Command::new("curl")
        .args(["-sS", "--config", "-", "-w", "\n%{http_code}", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl {url}: failed to spawn: {e}"))?;
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(cfg.as_bytes())
        .map_err(|e| format!("curl {url}: failed to write config: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("curl {url}: {e}"))?;
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

/// The workflow_id of the run the gate is running inside.
fn fetch_own_workflow_id(
    api: &str,
    repo: &str,
    run_id: &str,
    token: &str,
) -> Result<u64, ApiFailure> {
    let url = format!("{api}/repos/{repo}/actions/runs/{run_id}");
    let (status, body) = curl_api(&url, token).map_err(ApiFailure::Transient)?;
    api_status(status, &body, "look up own workflow run")?;
    parse_workflow_id(&body).map_err(ApiFailure::Fatal)
}

/// All workflow runs for the commit, across every trigger event, paged until
/// the API's own total_count is reached.
fn list_runs(api: &str, repo: &str, sha: &str, token: &str) -> Result<Vec<Run>, ApiFailure> {
    let mut all: Vec<Run> = Vec::new();
    let mut page = 1u32;
    loop {
        let url =
            format!("{api}/repos/{repo}/actions/runs?head_sha={sha}&per_page=100&page={page}");
        let (status, body) = curl_api(&url, token).map_err(ApiFailure::Transient)?;
        api_status(status, &body, "list workflow runs")?;
        let (runs, total) = parse_runs(&body).map_err(ApiFailure::Fatal)?;
        let got = runs.len();
        all.extend(runs);
        if all.len() as u64 >= total || got == 0 {
            return Ok(all);
        }
        page += 1;
        if page > 20 {
            return Err(ApiFailure::Fatal(format!(
                "more than 2000 workflow runs reported for {sha}; refusing to page further"
            )));
        }
    }
}

/// Run the gate: poll until every other run on GITHUB_SHA is green (exit 0),
/// any is red (loud failure), no other CI exists past the grace period (loud,
/// fail-closed), or the deadline passes (loud, naming what was still pending).
pub(crate) fn run(timeout_secs: u64, poll_secs: u64, grace_secs: u64) {
    let env = |k: &str| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| fail(&format!("ci-gate: {k} is not set")))
    };
    let repo = env("GITHUB_REPOSITORY");
    let sha = env("GITHUB_SHA");
    let run_id = env("GITHUB_RUN_ID");
    let token = env("GITHUB_TOKEN");
    let api = std::env::var("GITHUB_API_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://api.github.com".to_string());
    validate_repo(&repo).unwrap_or_else(|e| fail(&format!("ci-gate: {e}")));
    validate_sha(&sha).unwrap_or_else(|e| fail(&format!("ci-gate: {e}")));
    validate_run_id(&run_id).unwrap_or_else(|e| fail(&format!("ci-gate: {e}")));

    let start = Instant::now();
    let deadline = Duration::from_secs(timeout_secs);
    let grace = Duration::from_secs(grace_secs);
    let poll = Duration::from_secs(poll_secs);

    let own_workflow_id = loop {
        match fetch_own_workflow_id(&api, &repo, &run_id, &token) {
            Ok(id) => break id,
            Err(ApiFailure::Fatal(m)) => fail(&format!("ci-gate: {m}")),
            Err(ApiFailure::Transient(m)) => {
                eprintln!("ci-gate: transient API failure, will retry: {m}");
                if start.elapsed() >= deadline {
                    fail(&format!(
                        "ci-gate: timed out after {timeout_secs}s without resolving own \
                         workflow run; last failure: {m}"
                    ));
                }
                std::thread::sleep(poll);
            }
        }
    };

    let mut last_wait: Vec<String> = Vec::new();
    let mut last_transient: Option<String> = None;
    loop {
        match list_runs(&api, &repo, &sha, &token) {
            Err(ApiFailure::Fatal(m)) => fail(&format!("ci-gate: {m}")),
            Err(ApiFailure::Transient(m)) => {
                eprintln!("ci-gate: transient API failure, will retry: {m}");
                last_transient = Some(m);
            }
            Ok(runs) => match verdict(&runs, own_workflow_id) {
                Err(m) => fail(&format!("ci-gate: {m}")),
                Ok(Verdict::Pass { green }) => {
                    println!("ci-gate: all {green} other workflow run(s) on {sha} completed green");
                    return;
                }
                Ok(Verdict::Red(msgs)) => fail(&format!(
                    "ci-gate: refusing to publish {sha} — {} workflow run(s) on this \
                     commit failed: {}",
                    msgs.len(),
                    msgs.join("; ")
                )),
                Ok(Verdict::Wait(pending)) => {
                    eprintln!(
                        "ci-gate: waiting on {} run(s): {}",
                        pending.len(),
                        pending.join("; ")
                    );
                    last_wait = pending;
                }
                Ok(Verdict::NoOtherCi) => {
                    if start.elapsed() >= grace {
                        fail(&format!(
                            "ci-gate: no workflow run besides this release workflow exists \
                             for {sha} after {grace_secs}s — refusing to publish a commit \
                             nothing has tested. Add a workflow that runs the repo's \
                             checks on push (every rainix consumer has one), then re-run \
                             this job."
                        ));
                    }
                    eprintln!(
                        "ci-gate: no other workflow runs for {sha} yet; \
                         within the {grace_secs}s grace period for them to appear"
                    );
                }
            },
        }
        if start.elapsed() >= deadline {
            let detail = if !last_wait.is_empty() {
                format!("still pending: {}", last_wait.join("; "))
            } else if let Some(t) = last_transient {
                format!("last API failure: {t}")
            } else {
                "no other workflow runs were observed".to_string()
            };
            fail(&format!(
                "ci-gate: timed out after {timeout_secs}s waiting for CI on {sha}; {detail}"
            ));
        }
        std::thread::sleep(poll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(workflow_id: u64, status: &str, conclusion: Option<&str>) -> Run {
        Run {
            workflow_id,
            name: format!("wf-{workflow_id}"),
            path: format!(".github/workflows/wf-{workflow_id}.yaml"),
            status: status.to_string(),
            conclusion: conclusion.map(str::to_string),
            url: format!("https://github.com/o/r/actions/runs/{workflow_id}"),
        }
    }

    #[test]
    fn classify_incomplete_is_pending_regardless_of_conclusion() {
        for status in ["queued", "in_progress", "waiting", "requested", "pending"] {
            assert_eq!(classify(status, None).unwrap(), RunState::Pending);
            // Even a conclusion-carrying non-completed run is pending: only
            // `completed` has a final verdict.
            assert_eq!(
                classify(status, Some("success")).unwrap(),
                RunState::Pending
            );
        }
        // A status this gate has never seen can only delay, never pass or fail.
        assert_eq!(classify("hologram", None).unwrap(), RunState::Pending);
    }

    #[test]
    fn classify_green_conclusions() {
        for c in ["success", "skipped", "neutral"] {
            assert_eq!(classify("completed", Some(c)).unwrap(), RunState::Green);
        }
    }

    #[test]
    fn classify_red_conclusions() {
        for c in [
            "failure",
            "cancelled",
            "timed_out",
            "action_required",
            "stale",
            "startup_failure",
        ] {
            assert_eq!(classify("completed", Some(c)).unwrap(), RunState::Red);
        }
    }

    #[test]
    fn classify_unknown_completed_conclusion_is_loud() {
        // Fail-closed: a conclusion value this gate does not know must never
        // be treated as passing (or silently failing).
        let e = classify("completed", Some("great_success")).unwrap_err();
        assert!(e.contains("great_success"), "{e}");
        assert!(classify("completed", None).is_err());
    }

    #[test]
    fn verdict_all_green_passes_and_counts() {
        let runs = vec![
            run_with(1, "completed", Some("success")),
            run_with(2, "completed", Some("skipped")),
            run_with(9, "in_progress", None), // own workflow: invisible
        ];
        assert_eq!(verdict(&runs, 9).unwrap(), Verdict::Pass { green: 2 });
    }

    #[test]
    fn verdict_excludes_every_run_of_own_workflow() {
        // Two runs of the release workflow itself (e.g. a push run and a
        // dispatch re-run) must both be invisible, or the gate deadlocks on
        // itself.
        let runs = vec![
            run_with(9, "in_progress", None),
            run_with(9, "queued", None),
            run_with(1, "completed", Some("success")),
        ];
        assert_eq!(verdict(&runs, 9).unwrap(), Verdict::Pass { green: 1 });
    }

    #[test]
    fn verdict_red_names_the_failed_run() {
        let runs = vec![
            run_with(1, "completed", Some("failure")),
            run_with(2, "completed", Some("success")),
        ];
        match verdict(&runs, 9).unwrap() {
            Verdict::Red(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert!(msgs[0].contains("wf-1"), "{}", msgs[0]);
                assert!(msgs[0].contains("failure"), "{}", msgs[0]);
                assert!(msgs[0].contains("actions/runs"), "{}", msgs[0]);
            }
            v => panic!("expected Red, got {v:?}"),
        }
    }

    #[test]
    fn verdict_red_wins_over_pending() {
        // One red already forbids the publish; the gate must not keep waiting
        // on the rest first.
        let runs = vec![
            run_with(1, "completed", Some("cancelled")),
            run_with(2, "in_progress", None),
        ];
        assert!(matches!(verdict(&runs, 9).unwrap(), Verdict::Red(_)));
    }

    #[test]
    fn verdict_pending_waits() {
        let runs = vec![
            run_with(1, "completed", Some("success")),
            run_with(2, "queued", None),
        ];
        match verdict(&runs, 9).unwrap() {
            Verdict::Wait(p) => {
                assert_eq!(p.len(), 1);
                assert!(p[0].contains("wf-2"), "{}", p[0]);
                assert!(p[0].contains("queued"), "{}", p[0]);
            }
            v => panic!("expected Wait, got {v:?}"),
        }
    }

    #[test]
    fn verdict_no_other_ci_when_only_own_runs_exist() {
        assert_eq!(verdict(&[], 9).unwrap(), Verdict::NoOtherCi);
        let only_own = vec![run_with(9, "in_progress", None)];
        assert_eq!(verdict(&only_own, 9).unwrap(), Verdict::NoOtherCi);
    }

    #[test]
    fn verdict_unknown_conclusion_is_loud_even_beside_green() {
        let runs = vec![
            run_with(1, "completed", Some("success")),
            run_with(2, "completed", Some("mystery")),
        ];
        assert!(verdict(&runs, 9).is_err());
    }

    /// Live API shape, captured from GET
    /// /repos/rainlanguage/rain.string/actions/runs?head_sha=256c624… — the
    /// release run's status/conclusion as observed while it was still queued
    /// (`"pending"` / null), everything else verbatim from the response.
    /// Extra fields are present and ignored.
    #[test]
    fn parse_runs_live_shape() {
        let body = r#"{
            "total_count": 2,
            "workflow_runs": [
                {"id": 32835741741, "name": "Package Release",
                 "path": ".github/workflows/package-release.yaml",
                 "head_sha": "256c62449bcf4678638c6a21695f70269d2b2bef",
                 "event": "push", "status": "pending", "conclusion": null,
                 "workflow_id": 289149655, "run_attempt": 1,
                 "html_url": "https://github.com/rainlanguage/rain.string/actions/runs/32835741741"},
                {"id": 32835741982, "name": "rainix",
                 "path": ".github/workflows/rainix.yaml",
                 "head_sha": "256c62449bcf4678638c6a21695f70269d2b2bef",
                 "event": "push", "status": "completed", "conclusion": "success",
                 "workflow_id": 125389650, "run_attempt": 1,
                 "html_url": "https://github.com/rainlanguage/rain.string/actions/runs/32835741982"}
            ]
        }"#;
        let (runs, total) = parse_runs(body).unwrap();
        assert_eq!(total, 2);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].workflow_id, 289149655);
        assert_eq!(runs[0].status, "pending");
        assert_eq!(runs[0].conclusion, None);
        assert_eq!(runs[1].conclusion.as_deref(), Some("success"));
        // The two workflows are distinguishable for self-exclusion.
        assert_ne!(runs[0].workflow_id, runs[1].workflow_id);
    }

    #[test]
    fn parse_runs_malformed_is_an_error_not_a_skipped_run() {
        assert!(parse_runs("not json").is_err());
        assert!(parse_runs("{}").is_err());
        // Missing total_count alone (array fine) is an error, never zero:
        // total_count is the pagination loop's termination signal.
        assert!(parse_runs(r#"{"workflow_runs":[]}"#).is_err());
        assert!(parse_runs(r#"{"total_count": 1}"#).is_err()); // no array
                                                               // A run entry missing its workflow_id must not silently drop out of
                                                               // the wait set.
        assert!(parse_runs(
            r#"{"total_count":1,"workflow_runs":[
                {"name":"x","path":"p","status":"queued","conclusion":null,"html_url":"u"}]}"#
        )
        .is_err());
        // conclusion of a non-string, non-null type is malformed.
        assert!(parse_runs(
            r#"{"total_count":1,"workflow_runs":[
                {"workflow_id":1,"name":"x","path":"p","status":"queued","conclusion":7,"html_url":"u"}]}"#
        )
        .is_err());
    }

    #[test]
    fn parse_workflow_id_live_shape() {
        // Captured from GET /repos/rainlanguage/rain.string/actions/runs/32835741741.
        let body = r#"{"id": 32835741741, "workflow_id": 289149655,
                       "path": ".github/workflows/package-release.yaml",
                       "status": "completed", "conclusion": "success"}"#;
        assert_eq!(parse_workflow_id(body).unwrap(), 289149655);
        assert!(parse_workflow_id("{}").is_err());
        assert!(parse_workflow_id("not json").is_err());
        assert!(parse_workflow_id(r#"{"workflow_id":"str"}"#).is_err());
    }

    #[test]
    fn api_status_ok_and_auth_failures() {
        assert!(api_status(200, "{}", "x").is_ok());
        match api_status(401, "bad credentials", "x") {
            Err(ApiFailure::Fatal(m)) => assert!(m.contains("GITHUB_TOKEN"), "{m}"),
            other => panic!("expected Fatal, got {other:?}"),
        }
    }

    #[test]
    fn api_status_permission_refusal_names_the_grant() {
        // 403 (resource_not_accessible) and 404 (how the API hides what the
        // token cannot see) both carry the actionable fix.
        for status in [403u16, 404] {
            match api_status(status, r#"{"message":"Resource not accessible"}"#, "x") {
                Err(ApiFailure::Fatal(m)) => {
                    assert!(m.contains("actions: read"), "{m}");
                    assert!(m.contains("permissions"), "{m}");
                }
                other => panic!("expected Fatal for {status}, got {other:?}"),
            }
        }
    }

    #[test]
    fn api_status_rate_limits_and_5xx_are_transient() {
        for (status, body) in [
            (429u16, "slow down"),
            (
                403,
                r#"{"message":"API rate limit exceeded for installation"}"#,
            ),
            (
                403,
                r#"{"message":"You have exceeded a secondary rate limit"}"#,
            ),
            (500, "boom"),
            (502, "bad gateway"),
            (503, ""),
        ] {
            assert!(
                matches!(api_status(status, body, "x"), Err(ApiFailure::Transient(_))),
                "{status} {body}"
            );
        }
    }

    #[test]
    fn api_status_unexpected_is_fatal() {
        assert!(matches!(
            api_status(302, "", "x"),
            Err(ApiFailure::Fatal(_))
        ));
        assert!(matches!(
            api_status(418, "teapot", "x"),
            Err(ApiFailure::Fatal(_))
        ));
    }

    #[test]
    fn curl_config_carries_token_and_protocol_headers() {
        let cfg = curl_config("ghs_abc123").unwrap();
        assert!(cfg.contains("Authorization: Bearer ghs_abc123"), "{cfg}");
        assert!(cfg.contains("Accept: application/vnd.github+json"), "{cfg}");
        assert!(cfg.contains("X-GitHub-Api-Version"), "{cfg}");
        assert!(cfg.contains("User-Agent"), "{cfg}");
    }

    #[test]
    fn curl_config_refuses_unquotable_tokens() {
        assert!(curl_config("").is_err());
        assert!(curl_config("has space").is_err());
        assert!(curl_config("has\"quote").is_err());
        assert!(curl_config("has\\slash").is_err());
        assert!(curl_config("has\nnewline").is_err());
        assert!(curl_config("hàs-utf8").is_err());
    }

    #[test]
    fn env_inputs_are_validated() {
        assert!(validate_repo("rainlanguage/rain.string").is_ok());
        assert!(validate_repo("no-slash").is_err());
        assert!(validate_repo("a/b/c").is_err());
        assert!(validate_repo("a/b?x=1").is_err());
        assert!(validate_repo("/name").is_err());
        assert!(validate_repo("owner/").is_err());

        assert!(validate_sha("256c62449bcf4678638c6a21695f70269d2b2bef").is_ok());
        assert!(validate_sha("main").is_err());
        assert!(validate_sha("256c624").is_err());
        assert!(validate_sha("z56c62449bcf4678638c6a21695f70269d2b2bef").is_err());

        assert!(validate_run_id("32835741741").is_ok());
        assert!(validate_run_id("").is_err());
        assert!(validate_run_id("12x").is_err());
    }

    #[test]
    fn curl_output_splits_into_status_and_body() {
        assert_eq!(
            split_status_body("body\n200").unwrap(),
            (200, "body".to_string())
        );
        assert_eq!(
            split_status_body("{\"a\":1}\nmore\n404").unwrap(),
            (404, "{\"a\":1}\nmore".to_string())
        );
        assert_eq!(split_status_body("\n404").unwrap(), (404, String::new()));
        assert!(split_status_body("no-newline").is_err());
        assert!(split_status_body("body\nnot-a-number").is_err());
    }
}
