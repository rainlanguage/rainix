//! `ormi-probe` — is `<name>/<version>` already deployed on Ormi?
//!
//! `subgraph-deploy` skips a version that is already live and deploys one that
//! is not. The two answers are only safe if everything else is refused: a
//! transport failure, an HTTP error, a body that is not one of the two known
//! shapes, or a wrong query base must never read as "not deployed", because the
//! next step is a deploy under a version label that may already exist.
//!
//! Ormi answers every query with HTTP 200 and tells the cases apart by body:
//!   {"data":{"_meta":{"block":{"number":N}}}}   deployed and answering
//!   {"error":"subgraph name/version error"}      this name/version does not exist
//!   {"error":"path project error"}               wrong account in the query base
//! Only the first two are verdicts; the third, like anything else, is a failure.
//! Transport errors, 429 and 5xx are retried with backoff before failing.
//!
//! Exit status: 0 deployed (prints `deployed`), 10 confirmed missing (prints
//! `missing`), 1 anything else. Nothing here handles the deploy key; the probe
//! hits the public query endpoint and needs no secret. Runs where `curl` is on
//! PATH (the nix package wraps the binary with a pinned curl).

use crate::{fail, soldeer_gate::split_status_body};
use serde_json::Value;
use std::process::Command;
use std::time::Duration;

/// Exit status for a confirmed-missing version, distinct from the generic
/// failure status 1 so the calling task can branch without parsing output.
pub(crate) const MISSING_EXIT: i32 = 10;

/// The exact body Ormi returns for a name/version it does not host.
const MISSING_ERROR: &str = "subgraph name/version error";

const QUERY: &str = r#"{"query":"{ _meta { block { number } } }"}"#;

/// Total tries for a transient failure, and the pause before try N+1 is
/// `BACKOFF * N`.
const ATTEMPTS: u32 = 4;
const BACKOFF: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq)]
pub(crate) enum Probe {
    Deployed,
    Missing,
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Answer(Probe),
    /// Worth another try: the answer did not come from a healthy endpoint.
    Retry(String),
    /// Not worth another try, and not a safe basis for a deploy.
    Fatal(String),
}

/// A name or version label that is safe to put in a URL path and a CLI
/// argument: ASCII alphanumerics plus `.`, `_` and `-`, starting with an
/// alphanumeric. Real values are `<SUBGRAPH_NAME>-<network>` and
/// `<address>-<commit>`. This runs before any request is made, so a
/// networks.json key or address (repo content) can never reshape the URL.
fn valid_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && s.as_bytes()[0].is_ascii_alphanumeric()
}

/// The Ormi query base: an `https://` URL with no query, fragment, quoting or
/// whitespace. Plain-text HTTP is refused because the skip decision is made on
/// the response, and a response an attacker can rewrite in transit can make the
/// task skip a deploy.
fn valid_base(s: &str) -> Result<&str, String> {
    let base = s.trim_end_matches('/');
    let host_and_path = base
        .strip_prefix("https://")
        .ok_or_else(|| format!("query base {s:?} must start with https://"))?;
    if host_and_path.is_empty()
        || host_and_path.starts_with('/')
        || !host_and_path
            .bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b'\'' | b'\\' | b'?' | b'#'))
    {
        return Err(format!(
            "query base {s:?} is not a plain https URL (no query, fragment, quotes or whitespace)"
        ));
    }
    Ok(base)
}

fn probe_url(base: &str, name: &str, version: &str) -> Result<String, String> {
    let base = valid_base(base)?;
    for (what, v) in [("name", name), ("version", version)] {
        if !valid_label(v) {
            return Err(format!(
                "{what} {v:?} is not a plain label (ASCII alphanumerics, '.', '_', '-')"
            ));
        }
    }
    Ok(format!("{base}/subgraphs/{name}/{version}/gn"))
}

/// Cap a response for an error message: it is attacker-influenced text and may
/// be a whole HTML page.
fn excerpt(body: &str) -> String {
    let flat: String = body
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut out: String = flat.chars().take(200).collect();
    if flat.chars().count() > 200 {
        out.push('…');
    }
    out
}

