//! `verify-status-recheck` — re-read, with a GET, the verification status polls
//! `forge script --verify` could not read.
//!
//! 0G's chainscan (16661) accepts forge's submission and verifies the contract,
//! but its `checkverifystatus` reads `guid` only from the query string. forge
//! sends that poll as a POST form, so every poll is answered `Invalid parameter
//! guid with value [undefined]`, forge cannot parse it, and the run fails on a
//! contract the explorer has verified. No forge flag or config changes how the
//! poll is sent.
//!
//! This reads forge's log and, only when every failure in it is a poll forge got
//! no readable answer to, asks the same explorer for each of those GUIDs with a
//! GET. Exit 0 only when every one reads verified. Anything else — a broadcast
//! that did not complete, a rejected submission, an explorer verdict, a chain
//! forge never reached — stays red, as forge left it. Runs where `curl` is on
//! PATH (the nix package wraps the binary with a pinned curl).
//!
//! Remove this subcommand, its action and the workflow step once the forge
//! that rainix-manual-sol-artifacts pins polls `checkverifystatus` by GET.

use crate::{fail, subgraph_probe::valid_base, soldeer_gate::split_status_body};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const COMPLETE: &str = "ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.";
/// forge still prints COMPLETE after the RPC drops transactions, then verifies
/// what landed, so a partial deploy is only visible as this warning.
const DISCARDED: &str = "Some transactions were discarded by the RPC node.";
const SECTION: &str = "Start verification for (";
const CONTRACT: &str = "Start verifying contract `";
const FAILED: &str = "Failed to verify contract: ";
/// forge's wording when the submission landed and no poll answer parsed.
const UNREAD: &str = "Failed to verify contract: Checking verification result failed; \
                      Failed to request verification status";
const NOT_ALL: &str = "Not all (";
/// GET attempts per GUID, and the pause after each one that gets no verdict.
const ATTEMPTS: u32 = 20;
const DELAY: Duration = Duration::from_secs(15);

/// A contract forge submitted whose status poll it could not read.
#[derive(Debug, PartialEq)]
pub(crate) struct Unread {
    pub address: String,
    pub chain: String,
    pub guid: String,
}

/// `(a / b)` out of `Start verification for (N) contracts` or
/// `Not all (X / N) contracts were verified!`.
fn counts(line: &str, after: &str) -> Option<Vec<usize>> {
    let rest = &line[line.find(after)? + after.len()..];
    let inner = &rest[..rest.find(')')?];
    inner.split(" / ").map(|n| n.trim().parse().ok()).collect()
}

