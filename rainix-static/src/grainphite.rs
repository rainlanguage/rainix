//! `grainphite-gate` and `grainphite-report`: let a restacked upstack PR skip
//! CI it already passed (RAI-3953).
//!
//! grainphite stacks PRs. A restack of N PRs pushes N branches, and without a
//! gate every one of them runs full CI. The gate asks the grainphite server
//! whether this push needs a full run: the bottom PR, the merge queue, plain
//! pushes and `ci:full` PRs always do; an upstack PR whose own diff is the one
//! that last passed does not. The report tells the server how a full run went,
//! since only a pass earns a later skip.
//!
//! Both authenticate with the job's GitHub OIDC token (audience = the server
//! URL), so no secret is stored anywhere. The token reaches curl through a
//! config file on stdin, never argv.
//!
//! The gate fails open. No URL, no OIDC token (a fork PR), a server that is
//! down or an answer that does not parse all mean `skip=false`, and the gate
//! still exits 0, so a grainphite outage can only cost CI time, never skip it.

use crate::fail;
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

/// What the gate tells the workflow.
#[derive(Debug, PartialEq)]
struct Answer {
    skip: bool,
    /// Concurrency group for the gated jobs; empty means "use your own".
    group: String,
    summary: String,
}

impl Answer {
    fn run_in_full(why: &str) -> Self {
        Answer {
            skip: false,
            group: String::new(),
            summary: format!("running in full: {why}"),
        }
    }

    fn output(&self) -> String {
        format!("skip={}\ngroup={}\n", self.skip, self.group)
    }
}

/// A concurrency group name we are willing to hand to GitHub: short, and no
/// character that could end the output line or the expression.
fn valid_group(g: &str) -> bool {
    !g.is_empty()
        && g.len() <= 200
        && g.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

/// The server's `/ci/decide` answer. Anything off-shape is an error, which
/// the caller turns into a full run.
fn parse_decide(body: &str) -> Result<Answer, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("answer is not JSON: {e}"))?;
    let run = v["run"].as_bool().ok_or("answer has no boolean `run`")?;
    let group = v["group"].as_str().unwrap_or_default();
    if !valid_group(group) {
        return Err(format!("answer has a bad group: {group:?}"));
    }
    let summary = v["summary"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect::<String>();
    Ok(Answer {
        skip: !run,
        group: group.to_string(),
        summary: format!(
            "{}: {summary}",
            if run { "running in full" } else { "skipped" }
        ),
    })
}

/// `success` only when every job the report waits on succeeded. A cancelled
/// or skipped job did not test anything, so it does not count as a pass.
fn result_from_needs(needs: &str) -> Result<&'static str, String> {
    let v: Value = serde_json::from_str(needs).map_err(|e| format!("needs is not JSON: {e}"))?;
    let jobs = v.as_object().ok_or("needs is not an object")?;
    if jobs.is_empty() {
        return Err("needs lists no job".into());
    }
    let all_passed = jobs.values().all(|j| j["result"] == "success");
    Ok(if all_passed { "success" } else { "failure" })
}

/// Percent-encodes a query value.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// The server origin: https, no path, no trailing slash.
fn valid_url(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|host| {
        !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-.:".contains(&b))
    })
}

fn valid_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A bearer token as a curl config line. Tokens are plain ASCII; anything
/// with whitespace, quotes or backslashes is refused, not escaped.
fn auth_config(scheme: &str, token: &str) -> Result<String, String> {
    if token.is_empty()
        || !token
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
    {
        return Err("token is empty or not plain ASCII".into());
    }
    Ok(format!(
        "header = \"Authorization: {scheme} {token}\"\n\
         connect-timeout = 10\n\
         max-time = 30\n"
    ))
}

/// One HTTP call, returning (status, body). `body`, when given, is JSON.
fn curl(url: &str, cfg: &str, body: Option<&str>) -> Result<(u16, String), String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "--config", "-", "-w", "\n%{http_code}"]);
    if let Some(b) = body {
        cmd.args(["-H", "Content-Type: application/json", "--data-binary", b]);
    }
    let mut child = cmd
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(cfg.as_bytes())
        .map_err(|e| format!("curl config: {e}"))?;
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (body, code) = stdout.rsplit_once('\n').ok_or("curl: no status line")?;
    let status = code.trim().parse().map_err(|_| "curl: bad status line")?;
    Ok((status, body.to_string()))
}

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_default()
}

/// The job's OIDC token, minted for `audience`. Needs `id-token: write`.
fn oidc_token(audience: &str) -> Result<String, String> {
    let (req_url, req_token) = (
        env("ACTIONS_ID_TOKEN_REQUEST_URL"),
        env("ACTIONS_ID_TOKEN_REQUEST_TOKEN"),
    );
    if req_url.is_empty() || req_token.is_empty() {
        return Err("no OIDC token: a fork PR, or the job lacks `id-token: write`".into());
    }
    let url = format!("{req_url}&audience={}", encode(audience));
    let (status, body) = curl(&url, &auth_config("bearer", &req_token)?, None)?;
    if status != 200 {
        return Err(format!("OIDC token request: HTTP {status}"));
    }
    let v: Value = serde_json::from_str(&body).map_err(|_| "OIDC token: not JSON")?;
    v["value"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "OIDC token: no value".into())
}