/// One response -> one verdict. See the module doc for the accepted shapes.
fn classify(status: u16, body: &str) -> Verdict {
    if status == 429 || status >= 500 {
        return Verdict::Retry(format!("HTTP {status}: {}", excerpt(body)));
    }
    if status != 200 {
        return Verdict::Fatal(format!("HTTP {status}: {}", excerpt(body)));
    }
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Verdict::Fatal(format!("response is not JSON: {}", excerpt(body)));
    };
    let Some(obj) = v.as_object() else {
        return Verdict::Fatal(format!("response is not an object: {}", excerpt(body)));
    };
    // `errors` alongside `data` is a GraphQL failure, not a healthy answer.
    if !obj.contains_key("errors") {
        if let Some(n) = v.pointer("/data/_meta/block/number") {
            if n.is_u64() {
                return Verdict::Answer(Probe::Deployed);
            }
        }
    }
    if obj.len() == 1 && obj.get("error").and_then(Value::as_str) == Some(MISSING_ERROR) {
        return Verdict::Answer(Probe::Missing);
    }
    Verdict::Fatal(format!(
        "unrecognised response, refusing to treat it as deployed or missing: {}",
        excerpt(body)
    ))
}

/// Drive `fetch` until it yields a verdict, retrying transient failures. The
/// fetch and sleep are parameters so the retry policy is testable without a
/// network or a clock.
fn probe_with(
    mut fetch: impl FnMut() -> Result<(u16, String), String>,
    mut sleep: impl FnMut(Duration),
    attempts: u32,
) -> Result<Probe, String> {
    let mut last = String::new();
    for attempt in 1..=attempts {
        let verdict = match fetch() {
            Ok((status, body)) => classify(status, &body),
            Err(transport) => Verdict::Retry(transport),
        };
        match verdict {
            Verdict::Answer(p) => return Ok(p),
            Verdict::Fatal(e) => return Err(e),
            Verdict::Retry(e) => {
                eprintln!("ormi-probe: attempt {attempt}/{attempts} failed: {e}");
                last = e;
                if attempt < attempts {
                    sleep(BACKOFF * attempt);
                }
            }
        }
    }
    Err(format!("gave up after {attempts} attempts: {last}"))
}

/// POST the `_meta` query with curl, returning (HTTP status, body). Not `-f`
/// (the status is classified here) and not `-L` (a redirect is a failure, not
/// something to follow to another host). `--proto =https` pins the scheme.
fn curl_probe(url: &str) -> Result<(u16, String), String> {
    let out = Command::new("curl")
        .args([
            "-sS",
            "--proto",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "60",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "--data",
            QUERY,
            "-w",
            "\n%{http_code}",
            url,
        ])
        .output()
        .map_err(|e| format!("curl: failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl: {} ({})",
            String::from_utf8_lossy(&out.stderr).trim(),
            out.status
        ));
    }
    split_status_body(&String::from_utf8_lossy(&out.stdout))
}

