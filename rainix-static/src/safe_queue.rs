//! `safe-queue` — the Safe's queued (proposed, unexecuted) transactions for a
//! script that exposes `queueSafe() returns (address)`, written as parallel
//! arrays for the script's `setUp()` to execute on the fork before its
//! entrypoint. Whether the script opts in is read from its compiled ABI, so
//! run it after `forge build`. Runs where forge, cast and curl are on PATH.

use crate::{fail, soldeer_gate::split_status_body};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const USER_AGENT: &str = "rainix-static";

/// Whether a forge artifact's ABI has a function named `queueSafe`.
pub(crate) fn abi_has_queue_safe(artifact: &Value) -> bool {
    artifact
        .get("abi")
        .and_then(Value::as_array)
        .is_some_and(|abi| {
            abi.iter().any(|e| {
                e.get("type").and_then(Value::as_str) == Some("function")
                    && e.get("name").and_then(Value::as_str) == Some("queueSafe")
            })
        })
}

/// forge's artifact dir for a script: `<out>/<file name>`.
pub(crate) fn artifact_dir(out: &Path, script: &str) -> Result<PathBuf, String> {
    let name = Path::new(script)
        .file_name()
        .ok_or_else(|| format!("--script {script:?} has no file name"))?;
    Ok(out.join(name))
}

/// Whether any `*.json` artifact in `dir` has `queueSafe` in its ABI. A
/// missing dir or an unreadable artifact is an error.
pub(crate) fn script_has_queue_safe(dir: &Path) -> Result<bool, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut found = false;
    for entry in rd {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let at = |e: String| format!("{}: {e}", path.display());
        let text = std::fs::read_to_string(&path).map_err(|e| at(e.to_string()))?;
        let artifact: Value = serde_json::from_str(&text).map_err(|e| at(e.to_string()))?;
        found |= abi_has_queue_safe(&artifact);
    }
    Ok(found)
}

fn is_address(s: &str) -> bool {
    s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// The address returned by `forge script --sig 'queueSafe()' --json`, from the
/// output line carrying a non-null `returns`.
pub(crate) fn parse_forge_returns(stdout: &str) -> Result<String, String> {
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let Some(returns) = v.get("returns").filter(|r| !r.is_null()) else {
            continue;
        };
        let addr = returns
            .get("0")
            .and_then(|r| r.get("value"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("queueSafe() returns has no address: {returns}"))?;
        if !is_address(addr) {
            return Err(format!("queueSafe() returned {addr:?}, not an address"));
        }
        return Ok(addr.to_string());
    }
    Err("forge output has no line carrying queueSafe()'s returns".to_string())
}

/// The leading decimal integer of a `cast` output (`5` or `5 [5e0]`).
pub(crate) fn parse_cast_uint(stdout: &str) -> Result<u64, String> {
    let tok = stdout
        .split_whitespace()
        .next()
        .ok_or_else(|| "cast printed nothing".to_string())?;
    tok.parse()
        .map_err(|_| format!("cast printed {tok:?}, not a u64"))
}

pub(crate) fn config_url(chain_id: u64) -> String {
    format!("https://safe-config.safe.global/api/v1/chains/{chain_id}/")
}

/// `.transactionService` of a Safe config chain response, without a trailing
/// slash.
pub(crate) fn transaction_service(config: &Value) -> Result<String, String> {
    let s = config
        .get("transactionService")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Safe config has no transactionService".to_string())?;
    Ok(s.trim_end_matches('/').to_string())
}

pub(crate) fn queue_url(service: &str, safe: &str, nonce: u64) -> String {
    format!(
        "{service}/api/v1/safes/{safe}/multisig-transactions/\
         ?executed=false&nonce__gte={nonce}&ordering=nonce&limit=100"
    )
}

fn tx_nonce(tx: &Value) -> Result<u64, String> {
    match tx.get("nonce") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
    .ok_or_else(|| format!("queued transaction has no integer nonce: {tx}"))
}

fn field(tx: &Value, key: &str) -> Result<Value, String> {
    tx.get(key)
        .filter(|v| !v.is_null())
        .cloned()
        .ok_or_else(|| format!("queued transaction has no {key}: {tx}"))
}

/// The queue file for one page of the multisig-transactions list, refused
/// unless it is the whole queue: one proposal per nonce, from `nonce` with no
/// gap. Null `data` is written as `"0x"`; other values as the API gives them.
pub(crate) fn build_queue(page: &Value, nonce: u64) -> Result<Value, String> {
    if page.get("next").is_some_and(|n| !n.is_null()) {
        return Err("the queue does not fit in one page".to_string());
    }
    let results = page
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "multisig-transactions response has no results".to_string())?;
    let mut seen = HashSet::new();
    let mut nonces = Vec::with_capacity(results.len());
    for tx in results {
        let n = tx_nonce(tx)?;
        if !seen.insert(n) {
            return Err(format!("two proposals at nonce {n}"));
        }
        nonces.push(n);
    }
    for (i, n) in nonces.iter().enumerate() {
        let want = nonce + i as u64;
        if *n != want {
            return Err(format!(
                "the queue has a gap: nonce {n} where {want} is next"
            ));
        }
    }
    let keys = ["to", "value", "data", "operation", "nonce", "safeTxHash"];
    let mut cols: Vec<(&str, Vec<Value>)> = keys
        .into_iter()
        .map(|k| (k, Vec::with_capacity(results.len())))
        .collect();
    for tx in results {
        for (k, col) in &mut cols {
            let v = if *k == "data" {
                tx.get("data")
                    .filter(|d| !d.is_null())
                    .cloned()
                    .unwrap_or_else(|| Value::String("0x".to_string()))
            } else {
                field(tx, *k)?
            };
            col.push(v);
        }
    }
    let mut out = Map::new();
    for (k, col) in cols {
        out.insert(k.to_string(), Value::Array(col));
    }
    Ok(Value::Object(out))
}