/// POSTs `body` to `<GRAINPHITE_URL><path>` with the job's OIDC token.
fn call(path: &str, body: &str) -> Result<String, String> {
    let base = env("GRAINPHITE_URL");
    if base.is_empty() {
        return Err("GRAINPHITE_URL is not set".into());
    }
    if !valid_url(&base) {
        return Err(format!("GRAINPHITE_URL ({base:?}) is not an https origin"));
    }
    let token = oidc_token(&base)?;
    let (status, answer) = curl(
        &format!("{base}{path}"),
        &auth_config("Bearer", &token)?,
        Some(body),
    )?;
    if status != 200 {
        return Err(format!("{path}: HTTP {status}"));
    }
    Ok(answer)
}

fn head_sha() -> Result<String, String> {
    let sha = env("GRAINPHITE_HEAD_SHA");
    if valid_sha(&sha) {
        Ok(sha)
    } else {
        Err(format!("GRAINPHITE_HEAD_SHA ({sha:?}) is not a commit sha"))
    }
}

fn ask() -> Result<Answer, String> {
    let body = serde_json::json!({ "head_sha": head_sha()? }).to_string();
    parse_decide(&call("/ci/decide", &body)?)
}

/// Appends to a file GitHub reads after the step (outputs, job summary).
fn append(path: &str, text: &str) {
    if path.is_empty() {
        print!("{text}");
        return;
    }
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .unwrap_or_else(|e| fail(&format!("write {path}: {e}")));
}

pub(crate) fn gate(github_output: Option<&str>) {
    let answer = ask().unwrap_or_else(|e| {
        println!("::warning::grainphite: {e}");
        Answer::run_in_full(&e)
    });
    println!("grainphite: {}", answer.summary);
    append(
        &env("GITHUB_STEP_SUMMARY"),
        &format!("**grainphite CI gate:** {}\n", answer.summary),
    );
    append(github_output.unwrap_or_default(), &answer.output());
}

pub(crate) fn report() {
    let sent = result_from_needs(&env("GRAINPHITE_NEEDS")).and_then(|result| {
        let body = serde_json::json!({ "head_sha": head_sha()?, "result": result });
        call("/ci/report", &body.to_string()).map(|a| (result, a))
    });
    // A lost report only means the next restack runs in full: warn, not fail.
    match sent {
        Ok((result, answer)) => println!("grainphite: reported {result}: {answer}"),
        Err(e) => println!("::warning::grainphite report: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_answer() {
        let a = parse_decide(
            r#"{"run":true,"group":"ci.yml-grainphite-pr-1","summary":"bottom of its stack"}"#,
        )
        .unwrap();
        assert_eq!(a.output(), "skip=false\ngroup=ci.yml-grainphite-pr-1\n");
        assert_eq!(a.summary, "running in full: bottom of its stack");
    }

    #[test]
    fn a_skip_answer() {
        let a = parse_decide(r#"{"run":false,"group":"g","summary":"restack only"}"#).unwrap();
        assert!(a.skip);
        assert_eq!(a.summary, "skipped: restack only");
    }

    #[test]
    fn a_bad_answer_is_an_error_and_so_a_full_run() {
        for bad in [
            "",
            "<html>",
            r#"{"group":"g"}"#,
            r#"{"run":"false","group":"g"}"#,
            r#"{"run":false,"group":""}"#,
            r#"{"run":false,"group":"a\nskip=true"}"#,
            r#"{"run":false,"group":"${{ secrets.X }}"}"#,
        ] {
            assert!(parse_decide(bad).is_err(), "{bad}");
        }
        let fallback = Answer::run_in_full("down");
        assert_eq!(fallback.output(), "skip=false\ngroup=\n");
    }

    #[test]
    fn the_summary_cannot_break_lines() {
        let a = parse_decide(r#"{"run":true,"group":"g","summary":"a\n::error::b"}"#).unwrap();
        assert!(!a.summary.contains('\n'));
    }

    #[test]
    fn only_all_success_reports_success() {
        let ok = r#"{"gate":{"result":"success"},"test":{"result":"success"}}"#;
        assert_eq!(result_from_needs(ok), Ok("success"));
        for result in ["failure", "cancelled", "skipped"] {
            let needs =
                format!(r#"{{"gate":{{"result":"success"}},"test":{{"result":"{result}"}}}}"#);
            assert_eq!(result_from_needs(&needs), Ok("failure"), "{result}");
        }
        assert!(result_from_needs("{}").is_err());
        assert!(result_from_needs("").is_err());
    }

    #[test]
    fn urls_and_shas() {
        assert!(valid_url("https://grainphite-hooks.rainlang.xyz"));
        assert!(valid_url("https://localhost:8443"));
        for bad in [
            "http://grainphite.rainlang.xyz",
            "https://x.xyz/",
            "https://x.xyz/ci",
            "https://",
            "https://x.xyz?a",
        ] {
            assert!(!valid_url(bad), "{bad}");
        }
        assert!(valid_sha(&"a".repeat(40)));
        assert!(!valid_sha("main"));
        assert_eq!(encode("https://a.b:1"), "https%3A%2F%2Fa.b%3A1");
    }

    #[test]
    fn tokens_go_in_curl_config_only_when_plain() {
        assert!(auth_config("Bearer", "eyJ.abc-_.x").is_ok());
        for bad in ["", "a b", "a\"b", "a\\b", "a\nheader = \"x\""] {
            assert!(auth_config("Bearer", bad).is_err(), "{bad:?}");
        }
    }
}