pub(crate) fn run(base: &str, name: &str, version: &str) -> ! {
    let url = probe_url(base, name, version).unwrap_or_else(|e| fail(&format!("ormi-probe: {e}")));
    match probe_with(|| curl_probe(&url), std::thread::sleep, ATTEMPTS) {
        Ok(Probe::Deployed) => {
            println!("deployed");
            std::process::exit(0);
        }
        Ok(Probe::Missing) => {
            println!("missing");
            std::process::exit(MISSING_EXIT);
        }
        Err(e) => fail(&format!("ormi-probe: {name}/{version}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const LIVE: &str = r#"{"data":{"_meta":{"block":{"number":51907769}}}}"#;
    const MISSING: &str = r#"{"error":"subgraph name/version error"}"#;

    fn answer(status: u16, body: &str) -> Verdict {
        classify(status, body)
    }

    #[test]
    fn live_meta_is_deployed() {
        assert_eq!(answer(200, LIVE), Verdict::Answer(Probe::Deployed));
    }

    #[test]
    fn exact_missing_error_is_missing() {
        assert_eq!(answer(200, MISSING), Verdict::Answer(Probe::Missing));
        assert_eq!(
            answer(200, "  {\"error\" : \"subgraph name/version error\"}\n"),
            Verdict::Answer(Probe::Missing)
        );
    }

    #[test]
    fn wrong_account_is_not_missing() {
        // A mistyped query base answers "path project error"; reading that as
        // "not deployed" would redeploy every network on every run.
        assert!(matches!(
            answer(200, r#"{"error":"path project error"}"#),
            Verdict::Fatal(_)
        ));
    }

    #[test]
    fn unexpected_bodies_are_fatal() {
        for body in [
            "",
            "not json",
            "<html>502</html>",
            "[]",
            "null",
            r#"{}"#,
            r#"{"data":null}"#,
            r#"{"data":{"_meta":null}}"#,
            r#"{"data":{"_meta":{"block":{"number":null}}}}"#,
            r#"{"data":{"_meta":{"block":{"number":"7"}}}}"#,
            r#"{"errors":[{"message":"indexing_error"}]}"#,
            // GraphQL errors next to data: a failed subgraph is not a live one.
            r#"{"data":{"_meta":{"block":{"number":1}}},"errors":[{"message":"x"}]}"#,
            // The missing error plus anything else is not the known shape.
            r#"{"error":"subgraph name/version error","extra":1}"#,
            r#"{"error":"something else"}"#,
        ] {
            assert!(
                matches!(answer(200, body), Verdict::Fatal(_)),
                "{body:?} must be fatal"
            );
        }
    }

    #[test]
    fn http_errors_never_yield_an_answer() {
        // Even with a body that would be an answer at 200.
        for status in [301, 302, 400, 401, 403, 404] {
            assert!(matches!(answer(status, MISSING), Verdict::Fatal(_)));
            assert!(matches!(answer(status, LIVE), Verdict::Fatal(_)));
        }
        for status in [429, 500, 502, 503, 504] {
            assert!(matches!(answer(status, MISSING), Verdict::Retry(_)));
            assert!(matches!(answer(status, LIVE), Verdict::Retry(_)));
        }
    }

    #[test]
    fn transport_failure_retries_then_fails() {
        let calls = Cell::new(0);
        let sleeps = RefCell::new(Vec::new());
        let r = probe_with(
            || {
                calls.set(calls.get() + 1);
                Err("curl: (7) connection refused".to_string())
            },
            |d| sleeps.borrow_mut().push(d),
            3,
        );
        assert!(r.unwrap_err().contains("gave up after 3 attempts"));
        assert_eq!(calls.get(), 3);
        // Backoff grows and there is no pause after the last try.
        assert_eq!(*sleeps.borrow(), vec![BACKOFF, BACKOFF * 2]);
    }

    #[test]
    fn transient_failure_then_answer_succeeds() {
        let calls = Cell::new(0);
        let r = probe_with(
            || {
                calls.set(calls.get() + 1);
                match calls.get() {
                    1 => Err("curl: (28) timeout".to_string()),
                    2 => Ok((503, "busy".to_string())),
                    _ => Ok((200, MISSING.to_string())),
                }
            },
            |_| {},
            4,
        );
        assert_eq!(r, Ok(Probe::Missing));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn fatal_response_is_not_retried() {
        let calls = Cell::new(0);
        let r = probe_with(
            || {
                calls.set(calls.get() + 1);
                Ok((200, r#"{"error":"path project error"}"#.to_string()))
            },
            |_| panic!("a fatal response must not sleep"),
            4,
        );
        assert!(r.is_err());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn labels_must_be_plain() {
        for ok in [
            "raindex-robinhood",
            "0xAbC0000000000000000000000000000000000000-abc1234",
            "a",
            "v1.2_3",
        ] {
            assert!(valid_label(ok), "{ok:?}");
        }
        for bad in [
            "",
            "-x",
            ".x",
            "a/b",
            "a b",
            "a?b",
            "a#b",
            "a%2fb",
            "..",
            "a\nb",
            "a\"b",
            "naïve",
            &"a".repeat(129),
        ] {
            assert!(!valid_label(bad), "{bad:?}");
        }
    }

    #[test]
    fn base_must_be_plain_https() {
        assert_eq!(
            valid_base("https://subgraph.api.ormilabs.com/api/public/abc/").unwrap(),
            "https://subgraph.api.ormilabs.com/api/public/abc"
        );
        for bad in [
            "",
            "http://subgraph.api.ormilabs.com/api/public/abc",
            "ftp://x",
            "https://",
            "https:///x",
            "https://host/path?x=1",
            "https://host/path#frag",
            "https://host/pa th",
            "https://host/\"x",
            "https://host/\nx",
            "file:///etc/passwd",
        ] {
            assert!(valid_base(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn url_is_built_from_validated_parts() {
        assert_eq!(
            probe_url("https://h/api/public/id/", "raindex-base", "0xabc-1234567").unwrap(),
            "https://h/api/public/id/subgraphs/raindex-base/0xabc-1234567/gn"
        );
        assert!(probe_url("https://h/x", "a/../b", "v").is_err());
        assert!(probe_url("https://h/x", "a", "v?x=1").is_err());
        assert!(probe_url("http://h/x", "a", "v").is_err());
    }

    #[test]
    fn excerpt_is_bounded_and_single_line() {
        let long = "x".repeat(500);
        assert!(excerpt(&long).chars().count() <= 201);
        assert_eq!(excerpt("a\nb\r\nc"), "a b  c");
    }
}