fn run_tool(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd} {}: {}\n{}\n{}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stdout).trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// GET `url` with curl and parse a 200 body as JSON. Not `-L`: a redirect is
/// a failure. `--proto =https` pins the scheme.
fn curl_json(url: &str) -> Result<Value, String> {
    let stdout = run_tool(
        "curl",
        &[
            "-sS",
            "--proto",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "60",
            "-A",
            USER_AGENT,
            "-w",
            "\n%{http_code}",
            url,
        ],
    )?;
    let (status, body) = split_status_body(&stdout)?;
    if status != 200 {
        return Err(format!("GET {url}: HTTP {status}: {}", body.trim()));
    }
    serde_json::from_str(&body).map_err(|e| format!("GET {url}: {e}"))
}

fn or_fail<T>(r: Result<T, String>) -> T {
    r.unwrap_or_else(|e| fail(&format!("safe-queue: {e}")))
}

pub(crate) fn run(script: &str, rpc: &str, out: &Path) -> ! {
    let dir = or_fail(artifact_dir(Path::new("out"), script));
    if !or_fail(script_has_queue_safe(&dir)) {
        println!("{script} has no queueSafe(); no Safe queue");
        std::process::exit(0);
    }
    let forge = or_fail(run_tool(
        "forge",
        &[
            "script",
            script,
            "--sig",
            "queueSafe()",
            "--rpc-url",
            rpc,
            "--json",
        ],
    ));
    let safe = or_fail(parse_forge_returns(&forge));
    let chain_id = or_fail(
        run_tool("cast", &["chain-id", "--rpc-url", rpc]).and_then(|s| parse_cast_uint(&s)),
    );
    let nonce = or_fail(
        run_tool(
            "cast",
            &["call", &safe, "nonce()(uint256)", "--rpc-url", rpc],
        )
        .and_then(|s| parse_cast_uint(&s)),
    );
    let service = or_fail(curl_json(&config_url(chain_id)).and_then(|c| transaction_service(&c)));
    let queue =
        or_fail(curl_json(&queue_url(&service, &safe, nonce)).and_then(|p| build_queue(&p, nonce)));
    let text = or_fail(serde_json::to_string_pretty(&queue).map_err(|e| e.to_string())) + "\n";
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        or_fail(std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display())));
    }
    or_fail(std::fs::write(out, &text).map_err(|e| format!("{}: {e}", out.display())));
    println!("Safe {safe} on chain {chain_id} at nonce {nonce}; queued:");
    print!("{text}");
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-safe-queue-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const SAFE: &str = "0x3840aeDaEc8e82f79d8F6a8F6ADCa271E13E0329";

    fn tx(nonce: u64, data: Value) -> Value {
        json!({
            "to": format!("0x{:040x}", nonce),
            "value": "0",
            "data": data,
            "operation": 1,
            "nonce": nonce,
            "safeTxHash": format!("0x{:064x}", nonce),
            "confirmations": [],
        })
    }

    fn page(results: Vec<Value>) -> Value {
        json!({ "count": results.len(), "next": null, "previous": null, "results": results })
    }

    #[test]
    fn only_a_queue_safe_function_opts_in() {
        let with = |entry: Value| json!({ "abi": [{"type": "function", "name": "run"}, entry] });
        assert!(abi_has_queue_safe(&with(
            json!({"type": "function", "name": "queueSafe"})
        )));
        assert!(!abi_has_queue_safe(&with(
            json!({"type": "event", "name": "queueSafe"})
        )));
        assert!(!abi_has_queue_safe(&with(
            json!({"type": "error", "name": "queueSafe"})
        )));
        assert!(!abi_has_queue_safe(&with(
            json!({"type": "function", "name": "queueSafeX"})
        )));
        assert!(!abi_has_queue_safe(&json!({ "bytecode": {} })));
    }

    #[test]
    fn artifacts_live_under_the_script_file_name() {
        assert_eq!(
            artifact_dir(Path::new("out"), "script/20261123-x.s.sol").unwrap(),
            Path::new("out").join("20261123-x.s.sol")
        );
        assert!(artifact_dir(Path::new("out"), "script/..").is_err());
    }

    #[test]
    fn any_artifact_in_the_dir_opts_the_script_in() {
        let d = tmp_dir();
        std::fs::write(d.join("Lib.json"), json!({"abi": []}).to_string()).unwrap();
        std::fs::write(d.join("notes.txt"), "not json").unwrap();
        assert!(!script_has_queue_safe(&d).unwrap());
        std::fs::write(
            d.join("Script.json"),
            json!({"abi": [{"type": "function", "name": "queueSafe"}]}).to_string(),
        )
        .unwrap();
        assert!(script_has_queue_safe(&d).unwrap());
    }

    #[test]
    fn a_missing_dir_or_bad_artifact_is_an_error() {
        let d = tmp_dir();
        assert!(script_has_queue_safe(&d.join("absent")).is_err());
        std::fs::write(d.join("Bad.json"), "{").unwrap();
        assert!(script_has_queue_safe(&d).is_err());
    }

    #[test]
    fn the_safe_comes_from_the_returns_line() {
        let out = format!(
            "Compiling 1 files\n{{\"logs\":[],\"returns\":null}}\n\
             {{\"logs\":[],\"returns\":{{\"0\":{{\"internal_type\":\"address\",\"value\":\"{SAFE}\"}}}}}}\n"
        );
        assert_eq!(parse_forge_returns(&out).unwrap(), SAFE);
    }

    #[test]
    fn forge_output_without_an_address_is_an_error() {
        assert!(parse_forge_returns("{\"returns\":null}\n").is_err());
        assert!(parse_forge_returns("{\"returns\":{\"0\":{\"value\":\"0x12\"}}}").is_err());
        assert!(parse_forge_returns("{\"returns\":{\"0\":{\"value\":true}}}").is_err());
        assert!(parse_forge_returns("").is_err());
    }

    #[test]
    fn cast_uints_parse_with_or_without_the_exponent_form() {
        assert_eq!(parse_cast_uint("12\n").unwrap(), 12);
        assert_eq!(parse_cast_uint("16661 [1.666e4]\n").unwrap(), 16661);
        assert!(parse_cast_uint("").is_err());
        assert!(parse_cast_uint("0x0c").is_err());
    }

    #[test]
    fn the_transaction_service_loses_its_trailing_slash() {
        let service = |v: Value| transaction_service(&json!({ "transactionService": v }));
        assert_eq!(
            service(json!("https://safe-transaction-mainnet.safe.global/")).unwrap(),
            "https://safe-transaction-mainnet.safe.global"
        );
        assert!(service(json!("")).is_err());
        assert!(service(Value::Null).is_err());
        assert_eq!(
            queue_url("https://s", SAFE, 12),
            format!(
                "https://s/api/v1/safes/{SAFE}/multisig-transactions/\
                 ?executed=false&nonce__gte=12&ordering=nonce&limit=100"
            )
        );
    }

    #[test]
    fn the_queue_is_written_as_parallel_arrays_in_nonce_order() {
        let q = build_queue(
            &page(vec![tx(12, json!("0xabcd")), tx(13, Value::Null)]),
            12,
        )
        .unwrap();
        assert_eq!(
            q,
            json!({
                "to": [format!("0x{:040x}", 12), format!("0x{:040x}", 13)],
                "value": ["0", "0"],
                "data": ["0xabcd", "0x"],
                "operation": [1, 1],
                "nonce": [12, 13],
                "safeTxHash": [format!("0x{:064x}", 12), format!("0x{:064x}", 13)],
            })
        );
    }

    #[test]
    fn an_empty_queue_is_empty_arrays() {
        assert_eq!(
            build_queue(&page(vec![]), 6).unwrap(),
            json!({"to": [], "value": [], "data": [], "operation": [], "nonce": [], "safeTxHash": []})
        );
    }

    #[test]
    fn a_queue_that_is_not_the_whole_queue_is_refused() {
        let mut more = page(vec![tx(12, Value::Null)]);
        more["next"] = json!("https://s/next");
        assert!(build_queue(&more, 12).is_err());
        assert!(build_queue(&page(vec![tx(12, Value::Null), tx(12, Value::Null)]), 12).is_err());
        assert!(build_queue(&page(vec![tx(12, Value::Null), tx(14, Value::Null)]), 12).is_err());
        assert!(build_queue(&page(vec![tx(13, Value::Null)]), 12).is_err());
        assert!(build_queue(&json!({"next": null}), 12).is_err());
    }

    #[test]
    fn a_queued_transaction_missing_a_field_is_refused() {
        for key in ["to", "value", "operation", "nonce", "safeTxHash"] {
            let mut t = tx(12, Value::Null);
            t[key] = Value::Null;
            assert!(build_queue(&page(vec![t]), 12).is_err(), "{key}");
        }
    }
}