fn plain_hex_address(s: &str) -> bool {
    s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn plain_token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn header(line: &str) -> Result<(String, String), String> {
    let rest = &line[line.find(CONTRACT).ok_or("no contract header")? + CONTRACT.len()..];
    let (address, chain) = rest
        .split_once("` deployed on ")
        .ok_or_else(|| format!("unreadable contract header: {line}"))?;
    let chain = chain.trim();
    if !plain_hex_address(address) || !plain_token(chain) {
        return Err(format!("unreadable contract header: {line}"));
    }
    Ok((address.to_string(), chain.to_string()))
}

/// The contracts whose poll forge could not read, or why this log is not a
/// failure a GET can settle.
pub(crate) fn unread_polls(log: &str) -> Result<Vec<Unread>, String> {
    let lines: Vec<&str> = log.lines().map(str::trim).collect();
    if !lines.contains(&COMPLETE) {
        return Err("the broadcast did not complete".into());
    }
    if lines.iter().any(|l| l.contains(DISCARDED)) {
        return Err("the RPC discarded some of the broadcast's transactions".into());
    }
    // forge verifies one section per broadcast sequence, in order, stopping at
    // the first that fails. Fewer sections than chains is a chain nothing
    // verified, which a GET for the others cannot cover. This counts distinct
    // chains, not sequences, so a script that goes back to a chain it already
    // broadcast on is not caught here.
    let chains: BTreeSet<&str> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("Chain "))
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    let sections: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].starts_with(SECTION))
        .collect();
    let Some(&last) = sections.last() else {
        return Err("forge verified nothing".into());
    };
    if sections.len() != chains.len() {
        return Err(format!(
            "{} chains broadcast but forge started verification on {}",
            chains.len(),
            sections.len()
        ));
    }
    let not_all: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].contains(NOT_ALL) && lines[i].ends_with("contracts were verified!"))
        .collect();
    let [end] = not_all[..] else {
        return Err(format!(
            "expected forge's one `Not all (X / N)` failure, found {}",
            not_all.len()
        ));
    };
    let total = match counts(lines[last], SECTION).as_deref() {
        Some(&[n]) => n,
        _ => return Err(format!("unreadable section header: {}", lines[last])),
    };
    let passed = match counts(lines[end], NOT_ALL).as_deref() {
        Some(&[x, n]) if end > last && n == total => x,
        _ => return Err(format!("`{}` does not close the last section", lines[end])),
    };

    let starts: Vec<usize> = (last..end)
        .filter(|&i| lines[i].starts_with(CONTRACT))
        .collect();
    if starts.len() != total {
        return Err(format!(
            "forge announced {total} verifications and started {}",
            starts.len()
        ));
    }
    let mut unread = Vec::new();
    let mut succeeded = 0;
    for (k, &start) in starts.iter().enumerate() {
        let block = &lines[start..*starts.get(k + 1).unwrap_or(&end)];
        let (address, chain) = header(lines[start])?;
        let failures: Vec<&&str> = block.iter().filter(|l| l.contains(FAILED)).collect();
        match failures[..] {
            [] => succeeded += 1,
            [failure] if failure.contains(UNREAD) => {
                let guids: Vec<&str> = block
                    .iter()
                    .filter_map(|l| l.strip_prefix("GUID: `")?.strip_suffix('`'))
                    .collect();
                let [guid] = guids[..] else {
                    return Err(format!(
                        "{address}: expected one GUID, found {}",
                        guids.len()
                    ));
                };
                if !plain_token(guid) {
                    return Err(format!("{address}: unreadable GUID {guid:?}"));
                }
                unread.push(Unread {
                    address,
                    chain,
                    guid: guid.to_string(),
                });
            }
            _ => {
                return Err(format!(
                    "{address} failed for a reason other than an unread poll"
                ))
            }
        }
    }
    if succeeded != passed {
        return Err(format!(
            "forge reported {passed} verified and the log shows {succeeded}"
        ));
    }
    if unread.is_empty() {
        return Err("the log shows no unread poll".into());
    }
    Ok(unread)
}

/// The one verifier url `[etherscan]` names for `chain`, out of
/// `forge config --json`. An entry with no url is Etherscan's own API, which
/// this does not recheck. On a fresh checkout the first `nix develop` prints
/// its git-hook install to stdout ahead of the JSON, so that is skipped.
pub(crate) fn explorer_url(config: &str, chain: &str) -> Result<String, String> {
    let start = match config.find('{') {
        Some(0) | None => 0,
        Some(_) => config.find("\n{").map_or(config.len(), |i| i + 1),
    };
    let v: Value = serde_json::Deserializer::from_str(&config[start..])
        .into_iter()
        .next()
        .ok_or("forge config is empty")?
        .map_err(|e| format!("forge config is not JSON: {e}"))?;
    let entries = v
        .get("etherscan")
        .and_then(Value::as_object)
        .ok_or("forge config has no [etherscan] table")?;
    let urls: BTreeSet<&str> = entries
        .values()
        .filter(|e| match e.get("chain") {
            Some(Value::Number(n)) => n.to_string() == chain,
            Some(Value::String(s)) => s == chain,
            _ => false,
        })
        .map(|e| e.get("url").and_then(Value::as_str).unwrap_or(""))
        .collect();
    match urls.into_iter().collect::<Vec<_>>()[..] {
        [url] if !url.is_empty() => valid_base(url)
            .map(str::to_string)
            .map_err(|e| format!("chain {chain}: {e}")),
        [] => Err(format!("no [etherscan] entry for chain {chain}")),
        _ => Err(format!(
            "chain {chain} has no single [etherscan] url to recheck against"
        )),
    }
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Verified(String),
    Retry(String),
    Fatal(String),
}

/// One response -> one verdict, with forge's own reading of each result.
fn classify(status: u16, body: &str) -> Verdict {
    if status == 429 || status >= 500 {
        return Verdict::Retry(format!("HTTP {status}"));
    }
    if status != 200 {
        return Verdict::Fatal(format!("HTTP {status}"));
    }
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let (Some(ok), Some(result)) = (
        v.get("status").and_then(Value::as_str),
        v.get("result").and_then(Value::as_str),
    ) else {
        return Verdict::Fatal(format!("unrecognised response: {}", excerpt(body)));
    };
    match (ok, result) {
        (_, "Pending in queue" | "Unable to verify") => Verdict::Retry(result.to_string()),
        (_, r) if r.starts_with("Error: contract does not exist") => Verdict::Retry(r.to_string()),
        // Etherscan's rate limit is an HTTP 200; forge waits and retries it too.
        (_, r) if r.starts_with("Max rate limit reached") => Verdict::Retry(r.to_string()),
        (_, "Already Verified") | ("1", "Pass - Verified") => Verdict::Verified(result.to_string()),
        _ => Verdict::Fatal(format!("status {ok}: {}", excerpt(result))),
    }
}

fn excerpt(body: &str) -> String {
    let flat: String = body
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.chars().take(200).collect()
}

/// Drive `fetch` until it yields a verdict. The fetch and sleep are
/// parameters so the retry policy is testable without a network or a clock.
fn poll_with(
    mut fetch: impl FnMut() -> Result<(u16, String), String>,
    mut sleep: impl FnMut(Duration),
    attempts: u32,
    delay: Duration,
) -> Result<String, String> {
    let mut last = String::new();
    for attempt in 1..=attempts {
        let verdict = match fetch() {
            Ok((status, body)) => classify(status, &body),
            Err(transport) => Verdict::Retry(transport),
        };
        match verdict {
            Verdict::Verified(r) => return Ok(r),
            Verdict::Fatal(e) => return Err(e),
            Verdict::Retry(e) => {
                last = e;
                if attempt < attempts {
                    sleep(delay);
                }
            }
        }
    }
    Err(format!("no verdict after {attempts} attempts: {last}"))
}

/// `checkverifystatus` as a GET, returning (HTTP status, body). Not `-L`: a
/// redirect is a failure, not something to follow to another host.
fn curl_status(url: &str, guid: &str) -> Result<(u16, String), String> {
    let out = Command::new("curl")
        .args(["-sS", "--proto", "=https", "--connect-timeout", "15"])
        .args(["--max-time", "60", "--get", "-w", "\n%{http_code}"])
        .args(["--data-urlencode", "module=contract"])
        .args(["--data-urlencode", "action=checkverifystatus"])
        .args(["--data-urlencode", &format!("guid={guid}"), url])
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

pub(crate) fn run(log: &Path, config: &Path) -> ! {
    let read = |p: &Path| {
        std::fs::read_to_string(p)
            .unwrap_or_else(|e| fail(&format!("verify-status-recheck: {}: {e}", p.display())))
    };
    let unread = unread_polls(&read(log)).unwrap_or_else(|e| {
        fail(&format!(
            "verify-status-recheck: forge's failure is not an unread status poll, so it stands: {e}"
        ))
    });
    let config = read(config);
    let mut failed = Vec::new();
    for u in &unread {
        let verdict = explorer_url(&config, &u.chain).and_then(|url| {
            poll_with(
                || curl_status(&url, &u.guid),
                std::thread::sleep,
                ATTEMPTS,
                DELAY,
            )
        });
        match verdict {
            Ok(r) => println!("{} on chain {}: {r} (GUID {})", u.address, u.chain, u.guid),
            Err(e) => failed.push(format!("{} on chain {}: {e}", u.address, u.chain)),
        }
    }
    if !failed.is_empty() {
        fail(&format!("verify-status-recheck: {}", failed.join("; ")));
    }
    println!(
        "verify-status-recheck: the explorer reads all {} unread polls as verified",
        unread.len()
    );
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const ADDR: &str = "0xd708186f68D527B64c001398550627508205EE31";
    const GUID: &str = "5cdcdb27-317e-4d3f-a2b7-3d567773dc49";
    const POLL_BODY: &str = r#"{"status":"0","message":"Invalid parameter guid with value [undefined], guid is required.","result":{}}"#;

    /// Rain deploy run 44597944851 on 0G, two poll retries kept of eight. Its
    /// forge is 1.7.2-nightly 43923a4, from rainix-manual-sol-artifacts'
    /// RAINIX_SHA 41a94b6; capture a new 0G log when that pin moves.
    fn chainscan_log() -> String {
        format!(
            "\n==========================\n\nChain 16661\n\nEstimated gas price: 4.000000014 gwei\n\
             \n==========================\n\nONCHAIN EXECUTION COMPLETE & SUCCESSFUL.\n##\n\
             Start verification for (1) contracts\n\
             ETHERSCAN_API_KEY is set, defaulting to Etherscan verifier.\n\
             Start verifying contract `{ADDR}` deployed on 16661\nEVM version: cancun\n\n\
             Submitting verification for [src/concrete/MigrationRegistry.sol:MigrationRegistry] {ADDR}.\n\
             Submitted contract for verification:\n\tResponse: `OK`\n\tGUID: `{GUID}`\n\
             \tURL: https://chainscan.0g.ai/open/address/{ADDR}\n\
             \u{1b}[2m2026-11-19T19:29:09Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m etherscan: Failed to deserialize response\n\
             Warning: Failed to request verification status; Failed to deserialize content: data did not match any variant of untagged enum ResponseData\n\
             {POLL_BODY}; waiting 15 seconds before trying again (0 tries remaining)\n\
             Error: Failed to verify contract: Checking verification result failed; Failed to request verification status; Failed to deserialize content: data did not match any variant of untagged enum ResponseData\n\
             {POLL_BODY}\n\nTransactions saved to: broadcast/Deploy.sol/16661/run-latest.json\n\n\
             Error: Not all (0 / 1) contracts were verified!\n"
        )
    }

    fn config(entries: &str) -> String {
        format!(r#"{{"src":"src","etherscan":{{{entries}}}}}"#)
    }

    const ZEROG: &str = r#""base":{"chain":"base","key":"${K}"},"zerog":{"chain":16661,"url":"https://chainscan.0g.ai/open/api","key":"${Z}"}"#;

    #[test]
    fn a_chainscan_poll_failure_is_unread() {
        assert_eq!(
            unread_polls(&chainscan_log()),
            Ok(vec![Unread {
                address: ADDR.into(),
                chain: "16661".into(),
                guid: GUID.into(),
            }])
        );
    }

    #[test]
    fn an_unfinished_broadcast_stands() {
        let log = chainscan_log().replace(COMPLETE, "Error: transaction reverted");
        assert!(unread_polls(&log).unwrap_err().contains("did not complete"));
    }

    #[test]
    fn discarded_transactions_stand() {
        // forge still prints COMPLETE, then verifies only what landed.
        let log = chainscan_log().replace(
            "##\n",
            "##\nSome transactions were discarded by the RPC node. Use `--resume` to retry these transactions.\n",
        );
        assert!(unread_polls(&log).unwrap_err().contains("discarded"));
    }

    #[test]
    fn a_chain_forge_never_verified_stands() {
        // A second broadcasting chain after the failed one: forge stopped.
        let log = chainscan_log().replace("Chain 16661\n", "Chain 16661\nChain 8453\n");
        assert!(unread_polls(&log).unwrap_err().contains("2 chains"));
    }

    #[test]
    fn an_explorer_verdict_stands() {
        let log = chainscan_log().replace(
            "Checking verification result failed; Failed to request verification status;",
            "Checking verification result failed; Contract verification failed:",
        );
        assert!(unread_polls(&log).unwrap_err().contains("other than"));
    }

    #[test]
    fn a_rejected_submission_stands() {
        let log = chainscan_log()
            .replace(&format!("\tGUID: `{GUID}`\n"), "")
            .replace(
                "Checking verification result failed; Failed to request verification status;",
                "Failed to submit contract verification;",
            );
        assert!(unread_polls(&log).is_err());
    }

    #[test]
    fn a_missing_guid_stands() {
        let log = chainscan_log().replace(&format!("\tGUID: `{GUID}`\n"), "");
        assert!(unread_polls(&log)
            .unwrap_err()
            .contains("one GUID, found 0"));
    }

    #[test]
    fn counts_must_agree_with_the_blocks() {
        let log = chainscan_log().replace("for (1) contracts", "for (2) contracts");
        assert!(unread_polls(&log).is_err());
        let log = chainscan_log().replace("Not all (0 / 1)", "Not all (1 / 1)");
        assert!(unread_polls(&log).is_err());
        let log = chainscan_log().replace("Error: Not all (0 / 1) contracts were verified!\n", "");
        assert!(unread_polls(&log).unwrap_err().contains("found 0"));
    }

    #[test]
    fn a_failure_with_no_unread_poll_stands() {
        // `Not all (1 / 1)` with the one block clean: nothing for a GET to settle.
        let failure = chainscan_log()
            .lines()
            .find(|l| l.starts_with("Error: Failed to verify contract: "))
            .unwrap()
            .to_string();
        let log = chainscan_log()
            .replace(&format!("{failure}\n"), "")
            .replace("Not all (0 / 1)", "Not all (1 / 1)");
        assert!(unread_polls(&log).unwrap_err().contains("no unread poll"));
    }

    #[test]
    fn a_passing_contract_beside_an_unread_one_counts() {
        let other = "0x82b7e6FDfD25EF64b013F98e8F2c1fC1D1872eC9";
        let log = chainscan_log()
            .replace("for (1) contracts", "for (2) contracts")
            .replace("Not all (0 / 1)", "Not all (1 / 2)")
            .replace(
                "ETHERSCAN_API_KEY is set, defaulting to Etherscan verifier.\n",
                &format!(
                    "Start verifying contract `{other}` deployed on 16661\n\
                     Contract source code already verified\n"
                ),
            );
        let unread = unread_polls(&log).unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].address, ADDR);
    }

    #[test]
    fn the_url_is_the_chains_own_entry() {
        assert_eq!(
            explorer_url(&config(ZEROG), "16661").unwrap(),
            "https://chainscan.0g.ai/open/api"
        );
        // Etherscan's own entries carry no url; nothing to recheck against.
        assert!(explorer_url(&config(ZEROG), "base").is_err());
        assert!(explorer_url(&config(ZEROG), "1").is_err());
        let two =
            format!(r#"{ZEROG},"zerog2":{{"chain":16661,"url":"https://other.example/api"}}"#);
        assert!(explorer_url(&config(&two), "16661").is_err());
        let plain = r#""z":{"chain":16661,"url":"http://chainscan.0g.ai/open/api"}"#;
        assert!(explorer_url(&config(plain), "16661").is_err());
        assert!(explorer_url("not json", "16661").is_err());
        assert!(explorer_url("", "16661").is_err());
        let hooked = format!(
            "/w/.pre-commit-config.yaml\npre-commit installed at .git/hooks/pre-commit\n{}\n",
            config(ZEROG)
        );
        assert_eq!(
            explorer_url(&hooked, "16661").unwrap(),
            "https://chainscan.0g.ai/open/api"
        );
    }

    #[test]
    fn results_read_as_forge_reads_them() {
        let body = |status: &str, result: &str| {
            format!(r#"{{"status":"{status}","message":"OK","result":"{result}"}}"#)
        };
        assert!(matches!(
            classify(200, &body("1", "Pass - Verified")),
            Verdict::Verified(_)
        ));
        assert!(matches!(
            classify(200, &body("1", "Already Verified")),
            Verdict::Verified(_)
        ));
        for retry in [
            body("0", "Pending in queue"),
            body("0", "Unable to verify"),
            body("0", "Error: contract does not exist"),
            r#"{"status":"0","message":"NOTOK","result":"Max rate limit reached"}"#.to_string(),
        ] {
            assert!(
                matches!(classify(200, &retry), Verdict::Retry(_)),
                "{retry}"
            );
        }
        assert!(matches!(classify(429, ""), Verdict::Retry(_)));
        for fatal in [
            body("0", "Fail - Unable to verify"),
            body("0", "Pass - Verified"),
            body("1", "something else"),
            POLL_BODY.to_string(),
            "<html>".to_string(),
        ] {
            assert!(
                matches!(classify(200, &fatal), Verdict::Fatal(_)),
                "{fatal}"
            );
        }
        assert!(matches!(classify(503, ""), Verdict::Retry(_)));
        assert!(matches!(
            classify(302, &body("1", "Pass - Verified")),
            Verdict::Fatal(_)
        ));
    }

    #[test]
    fn pending_retries_until_verified() {
        let calls = Cell::new(0);
        let sleeps = RefCell::new(Vec::new());
        let r = poll_with(
            || {
                calls.set(calls.get() + 1);
                match calls.get() {
                    1 => Err("curl: (28) timeout".into()),
                    2 => Ok((200, r#"{"status":"0","result":"Pending in queue"}"#.into())),
                    _ => Ok((200, r#"{"status":"1","result":"Pass - Verified"}"#.into())),
                }
            },
            |d| sleeps.borrow_mut().push(d),
            5,
            Duration::from_secs(15),
        );
        assert_eq!(r.as_deref(), Ok("Pass - Verified"));
        assert_eq!(*sleeps.borrow(), vec![Duration::from_secs(15); 2]);
    }

    #[test]
    fn retries_run_out_and_fatal_stops() {
        let r = poll_with(|| Ok((503, String::new())), |_| {}, 3, Duration::ZERO);
        assert!(r.unwrap_err().contains("after 3 attempts"));
        let calls = Cell::new(0);
        let r = poll_with(
            || {
                calls.set(calls.get() + 1);
                Ok((200, POLL_BODY.into()))
            },
            |_| panic!("a fatal response must not sleep"),
            3,
            Duration::ZERO,
        );
        assert!(r.is_err());
        assert_eq!(calls.get(), 1);
    }
}
