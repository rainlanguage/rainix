// RPC preflight: pick a working fork endpoint per network before forge runs.
//
// Foundry maps one alias to exactly one URL in [rpc_endpoints] and its
// --fork-retries only retries that same URL, so a deterministic upstream
// failure (plan quota exhausted, node is not archive, host is gone) cannot be
// recovered inside forge. This subcommand does the recovery one layer up: it
// builds a merged pool of candidate URLs per network, probes each one the way
// forge's fork backend actually reads a chain, and exports the first healthy
// one as <NETWORK>_RPC_URL for the steps that follow.
//
// MERGED POOL, NOT A PRECEDENCE CHAIN. `RPC_URL_<NET>_FORK` as a secret and as
// a variable each hold a LIST of URLs; the candidate pool is the CONCATENATION
// of both lists plus the hardcoded public archive defaults below. A URL in the
// variable is a real candidate even when the secret is also set — a non-empty
// earlier source never causes a later one to be skipped, it only orders them.
// Keyed/paid URLs belong in the secret (masked), public keyless URLs in the
// variable (visible, greppable, no masking noise); merging is what lets a
// public archive endpoint back up a keyed one without putting a non-secret in
// a secret. Order is secret entries, then variable entries, then defaults:
// prefer the paid/dedicated endpoint, fall back to the org's curated public
// list, and land on the hardcoded safety net only when both are exhausted.
//
// NO URL IS EVER PRINTED. See `Url` and `Reason` — the redaction is structural,
// not a scrubbing pass that a later edit can bypass.

use std::collections::BTreeSet;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::fail;

/// A candidate RPC URL.
///
/// This type is the no-leak guarantee. It has no `Display`, and its `Debug`
/// prints `<redacted>`, so no format string anywhere in this crate can put a
/// URL on stdout or stderr — including a future edit that adds a `{:?}` to a
/// log line. The inner string is reachable only through `expose()`, which has
/// exactly three call sites: the argv of the curl child process (whose stdout
/// and stderr are captured and never re-printed), the write to the file named
/// by --github-env, and the `::add-mask::` workflow command — whose whole
/// purpose is that the runner redacts the value it is given, including in the
/// command line itself. Every log line is built from a `Source` label and a
/// `Reason`, both of which are closed enums over fixed text.
pub(crate) struct Url(String);

impl fmt::Debug for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Url {
    fn expose(&self) -> &str {
        &self.0
    }
}

/// Where a candidate came from. This — never the URL — is what gets logged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Source {
    Secret(usize),
    Variable(usize),
    Default(usize),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Secret(i) => write!(f, "secret[{i}]"),
            Source::Variable(i) => write!(f, "variable[{i}]"),
            Source::Default(i) => write!(f, "default[{i}]"),
        }
    }
}

impl Source {
    /// True for candidates that came from a GitHub secret, i.e. the ones whose
    /// URL may embed an API key and must be masked before it reaches GITHUB_ENV.
    fn is_secret(&self) -> bool {
        matches!(self, Source::Secret(_))
    }
}

/// Why a candidate was rejected.
///
/// A closed enum, and its `Display` is built only from fixed strings plus
/// integers we produced ourselves (a JSON-RPC error code, an HTTP status, a
/// curl exit code, a chain id, a block number). No bytes from the network and
/// no part of a URL can reach a log line through it — an upstream that echoes
/// the request URL back inside an error message or an HTML error page cannot
/// leak it, because the message body is classified and then dropped.
///
/// Callers branch on the variant, never on message text: classification happens
/// once, here, and everything downstream is a typed discriminant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Reason {
    /// curl could not complete the request at all (DNS, TLS, connect, timeout).
    Unreachable { curl_exit: i32 },
    /// HTTP error with a body we could not classify as a JSON-RPC error.
    Http { status: u32 },
    /// Response was not JSON-RPC, or the result had the wrong shape.
    BadResponse,
    /// Right host, wrong chain — a misconfigured candidate that would silently
    /// fork the wrong network if selected.
    WrongChain { expected: u64, got: u64 },
    /// Plan/usage quota or rate limit. The failure that started all this.
    Quota { code: i64 },
    /// Node cannot serve state at the probe block: pruning node, or archive
    /// access gated behind a token we do not have.
    NotArchive { code: i64, block: u64 },
    /// Endpoint answers some methods but not the ones forge needs.
    MethodUnsupported { code: i64 },
    /// Authentication/authorisation rejected.
    Auth { code: i64 },
    /// A JSON-RPC error we did not recognise; the code is the discriminant.
    RpcError { code: i64 },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Unreachable { curl_exit } => {
                write!(f, "unreachable (curl exit {curl_exit})")
            }
            Reason::Http { status } => write!(f, "http {status}"),
            Reason::BadResponse => f.write_str("malformed or unexpected JSON-RPC response"),
            Reason::WrongChain { expected, got } => {
                write!(
                    f,
                    "wrong chain (expected {expected}, endpoint reports {got})"
                )
            }
            Reason::Quota { code } => {
                write!(f, "quota exhausted / rate limited (rpc error {code})")
            }
            Reason::NotArchive { code, block } => write!(
                f,
                "not archive: no historical state at block {block} (rpc error {code})"
            ),
            Reason::MethodUnsupported { code } => {
                write!(f, "required method not supported (rpc error {code})")
            }
            Reason::Auth { code } => write!(f, "unauthorized / key required (rpc error {code})"),
            Reason::RpcError { code } => write!(f, "rpc error {code}"),
        }
    }
}

/// One network's identity, probe calibration and hardcoded public fallbacks.
pub(crate) struct Network {
    /// foundry.toml [rpc_endpoints] alias.
    pub key: &'static str,
    /// Env var foundry.toml interpolates, e.g. `${ARBITRUM_RPC_URL}`.
    pub env_name: &'static str,
    /// Org secret/variable name holding this network's candidate list.
    pub secret_name: &'static str,
    pub chain_id: u64,
    /// Blocks the probe must be able to read account state at. Empty means the
    /// network is only ever forked at latest, so a pruning node is acceptable.
    ///
    /// Each entry is at or below the OLDEST block any repo in the org pins for
    /// this network, because the archive question is not "is this node an
    /// archive node" but "can it serve MY fork block". Probing shallower than
    /// the deepest live pin qualifies a node that then dies mid-suite.
    pub archive_blocks: &'static [u64],
    /// Contract with code at every `archive_blocks` entry, used for the
    /// historical `eth_call`. `totalSupply()` on the canonical wrapped native
    /// token: deployed at or near genesis on every chain here, so one address
    /// covers the whole history.
    pub probe_contract: &'static str,
    /// Hardcoded public keyless archive endpoints, the terminal element of the
    /// pool. Measured, not taken from a public RPC list: every entry answered
    /// the historical probe below 5/5 times. Providers that scored partially
    /// are deliberately absent — a load balancer over a heterogeneous backend
    /// pool can pass a preflight and still fail the suite.
    pub defaults: &'static [&'static str],
}

/// The org's networks. One table, so adding a network is a one-site edit rather
/// than the six-site copy-paste the workflow env blocks used to be.
pub(crate) const NETWORKS: &[Network] = &[
    Network {
        key: "arbitrum",
        env_name: "ARBITRUM_RPC_URL",
        secret_name: "RPC_URL_ARBITRUM_FORK",
        chain_id: 42161,
        // rain.tofu.erc20-decimals pins 280_000_000, the deepest live Arbitrum
        // pin in the org (~20 months back).
        archive_blocks: &[280_000_000],
        probe_contract: "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1", // WETH
        defaults: &[
            "https://arb-pokt.nodies.app",
            "https://42161.rpc.thirdweb.com",
            "https://arbitrum.gateway.tenderly.co",
        ],
    },
    Network {
        key: "base",
        env_name: "BASE_RPC_URL",
        secret_name: "RPC_URL_BASE_FORK",
        chain_id: 8453,
        // rain.deploy's findDeployBlock binary-searches from block 0, so Base
        // needs full archive back to genesis; block 1 is the strict form of
        // that (clients often special-case genesis). The second probe block is
        // mid-history, so a node holding only early state cannot pass either.
        archive_blocks: &[1, 39_000_000],
        probe_contract: "0x4200000000000000000000000000000000000006", // WETH predeploy
        defaults: &[
            "https://mainnet.base.org",
            "https://base-pokt.nodies.app",
            "https://base.gateway.tenderly.co",
            "https://base.drpc.org",
        ],
    },
    Network {
        key: "base_sepolia",
        env_name: "BASE_SEPOLIA_RPC_URL",
        secret_name: "RPC_URL_BASE_SEPOLIA_FORK",
        chain_id: 84532,
        // rain.metadata rolls to METABOARD_START_BLOCK_BASE_SEPOLIA 38_683_088.
        archive_blocks: &[38_000_000],
        probe_contract: "0x4200000000000000000000000000000000000006", // WETH predeploy
        defaults: &[
            "https://sepolia.base.org",
            "https://base-sepolia.gateway.tenderly.co",
            "https://84532.rpc.thirdweb.com",
            "https://base-sepolia.drpc.org",
        ],
    },
    Network {
        key: "flare",
        env_name: "FLARE_RPC_URL",
        secret_name: "RPC_URL_FLARE_FORK",
        chain_id: 14,
        // rain.flare forks at 31_843_105 in ~19 test files (~23 months back) —
        // the deepest historical read in the org outside rain.deploy's genesis
        // search, and Flare public nodes are the most prune-happy.
        archive_blocks: &[31_000_000],
        probe_contract: "0x1D80c49BbBCd1C0911346656B529DF9E5c2F783d", // WNAT
        defaults: &[
            "https://flare-api.flare.network/ext/C/rpc",
            "https://flare.rpc.thirdweb.com",
            "https://flare.gateway.tenderly.co",
            "https://flare.public-rpc.com",
        ],
    },
    Network {
        key: "polygon",
        env_name: "POLYGON_RPC_URL",
        secret_name: "RPC_URL_POLYGON_FORK",
        chain_id: 137,
        // rain.metadata rolls to METABOARD_START_BLOCK_POLYGON 82_855_948.
        archive_blocks: &[82_000_000],
        probe_contract: "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270", // WMATIC
        defaults: &[
            "https://polygon.gateway.tenderly.co",
            "https://137.rpc.thirdweb.com",
            "https://polygon.drpc.org",
        ],
    },
    Network {
        key: "ethereum",
        env_name: "ETHEREUM_RPC_URL",
        secret_name: "RPC_URL_ETHEREUM_FORK",
        chain_id: 1,
        // Every Ethereum fork in the org is at latest — no repo pins a block
        // and no isStartBlock/findDeployBlock is wired to an Ethereum endpoint.
        // A pruning node is sufficient, so do not exclude one.
        archive_blocks: &[],
        probe_contract: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2", // WETH
        // eth-pokt.nodies.app is demoted to LAST per rainlanguage/rainix#340:
        // from GitHub runners it answers the light probe and then returns
        // `408 {"message":"Request timeout on the free plan..."}` under real
        // fork traffic, which took out a rain.factory.deploy dispatch and a
        // rain.metadata test run.
        //
        // THE TWO VANTAGE POINTS DISAGREE, so read the order as a preference
        // rather than a ranking. Sustained-load measurement from one non-CI
        // host on 2026-08-20 (60s rest, then four consecutive bursts of 16)
        // says almost the opposite of the CI evidence above:
        //
        //   eth.drpc.org                 16/16 16/16 16/16 16/16
        //   eth-pokt.nodies.app          16/16 16/16 16/16 16/16
        //   mainnet.gateway.tenderly.co  15/16  0/16 14/16  1/16
        //
        // Public rate limits are per-IP, so a measurement from here does not
        // predict a GitHub runner and vice versa — neither source is wrong and
        // no static order can be right for both. drpc leads because it is the
        // only endpoint with no evidence against it from EITHER vantage;
        // tenderly is kept ahead of pokt because #340's demotion of pokt rests
        // on the environment that actually matters (CI), even though it looks
        // best from here. What makes this safe either way is the burst check in
        // `probe`: whichever of these is throttled for the runner running right
        // now is rejected and failed over. Reorder on CI evidence, not on a
        // local measurement.
        defaults: &[
            "https://eth.drpc.org",
            "https://mainnet.gateway.tenderly.co",
            "https://eth-pokt.nodies.app",
        ],
    },
    Network {
        key: "sepolia",
        env_name: "SEPOLIA_RPC_URL",
        secret_name: "RPC_URL_SEPOLIA_FORK",
        chain_id: 11155111,
        // The `RPC_URL_SEPOLIA_FORK` org secret existed with nothing consuming
        // it (rainlanguage/rainix#340): no entry here meant it was never
        // demand-scanned, never probed and never exported. Modelling it is what
        // makes the secret reachable.
        //
        // Latest-only: no repo in the org forks Sepolia at a pinned block,
        // precisely because the secret has never been consumable, so there is
        // no live pin to protect and no reason to reject a pruning node. Per
        // the rule on `archive_blocks`, add the deepest pin here the moment a
        // consumer appears — probing shallower than a live pin is the failure
        // this field exists to prevent.
        archive_blocks: &[],
        probe_contract: "0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14", // WETH9
        // Measured 2026-08-20, same method as the rest of the table: 5/5 on the
        // sequential probe, then a 16-way burst. publicnode and thirdweb each
        // went 5/5 and 16/16; tenderly went 5/5 but shed 3/16 to `-32005 rate
        // limit exceeded`, so it is ordered last of the three.
        //
        // Deliberately absent, all measured the same day and all rejected:
        // 1rpc.io/sepolia scored 4/5 and then 8/16 (`cu limit exceeded`, served
        // with HTTP 200); sepolia.drpc.org refuses 100% behind a free-plan gate
        // (`code 35`); rpc.sepolia.org is gone (404); eth-sepolia.public.
        // blastapi.io is discontinued (403); endpoints.omniatech.io 521s.
        defaults: &[
            "https://ethereum-sepolia-rpc.publicnode.com",
            "https://11155111.rpc.thirdweb.com",
            "https://sepolia.gateway.tenderly.co",
        ],
    },
    Network {
        key: "hyperevm",
        env_name: "HYPEREVM_RPC_URL",
        secret_name: "RPC_URL_HYPEREVM_FORK",
        chain_id: 999,
        // Latest-only in every consumer today.
        archive_blocks: &[],
        probe_contract: "0x5555555555555555555555555555555555555555", // WHYPE
        defaults: &[
            "https://rpc.hyperliquid.xyz/evm",
            "https://rpc.hyperlend.finance",
            "https://hyperliquid.drpc.org",
        ],
    },
    Network {
        key: "robinhood",
        env_name: "ROBINHOOD_RPC_URL",
        secret_name: "RPC_URL_ROBINHOOD_FORK",
        chain_id: 4663,
        // Robinhood Chain is an Arbitrum Orbit L2 settling to Ethereum.
        // Latest-only in every consumer today (st0x.deploy forks it at head
        // for its prod-state and cross-chain parity pins); add the deepest
        // pin here the moment a consumer forks it at a block.
        archive_blocks: &[],
        probe_contract: "0x0Bd7D308f8E1639FAb988df18A8011f41EAcAD73", // L2 WETH
        // Measured 2026-09-09, same method as the rest of the table: 5/5 on
        // the sequential probe, then a 16-way burst, from one non-CI host.
        // The official public endpoint (rate-limited per the chain docs),
        // tenderly and publicnode each went 5/5 and 16/16. Rejected the same
        // day: robinhood.drpc.org answers `eth_chainId` and then refuses
        // every `eth_call` (0/5) behind its plan gate; 4663.rpc.thirdweb.com
        // reports `-32001 Invalid chain`.
        defaults: &[
            "https://rpc.mainnet.chain.robinhood.com",
            "https://robinhood-chain.gateway.tenderly.co",
            "https://robinhood-rpc.publicnode.com",
        ],
    },
];

/// Split a secret/variable value into candidate URLs.
///
/// Newline is the separator, and it is the only one that works: it cannot occur
/// inside a URL so it needs no escaping, GitHub's secret and variable editors
/// accept multi-line values natively, and the runner registers each LINE of a
/// multi-line secret as a separately masked value. A comma would be ambiguous
/// (legal in a query string) and a space likewise. `#` starts a comment so a
/// candidate can be parked with a note about why. A single bare URL is a
/// one-element list, which is exactly what these secrets hold today.
fn parse_list(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Build the merged candidate pool for one network: secret list, then variable
/// list, then hardcoded defaults. Duplicates are dropped keeping the earliest
/// occurrence, so listing the same URL in both the secret and the variable
/// costs one probe, not two.
fn pool(net: &Network, secret_raw: &str, vars_raw: &str) -> Vec<(Source, Url)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let push = |src: Source, u: String, seen: &mut BTreeSet<String>, out: &mut Vec<_>| {
        if seen.insert(u.clone()) {
            out.push((src, Url(u)));
        }
    };
    for (i, u) in parse_list(secret_raw).into_iter().enumerate() {
        push(Source::Secret(i), u, &mut seen, &mut out);
    }
    for (i, u) in parse_list(vars_raw).into_iter().enumerate() {
        push(Source::Variable(i), u, &mut seen, &mut out);
    }
    for (i, u) in net.defaults.iter().enumerate() {
        push(Source::Default(i), u.to_string(), &mut seen, &mut out);
    }
    out
}

/// Classify a JSON-RPC error into a typed reason.
///
/// Providers disagree wildly on codes for the same condition — a pruning node
/// is `-32000 "missing trie node"` on one host, `-32000 "state ... is pruned"`
/// on another and `-32602 "Archive requests require a personal token"` on a
/// third — so the code alone cannot discriminate. Message keywords are matched
/// HERE, once, and the message is then discarded: nothing downstream ever sees
/// it, and no decision anywhere is made by substring-matching error text.
fn classify(code: i64, message: &str, block: Option<u64>) -> Reason {
    let m = message.to_ascii_lowercase();
    let has = |needle: &str| m.contains(needle);

    // Plan/quota FIRST, and specifically before the archive arm below: the
    // free-plan gate reads "chain is not available on free plan", which the
    // archive arm's "is not available" needle also matches. Whichever arm runs
    // first wins, so a billing wall would otherwise be reported as a pruning
    // node — the wrong actionable fact, and one that sends a reader looking for
    // a deeper archive endpoint instead of a paid key.
    //
    // "free plan" / "paid plan" / "upgrade to" are the wording the throttling
    // providers in this table actually use; none of them says "quota" or "rate
    // limit" when the refusal is plan-based rather than burst-based.
    if code == -32001
        || has("usage limit")
        || has("current plan")
        || has("free plan")
        || has("paid plan")
        || has("upgrade to")
        || has("quota")
        || has("rate limit")
        || has("too many requests")
        || has("exceeded")
    {
        return Reason::Quota { code };
    }
    // Before the auth arm: "Archive requests require a personal token" is an
    // archive refusal first and an auth error second, and the archive framing
    // is the actionable one (move the network to a candidate that serves it).
    if has("missing trie node")
        || has("pruned")
        || has("archive")
        || has("is not available")
        || has("historical state")
        || has("state at block")
        || has("older block")
        || has("no state")
        || has("state not found")
        || has("header not found")
        || has("block not found")
    {
        return Reason::NotArchive {
            code,
            block: block.unwrap_or(0),
        };
    }
    if code == -32601 || has("not supported") || has("method not found") || has("unsupported") {
        return Reason::MethodUnsupported { code };
    }
    if has("unauthorized")
        || has("forbidden")
        || has("api key")
        || has("apikey")
        || has("must be authenticated")
        || has("invalid key")
    {
        return Reason::Auth { code };
    }
    Reason::RpcError { code }
}

/// Pull an error `(code, message)` out of a response body, whatever shape the
/// provider chose to send it in.
///
/// Three shapes occur among the endpoints in this table, all captured live:
///
///   * `{"error":{"code":-32005,"message":"rate limit exceeded"}}` — the
///     JSON-RPC envelope the spec asks for (tenderly, HTTP 429).
///   * `{"error":"cu limit exceeded; ..."}` — `error` as a bare STRING rather
///     than an object, served with **HTTP 200** (1rpc.io). This is the shape
///     that matters most: with no envelope to read and a success status, an
///     unhandled body here is indistinguishable from a healthy response, so
///     the candidate is SELECTED and the suite dies later.
///   * `{"message":"Request timeout on the free plan...","code":30}` — no
///     envelope at all, the fields sit at the top level (drpc, HTTP 408).
///
/// A top-level `message`/`code` pair is only an error when there is no
/// `result` beside it, so a healthy response that happens to carry a `message`
/// field is never misread as a failure.
fn error_fields(json: &serde_json::Value) -> Option<(i64, &str)> {
    // An explicit `"error": null` is NOT an error: serde_json returns
    // `Some(Value::Null)` for it, and providers echo it beside a healthy
    // `result`. Filtering it out here keeps such a response a success.
    if let Some(err) = json.get("error").filter(|err| !err.is_null()) {
        // `error` as a bare string carries no code of its own; 0 is the
        // "no code supplied" discriminant, exactly as for a missing field.
        if let Some(message) = err.as_str() {
            return Some((0, message));
        }
        let code = err
            .get("code")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let message = err
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        return Some((code, message));
    }
    if json.get("result").is_none() {
        if let Some(message) = json.get("message").and_then(serde_json::Value::as_str) {
            let code = json
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            return Some((code, message));
        }
    }
    None
}

/// Reject on an HTTP status alone, when the body told us nothing.
///
/// 429 is a rate limit by definition, so it is a quota rejection even when the
/// body is an HTML error page we refuse to parse — reporting it as a bare
/// status would hide the one failure class this whole subcommand exists to
/// route around.
fn http_reason(status: u32) -> Reason {
    match status {
        429 => Reason::Quota { code: 0 },
        s if s >= 400 => Reason::Http { status: s },
        _ => Reason::BadResponse,
    }
}

/// POST one JSON-RPC request and return its `result`.
///
/// curl's stdout is captured and parsed; its stderr is captured and DROPPED
/// without being read, because curl writes the request host into its own error
/// text. `-s` also keeps the progress meter off stdout.
fn rpc(
    url: &Url,
    timeout: u32,
    body: &str,
    block: Option<u64>,
) -> Result<serde_json::Value, Reason> {
    finish_rpc(spawn_rpc(url, timeout, body)?, block)
}

/// Spawn one curl child with the request body already written to its stdin.
///
/// Split out from `rpc` so a caller can start MANY requests before waiting on
/// any of them: spawning and collecting in one step serialises the probe, and a
/// serial probe cannot generate the request RATE that plan throttling responds
/// to. See `burst`.
fn spawn_rpc(url: &Url, timeout: u32, body: &str) -> Result<std::process::Child, Reason> {
    let mut child = Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "-m",
            &timeout.to_string(),
            "--data-binary",
            "@-",
            "-w",
            "\n%{http_code}",
            url.expose(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Reason::Unreachable { curl_exit: -1 })?;
    child
        .stdin
        .as_mut()
        .and_then(|s| s.write_all(body.as_bytes()).ok())
        .ok_or(Reason::Unreachable { curl_exit: -1 })?;
    Ok(child)
}

/// Wait for a spawned curl child and parse its response.
fn finish_rpc(child: std::process::Child, block: Option<u64>) -> Result<serde_json::Value, Reason> {
    let out = child
        .wait_with_output()
        .map_err(|_| Reason::Unreachable { curl_exit: -1 })?;
    if !out.status.success() {
        return Err(Reason::Unreachable {
            curl_exit: out.status.code().unwrap_or(-1),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body_text, status_text) = text.rsplit_once('\n').unwrap_or(("", ""));
    let status: u32 = status_text.trim().parse().unwrap_or(0);

    let json: serde_json::Value = match serde_json::from_str(body_text) {
        Ok(v) => v,
        // A non-JSON body (an HTML error page, a proxy banner) is never parsed
        // for meaning and never printed; only the status survives.
        Err(_) => return Err(http_reason(status)),
    };
    if let Some((code, message)) = error_fields(&json) {
        return Err(classify(code, message, block));
    }
    if status >= 400 {
        return Err(http_reason(status));
    }
    json.get("result").cloned().ok_or(Reason::BadResponse)
}

fn hex_result(v: &serde_json::Value) -> Option<&str> {
    v.as_str().filter(|s| s.starts_with("0x"))
}

/// Chain id from an `eth_chainId` result.
fn parse_chain_id(v: &serde_json::Value) -> Option<u64> {
    hex_result(v).and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
}

/// Blocks the probe reads state at.
///
/// Latest-only when the network has no pinned historical reads in the org, or
/// when the caller is a deploy/broadcast path that only ever touches head state
/// — forcing archive there would reject a perfectly good pruning endpoint for
/// no benefit.
fn probe_blocks(net: &Network, archive: bool) -> Vec<Option<u64>> {
    if archive && !net.archive_blocks.is_empty() {
        net.archive_blocks.iter().copied().map(Some).collect()
    } else {
        vec![None]
    }
}

/// JSON-RPC block parameter for a probe block.
fn block_tag(b: Option<u64>) -> String {
    match b {
        Some(n) => format!("0x{n:x}"),
        None => "latest".to_string(),
    }
}

/// Validate an `eth_call` of `totalSupply()`.
///
/// A well-formed answer is exactly one 32-byte word ("0x" + 64 hex digits).
/// `0x` means the call executed against an account with no code, i.e. the node
/// served an empty state for the probe block rather than erroring — a silent
/// form of "not archive" that would otherwise look like a pass.
fn check_call_result(v: &serde_json::Value, block: Option<u64>) -> Result<(), Reason> {
    match hex_result(v) {
        Some(s) if s.len() == 66 => Ok(()),
        Some(_) => Err(Reason::NotArchive {
            code: 0,
            block: block.unwrap_or(0),
        }),
        None => Err(Reason::BadResponse),
    }
}

/// Fire `n` identical requests SIMULTANEOUSLY and report each outcome
/// (`None` = the request succeeded).
///
/// Every request is spawned before any is collected — that is the entire point.
/// The sequential checks above issue a handful of calls with a full round trip
/// of think time between them, which is a request RATE no plan throttle reacts
/// to; that is why an endpoint can pass the preflight and then die under forge,
/// which opens many concurrent state reads. This reproduces the rate, not just
/// the calls.
///
/// `latest` and the cheapest available call, deliberately: the question here is
/// "how many requests per second will you serve", not "how deep is your
/// history" — that is already settled above — and multiplying archive reads by
/// `n` would be a heavy and pointless load on every provider in the table.
fn burst(url: &Url, net: &Network, n: u32, timeout: u32) -> Vec<Option<Reason>> {
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{{"to":"{}","data":"0x18160ddd"}},"latest"]}}"#,
        net.probe_contract
    );
    let spawned: Vec<Result<std::process::Child, Reason>> =
        (0..n).map(|_| spawn_rpc(url, timeout, &body)).collect();
    spawned
        .into_iter()
        .map(|c| match c {
            Ok(child) => match finish_rpc(child, None) {
                Ok(v) => check_call_result(&v, None).err(),
                Err(reason) => Some(reason),
            },
            Err(reason) => Some(reason),
        })
        .collect()
}

/// Verdict on a load burst: `Some(reason)` rejects the candidate.
///
/// The burst answers exactly ONE question — does this endpoint throttle at the
/// request rate a fork suite generates — so it rejects for throttling and for
/// nothing else. Correctness (chain id, archive depth, response shape) is
/// already settled by the sequential phase, and a lone transient timeout inside
/// a 16-way burst is not evidence of an unhealthy endpoint. Treating any single
/// failure as fatal would make this preflight flakier than the outage it exists
/// to prevent, and would reject endpoints the org depends on.
///
/// Hence a MAJORITY threshold rather than "any". Measured against the very
/// candidates in this table (2026-08-20, from one host — see the ethereum
/// `defaults` note on why one vantage point is not the whole story): a healthy
/// public endpoint sheds the occasional request under burst — eth.drpc.org
/// returned a single 429 in a burst of 8 and then served 32/32 twice — while a
/// throttled or plan-gated one fails nearly all of them —
/// mainnet.gateway.tenderly.co returned 9/16 and then 32/32 rate-limited, and
/// sepolia.drpc.org refuses 100% behind a free-plan gate. Half separates those
/// two populations with room to spare in both directions.
fn burst_verdict(outcomes: &[Option<Reason>]) -> Option<Reason> {
    let throttled: Vec<Reason> = outcomes
        .iter()
        .flatten()
        .copied()
        .filter(|r| matches!(r, Reason::Quota { .. }))
        .collect();
    if throttled.len() * 2 > outcomes.len() {
        throttled.first().copied()
    } else {
        None
    }
}

/// Probe one candidate. Returns Ok only if EVERY check passes on EVERY sample.
///
/// The three checks mirror what forge's fork backend does, in the order that
/// fails cheapest first:
///   1. eth_chainId  — a candidate for the wrong chain would silently fork the
///      wrong network, which is worse than a red job.
///   2. eth_getBalance at the probe block — historical ACCOUNT state, which is
///      what forge fetches first and what a pruning node refuses.
///   3. eth_call at the probe block — historical CONTRACT execution. Necessary
///      on top of (2) because some hosts serve historical code and state but
///      answer every eth_call with "method not supported"; a code-only probe
///      selects them and the suite dies later.
///   4. a simultaneous `burst_size` burst — LOAD. Checks 1-3 are correctness
///      questions, and an endpoint that is throttled rather than broken answers
///      all of them perfectly: all three ethereum defaults passed 1-3 on
///      2026-08-20 while two of them shed most of a concurrent burst. Without
///      this step the preflight cannot tell a healthy endpoint from one that
///      will 408 the moment forge opens real fork traffic, which is precisely
///      the failure rainlanguage/rainix#340 is about.
///
/// `samples` consecutive full passes are required. A single sample is not
/// enough: the public load balancers round-robin over a mix of archive and
/// pruning backends, so one lucky answer qualifies an endpoint that then fails
/// partway through a suite that issues thousands of historical reads.
fn probe(
    url: &Url,
    net: &Network,
    samples: u32,
    timeout: u32,
    archive: bool,
    burst_size: u32,
) -> Result<(), Reason> {
    let chain = rpc(
        url,
        timeout,
        r#"{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}"#,
        None,
    )?;
    let got = parse_chain_id(&chain).ok_or(Reason::BadResponse)?;
    if got != net.chain_id {
        return Err(Reason::WrongChain {
            expected: net.chain_id,
            got,
        });
    }

    let blocks = probe_blocks(net, archive);

    for _ in 0..samples {
        for b in &blocks {
            let tag = block_tag(*b);
            let bal = rpc(
                url,
                timeout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["{}","{}"]}}"#,
                    net.probe_contract, tag
                ),
                *b,
            )?;
            hex_result(&bal).ok_or(Reason::BadResponse)?;

            let call = rpc(
                url,
                timeout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{{"to":"{}","data":"0x18160ddd"}},"{}"]}}"#,
                    net.probe_contract, tag
                ),
                *b,
            )?;
            check_call_result(&call, *b)?;
        }
    }

    // Load last: it is the most expensive check and the only one that can be
    // skipped (--burst 0), so everything cheap and disqualifying runs first.
    //
    // `samples` ROUNDS, back to back, and any one round failing is fatal. One
    // round is not enough, and the reason is the shape of the limiter: these
    // endpoints meter a token bucket, so the first burst after an idle period
    // is served out of a full bucket and tells you nothing about sustained
    // load. Measured against mainnet.gateway.tenderly.co on 2026-08-20 —
    // 60s rest, then four consecutive bursts of 16 — the rounds throttled
    // 1/16, then 16/16, then 2/16, then 15/16: round one PASSES and rounds two
    // and four collapse completely. A fork suite is sustained load, not one
    // burst, so the probe has to be too. (eth.drpc.org served 16/16 in all four
    // rounds of the same run, so this does not simply reject everything.)
    for _ in 0..samples {
        if let Some(reason) = burst_verdict(&burst(url, net, burst_size, timeout)) {
            return Err(reason);
        }
    }
    Ok(())
}

/// True when `text` references the identifier `name` as a WHOLE WORD.
///
/// A plain `contains` was sufficient only while no network name nested inside
/// another. `sepolia` breaks that: `SEPOLIA_RPC_URL` is a substring of
/// `BASE_SEPOLIA_RPC_URL`, so a substring scan makes every repo that forks Base
/// Sepolia — most of the org — also look like it demands Ethereum Sepolia. A
/// falsely demanded network is probed, and a network with no healthy candidate
/// FAILS THE JOB, so the naive form would red repos that never touch Sepolia.
///
/// These names are identifiers, so the boundary rule is the identifier rule: a
/// match counts only when neither neighbouring byte is `[A-Za-z0-9_]`. That
/// keeps every real reference (`${SEPOLIA_RPC_URL}`, `vm.envString("…")`,
/// `SEPOLIA_RPC_URL=…`) and drops every nested one.
fn mentions(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    text.match_indices(name).any(|(i, _)| {
        let before = i == 0 || !ident(bytes[i - 1]);
        let end = i + name.len();
        let after = end >= bytes.len() || !ident(bytes[end]);
        before && after
    })
}

/// Networks the repo actually uses, by scanning tracked files for the env name
/// foundry.toml interpolates (`ARBITRUM_RPC_URL`) or the raw secret name
/// (`RPC_URL_ARBITRUM_FORK`, which some repos read straight from vm.envString).
///
/// This is what keeps the preflight from changing behaviour for a network a
/// repo does not touch: an unmentioned network is never probed, never exported
/// and can never fail the job.
fn demanded(root: &Path) -> BTreeSet<&'static str> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .unwrap_or_else(|e| fail(&format!("rpc-preflight: git ls-files failed to spawn: {e}")));
    if !out.status.success() {
        fail("rpc-preflight: git ls-files failed; run this from a git checkout");
    }
    let mut found = BTreeSet::new();
    for name in String::from_utf8_lossy(&out.stdout).split('\0') {
        if name.is_empty() {
            continue;
        }
        let path = root.join(name);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        // Skip anything too large to be a source file; nothing that references
        // an RPC env var is a multi-megabyte blob.
        if meta.len() > 2 * 1024 * 1024 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for net in NETWORKS {
            if mentions(&text, net.env_name) || mentions(&text, net.secret_name) {
                found.insert(net.key);
            }
        }
    }
    found
}

/// Append `<name>=<value>` to the GITHUB_ENV-style file.
///
/// The ONLY sink a URL reaches besides curl's argv. There is deliberately no
/// stdout mode: without a file to write to, the subcommand fails rather than
/// falling back to printing, so no invocation can ever put a URL on a log.
fn export(path: &Path, name: &str, url: &Url) {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap_or_else(|e| fail(&format!("rpc-preflight: open {}: {e}", path.display())));
    // A URL cannot contain a newline (parse_list splits on them), so the plain
    // KEY=value form is sufficient and no heredoc delimiter is needed.
    writeln!(f, "{}={}", name, url.expose())
        .unwrap_or_else(|e| fail(&format!("rpc-preflight: write {}: {e}", path.display())));
}

/// Run the preflight for every network the repo uses.
pub(crate) fn run(
    root: &Path,
    github_env: &Path,
    samples: u32,
    timeout: u32,
    archive: bool,
    burst_size: u32,
) {
    let want = demanded(root);
    if want.is_empty() {
        println!("rpc-preflight: repo references no fork RPC env vars; nothing to do");
        return;
    }

    let mut failed: Vec<&'static str> = Vec::new();
    for net in NETWORKS {
        if !want.contains(net.key) {
            continue;
        }
        let secret_raw =
            std::env::var(format!("RAINIX_RPC_SECRET_{}", upper(net.key))).unwrap_or_default();
        let vars_raw =
            std::env::var(format!("RAINIX_RPC_VARS_{}", upper(net.key))).unwrap_or_default();
        let candidates = pool(net, &secret_raw, &vars_raw);

        if candidates.is_empty() {
            // Exactly today's behaviour: the env var stays unset and forge
            // reports it if a test actually needs it. Nothing to fail over to
            // means nothing to fail about.
            println!(
                "rpc-preflight: {}: no candidates configured ({} / vars {}); leaving {} unset",
                net.key, net.secret_name, net.secret_name, net.env_name
            );
            continue;
        }

        let mode = if archive && !net.archive_blocks.is_empty() {
            format!("archive at block {}", net.archive_blocks[0])
        } else {
            "latest".to_string()
        };
        let mut rejected: Vec<(Source, Reason)> = Vec::new();
        let mut selected: Option<(Source, Url)> = None;
        for (src, url) in candidates {
            match probe(&url, net, samples, timeout, archive, burst_size) {
                Ok(()) => {
                    selected = Some((src, url));
                    break;
                }
                Err(reason) => rejected.push((src, reason)),
            }
        }

        for (src, reason) in &rejected {
            println!("rpc-preflight: {}: {src} rejected: {reason}", net.key);
        }

        match selected {
            Some((src, url)) => {
                if src.is_secret() {
                    // Defence in depth. The runner already masks each line of a
                    // multi-line secret, but a keyed URL that arrives through a
                    // variable by mistake is not masked, and this costs nothing.
                    // The command's own argument is redacted by the runner.
                    println!("::add-mask::{}", url.expose());
                }
                export(github_env, net.env_name, &url);
                // Also export under the raw secret name: a few repos read
                // vm.envString("RPC_URL_<NET>_FORK") directly, and once that
                // secret can hold a LIST those repos would otherwise receive a
                // multi-line string that createSelectFork cannot use.
                export(github_env, net.secret_name, &url);
                if !rejected.is_empty() {
                    println!(
                        "::warning::rpc-preflight: {} fell back to {src} after {} unhealthy candidate(s)",
                        net.key,
                        rejected.len()
                    );
                }
                println!(
                    "rpc-preflight: {}: SELECTED {src} (chain {}, {mode}, {samples}/{samples} samples)",
                    net.key, net.chain_id
                );
            }
            None => {
                eprintln!(
                    "::error::rpc-preflight: {}: no healthy endpoint among {} candidate(s) for chain {} ({mode}). \
                     Every candidate and why it was rejected is listed above by SOURCE; \
                     add a working URL to the {} secret (keyed) or variable (public, newline-separated).",
                    net.key,
                    rejected.len(),
                    net.chain_id,
                    net.secret_name
                );
                failed.push(net.key);
            }
        }
    }

    if !failed.is_empty() {
        fail(&format!(
            "rpc-preflight: no healthy RPC endpoint for: {}",
            failed.join(", ")
        ));
    }
    println!("rpc-preflight: clean");
}

fn upper(key: &str) -> String {
    key.to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_never_formats_its_value() {
        let u = Url("https://secret.example/KEY123".to_string());
        assert_eq!(format!("{u:?}"), "<redacted>");
        assert!(!format!("{u:?}").contains("KEY123"));
    }

    #[test]
    fn reason_text_is_url_free_and_typed() {
        // Every variant renders from fixed text plus integers we produced.
        for r in [
            Reason::Unreachable { curl_exit: 6 },
            Reason::Http { status: 521 },
            Reason::BadResponse,
            Reason::WrongChain {
                expected: 42161,
                got: 1,
            },
            Reason::Quota { code: -32001 },
            Reason::NotArchive {
                code: -32000,
                block: 280_000_000,
            },
            Reason::MethodUnsupported { code: -32601 },
            Reason::Auth { code: -32602 },
            Reason::RpcError { code: -32000 },
        ] {
            let s = r.to_string();
            assert!(!s.contains("http://"), "{s}");
            assert!(!s.contains("https://"), "{s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn list_parsing() {
        assert_eq!(parse_list(""), Vec::<String>::new());
        // A bare single URL is a one-element list — what the secrets hold today.
        assert_eq!(parse_list("https://a.example"), vec!["https://a.example"]);
        assert_eq!(
            parse_list("https://a.example\nhttps://b.example\n"),
            vec!["https://a.example", "https://b.example"]
        );
        // CRLF, blank lines, indentation and comments.
        assert_eq!(
            parse_list("  https://a.example  \r\n\n# dead, quota\n  https://b.example # keyed\n"),
            vec!["https://a.example", "https://b.example"]
        );
        assert_eq!(parse_list("# everything commented\n"), Vec::<String>::new());
    }

    fn net(key: &'static str) -> &'static Network {
        NETWORKS.iter().find(|n| n.key == key).unwrap()
    }

    #[test]
    fn pool_is_a_merge_not_a_precedence_chain() {
        let n = net("arbitrum");
        let p = pool(n, "https://keyed.example", "https://public.example");
        let srcs: Vec<String> = p.iter().map(|(s, _)| s.to_string()).collect();
        // The variable entry is present even though the secret was non-empty,
        // and the hardcoded defaults are present even though both were.
        assert_eq!(srcs[0], "secret[0]");
        assert_eq!(srcs[1], "variable[0]");
        assert_eq!(srcs[2], "default[0]");
        assert_eq!(p.len(), 2 + n.defaults.len());
    }

    #[test]
    fn pool_orders_secret_then_variable_then_default() {
        let n = net("polygon");
        let p = pool(n, "https://s1\nhttps://s2", "https://v1");
        let srcs: Vec<String> = p.iter().map(|(s, _)| s.to_string()).collect();
        assert_eq!(&srcs[..3], &["secret[0]", "secret[1]", "variable[0]"]);
        assert!(srcs[3..].iter().all(|s| s.starts_with("default[")));
    }

    #[test]
    fn pool_dedupes_across_sources_keeping_first() {
        let n = net("base");
        // Same URL in the secret and the variable, plus a default repeated.
        let p = pool(
            n,
            "https://same.example",
            "https://same.example\nhttps://mainnet.base.org",
        );
        let srcs: Vec<String> = p.iter().map(|(s, _)| s.to_string()).collect();
        assert_eq!(srcs[0], "secret[0]");
        // mainnet.base.org is default[0]; listed in the variable it stays at
        // the variable's position and is not probed twice.
        assert_eq!(srcs[1], "variable[1]");
        assert_eq!(p.len(), 1 + 1 + n.defaults.len() - 1);
    }

    #[test]
    fn pool_with_nothing_configured_is_just_the_defaults() {
        let n = net("flare");
        let p = pool(n, "", "");
        assert_eq!(p.len(), n.defaults.len());
        assert!(p.iter().all(|(s, _)| matches!(s, Source::Default(_))));
    }

    #[test]
    fn pool_is_empty_when_nothing_is_configured_and_there_is_no_default() {
        // Every network in the table currently ships defaults, so construct the
        // degenerate case explicitly: an empty pool must stay empty, because
        // `run` treats that as a skip (today's behaviour) rather than a failure.
        let n = Network {
            key: "nowhere",
            env_name: "NOWHERE_RPC_URL",
            secret_name: "RPC_URL_NOWHERE_FORK",
            chain_id: 0,
            archive_blocks: &[],
            probe_contract: "0x0000000000000000000000000000000000000000",
            defaults: &[],
        };
        assert!(pool(&n, "", "").is_empty());
        assert!(pool(&n, "  \n # only a comment\n", "").is_empty());
    }

    #[test]
    fn quota_is_classified_from_code_or_wording() {
        assert_eq!(
            classify(-32001, "anything", None),
            Reason::Quota { code: -32001 }
        );
        // The exact drpc body that caused the outage.
        assert_eq!(
            classify(
                -32001,
                "You've reached the usage limit for your current plan",
                None
            ),
            Reason::Quota { code: -32001 }
        );
        assert_eq!(
            classify(-32005, "rate limit exceeded", None),
            Reason::Quota { code: -32005 }
        );
    }

    #[test]
    fn pruning_nodes_are_classified_as_not_archive() {
        let b = Some(280_000_000);
        for msg in [
            "missing trie node 4ffbde5a is not available, not found",
            "state 0x3068d1 is not available",
            "state at block #39000001 is pruned",
            "historical state not available",
            "It looks like you're trying to fork from an older block with a non-archive node",
            // publicnode gates archive behind a token; the archive framing is
            // the actionable one, so it wins over the auth arm.
            "Archive requests require a personal token.",
        ] {
            assert_eq!(
                classify(-32000, msg, b),
                Reason::NotArchive {
                    code: -32000,
                    block: 280_000_000
                },
                "{msg}"
            );
        }
    }

    #[test]
    fn unsupported_method_and_auth_are_distinguished() {
        assert_eq!(
            classify(-32000, "The method eth_call is not supported.", None),
            Reason::MethodUnsupported { code: -32000 }
        );
        assert_eq!(
            classify(-32601, "whatever", None),
            Reason::MethodUnsupported { code: -32601 }
        );
        assert_eq!(
            classify(-32000, "Unauthorized", None),
            Reason::Auth { code: -32000 }
        );
        assert_eq!(
            classify(-32000, "api key required", None),
            Reason::Auth { code: -32000 }
        );
    }

    #[test]
    fn unrecognised_errors_keep_their_code_as_the_discriminant() {
        assert_eq!(
            classify(-32603, "internal error", None),
            Reason::RpcError { code: -32603 }
        );
    }

    #[test]
    fn chain_id_parsing() {
        assert_eq!(parse_chain_id(&json!("0xa4b1")), Some(42161));
        assert_eq!(parse_chain_id(&json!("0x1")), Some(1));
        assert_eq!(parse_chain_id(&json!("0xe")), Some(14));
        // Decimal, missing prefix, non-string, garbage hex: all unusable.
        assert_eq!(parse_chain_id(&json!("42161")), None);
        assert_eq!(parse_chain_id(&json!(42161)), None);
        assert_eq!(parse_chain_id(&json!("0xzz")), None);
        assert_eq!(parse_chain_id(&json!(null)), None);
    }

    #[test]
    fn probe_blocks_follow_the_archive_decision() {
        // An archive network under archive mode probes every configured block.
        assert_eq!(
            probe_blocks(net("base"), true),
            vec![Some(1), Some(39_000_000)]
        );
        assert_eq!(probe_blocks(net("arbitrum"), true), vec![Some(280_000_000)]);
        // --no-archive (deploy/broadcast) collapses to latest even for a
        // network that has pinned blocks.
        assert_eq!(probe_blocks(net("arbitrum"), false), vec![None]);
        // A latest-only network is latest in either mode.
        assert_eq!(probe_blocks(net("ethereum"), true), vec![None]);
        assert_eq!(probe_blocks(net("ethereum"), false), vec![None]);
    }

    #[test]
    fn block_tags_are_hex_or_latest() {
        assert_eq!(block_tag(None), "latest");
        assert_eq!(block_tag(Some(0)), "0x0");
        assert_eq!(block_tag(Some(1)), "0x1");
        // Hex, not decimal — a decimal block param is silently misread.
        assert_eq!(block_tag(Some(280_000_000)), "0x10b07600");
        assert_eq!(block_tag(Some(39_000_000)), "0x25317c0");
    }

    #[test]
    fn call_result_must_be_one_full_word() {
        let word = json!("0x0000000000000000000000000000000000000000000000000000000000000000");
        assert_eq!(check_call_result(&word, Some(1)), Ok(()));
        // "0x" is the node answering from empty state rather than erroring —
        // the silent not-archive that a naive probe would score as a pass.
        assert_eq!(
            check_call_result(&json!("0x"), Some(280_000_000)),
            Err(Reason::NotArchive {
                code: 0,
                block: 280_000_000
            })
        );
        // Short or long by one nibble is still not a word.
        assert!(matches!(
            check_call_result(&json!("0x00"), None),
            Err(Reason::NotArchive { .. })
        ));
        assert!(matches!(
            check_call_result(&json!(format!("0x{}", "0".repeat(63))), None),
            Err(Reason::NotArchive { .. })
        ));
        assert!(matches!(
            check_call_result(&json!(format!("0x{}", "0".repeat(65))), None),
            Err(Reason::NotArchive { .. })
        ));
        // Not a hex string at all.
        assert_eq!(
            check_call_result(&json!(null), None),
            Err(Reason::BadResponse)
        );
        assert_eq!(check_call_result(&json!(1), None), Err(Reason::BadResponse));
        assert_eq!(
            check_call_result(&json!("no prefix"), None),
            Err(Reason::BadResponse)
        );
    }

    #[test]
    fn network_table_is_internally_consistent() {
        let mut keys = BTreeSet::new();
        let mut envs = BTreeSet::new();
        let mut secrets = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for n in NETWORKS {
            assert!(keys.insert(n.key), "duplicate key {}", n.key);
            assert!(envs.insert(n.env_name), "duplicate env {}", n.env_name);
            assert!(
                secrets.insert(n.secret_name),
                "duplicate secret {}",
                n.secret_name
            );
            assert!(ids.insert(n.chain_id), "duplicate chain id {}", n.chain_id);
            // The secret name and the env name are different spellings of the
            // same network; both are demand-scan signals.
            assert_eq!(n.secret_name, format!("RPC_URL_{}_FORK", upper(n.key)));
            assert!(n.probe_contract.starts_with("0x") && n.probe_contract.len() == 42);
            assert!(n.defaults.iter().all(|d| d.starts_with("https://")));
        }
    }

    #[test]
    fn no_network_name_is_a_whole_word_mention_inside_another() {
        // This assertion used to be "no name is a SUBSTRING of another", which
        // held only while no network nested inside another. `sepolia` breaks
        // that literally — SEPOLIA_RPC_URL is a substring of
        // BASE_SEPOLIA_RPC_URL — and the table needs both names, so the scan
        // matches on identifier boundaries instead (`mentions`).
        //
        // The invariant that actually protects the demand scan is therefore the
        // boundary one: no network's name may match as a WHOLE WORD inside
        // another's, or a repo mentioning only the longer name would demand
        // both networks and be failed by a probe for a chain it never touches.
        for a in NETWORKS {
            for b in NETWORKS {
                if a.key != b.key {
                    assert!(
                        !mentions(b.env_name, a.env_name),
                        "{} matches {} as a whole word",
                        b.env_name,
                        a.env_name
                    );
                    assert!(
                        !mentions(b.secret_name, a.secret_name),
                        "{} matches {} as a whole word",
                        b.secret_name,
                        a.secret_name
                    );
                }
            }
        }
        // Guard against this test quietly becoming vacuous: the raw substring
        // nesting it was weakened FROM is real and still in the table, so
        // dropping the boundary rule would make `base_sepolia` match `sepolia`.
        assert!(net("base_sepolia")
            .env_name
            .contains(net("sepolia").env_name));
    }

    #[test]
    fn mentions_requires_identifier_boundaries() {
        // The nesting that forced the rule.
        assert!(!mentions("BASE_SEPOLIA_RPC_URL", "SEPOLIA_RPC_URL"));
        assert!(mentions("BASE_SEPOLIA_RPC_URL", "BASE_SEPOLIA_RPC_URL"));
        // Every punctuation a real reference is wrapped in still matches.
        assert!(mentions("${SEPOLIA_RPC_URL}", "SEPOLIA_RPC_URL"));
        assert!(mentions(
            "vm.envString(\"SEPOLIA_RPC_URL\")",
            "SEPOLIA_RPC_URL"
        ));
        assert!(mentions("SEPOLIA_RPC_URL=https://x", "SEPOLIA_RPC_URL"));
        assert!(mentions("  SEPOLIA_RPC_URL\n", "SEPOLIA_RPC_URL"));
        // Start and end of input are boundaries.
        assert!(mentions("SEPOLIA_RPC_URL", "SEPOLIA_RPC_URL"));
        // Adjacent identifier bytes on either side are not.
        assert!(!mentions("MY_SEPOLIA_RPC_URL", "SEPOLIA_RPC_URL"));
        assert!(!mentions("SEPOLIA_RPC_URL2", "SEPOLIA_RPC_URL"));
        assert!(!mentions("XSEPOLIA_RPC_URLX", "SEPOLIA_RPC_URL"));
        // A later valid occurrence still counts when an earlier one is nested.
        assert!(mentions(
            "BASE_SEPOLIA_RPC_URL and ${SEPOLIA_RPC_URL}",
            "SEPOLIA_RPC_URL"
        ));
        assert!(!mentions("", "SEPOLIA_RPC_URL"));
    }

    #[test]
    fn sepolia_is_modelled_so_its_secret_is_consumed() {
        // rainlanguage/rainix#340: the RPC_URL_SEPOLIA_FORK org secret existed
        // with no table entry, so it was never scanned, probed or exported.
        let n = net("sepolia");
        assert_eq!(n.chain_id, 11155111);
        assert_eq!(n.env_name, "SEPOLIA_RPC_URL");
        assert_eq!(n.secret_name, "RPC_URL_SEPOLIA_FORK");
        // Distinct from Base Sepolia in every field that identifies a network.
        let b = net("base_sepolia");
        assert_ne!(n.chain_id, b.chain_id);
        assert_ne!(n.probe_contract, b.probe_contract);
        assert!(!n.defaults.is_empty());
    }

    #[test]
    fn ethereum_prefers_the_endpoints_that_survived_ci() {
        // rainlanguage/rainix#340 demoted eth-pokt.nodies.app: it passes a
        // light probe from CI and then 408s under fork load. The burst check is
        // what actually protects the selection, but the declared preference
        // still must not LEAD with an endpoint that is known to fail under
        // sustained load from some vantage point — which, on 2026-08-20
        // measurement, is true of both pokt (from CI) and tenderly (from here).
        // eth.drpc.org is the only one with no evidence against it either way.
        let d = net("ethereum").defaults;
        assert_eq!(d.last(), Some(&"https://eth-pokt.nodies.app"));
        assert_eq!(d[0], "https://eth.drpc.org");
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn error_fields_reads_every_wire_shape_seen_in_the_wild() {
        // The spec envelope (tenderly, HTTP 429).
        assert_eq!(
            error_fields(&json!({"error": {"code": -32005, "message": "rate limit exceeded"}})),
            Some((-32005, "rate limit exceeded"))
        );
        // `error` as a bare STRING, served with HTTP 200 (1rpc.io). Unhandled,
        // this is a throttle that looks exactly like a healthy response.
        assert_eq!(
            error_fields(&json!({"error": "cu limit exceeded", "path": "/eth-sepolia"})),
            Some((0, "cu limit exceeded"))
        );
        // No envelope; the fields sit at the top level (drpc, HTTP 408).
        assert_eq!(
            error_fields(&json!({"message": "Request timeout on the free plan", "code": 30})),
            Some((30, "Request timeout on the free plan"))
        );
        // Missing pieces degrade to the "no code / no message" discriminants
        // rather than losing the error.
        assert_eq!(
            error_fields(&json!({"error": {"message": "boom"}})),
            Some((0, "boom"))
        );
        assert_eq!(
            error_fields(&json!({"error": {"code": -1}})),
            Some((-1, ""))
        );
    }

    #[test]
    fn error_fields_never_fires_on_a_healthy_response() {
        assert_eq!(error_fields(&json!({"result": "0x1"})), None);
        // A `message` beside a real `result` is not an error — only a top-level
        // message with NO result is.
        assert_eq!(
            error_fields(&json!({"result": "0x1", "message": "ok", "code": 0})),
            None
        );
        assert_eq!(error_fields(&json!({})), None);
        assert_eq!(error_fields(&json!({"code": 30})), None);
        // An explicit `"error": null` beside a real result. serde_json's `get`
        // returns `Some(Value::Null)` here, so an unfiltered read would default
        // code/message to 0/"" and reject a healthy endpoint as RpcError{0}.
        assert_eq!(error_fields(&json!({"result": "0x1", "error": null})), None);
        // The same null with no result at all is still not an error envelope.
        assert_eq!(error_fields(&json!({"error": null})), None);
    }

    #[test]
    fn http_429_is_a_quota_rejection_even_with_an_unparseable_body() {
        // An HTML rate-limit page carries no JSON to classify, but 429 means
        // rate limited by definition and that is the class this whole
        // subcommand routes around.
        assert_eq!(http_reason(429), Reason::Quota { code: 0 });
        assert_eq!(http_reason(500), Reason::Http { status: 500 });
        assert_eq!(http_reason(404), Reason::Http { status: 404 });
        assert_eq!(http_reason(521), Reason::Http { status: 521 });
        // Below 400 with a body we could not parse is malformed, not an HTTP
        // failure — the 200-with-an-error-body case.
        assert_eq!(http_reason(200), Reason::BadResponse);
    }

    #[test]
    fn burst_rejects_only_a_throttled_majority() {
        let q = Some(Reason::Quota { code: 30 });
        // Clean burst.
        assert_eq!(burst_verdict(&[None, None, None, None]), None);
        // A minority of throttled requests is tolerated: every healthy public
        // endpoint sheds the occasional one, and rejecting on that would make
        // the preflight flakier than the outage it prevents.
        assert_eq!(burst_verdict(&[q, None, None, None]), None);
        // Exactly half is still not a majority.
        assert_eq!(burst_verdict(&[q, q, None, None]), None);
        // A majority is a rejection, and it reports the throttle reason.
        assert_eq!(
            burst_verdict(&[q, q, q, None]),
            Some(Reason::Quota { code: 30 })
        );
        assert_eq!(burst_verdict(&[q, q]), Some(Reason::Quota { code: 30 }));
    }

    #[test]
    fn burst_ignores_failures_that_are_not_throttling() {
        // Correctness is settled by the sequential phase; the burst judges load
        // and nothing else. A transient timeout inside a wide burst must not
        // reject an endpoint that just passed every correctness check.
        let boom = Some(Reason::Unreachable { curl_exit: 28 });
        assert_eq!(burst_verdict(&[boom, boom, boom, boom]), None);
        assert_eq!(burst_verdict(&[Some(Reason::BadResponse); 4]), None);
        // Mixed: throttling is a minority of the burst even though most
        // requests failed for other reasons.
        assert_eq!(
            burst_verdict(&[Some(Reason::Quota { code: 1 }), boom, boom, boom]),
            None
        );
        // An empty burst (--burst 0) disables the check.
        assert_eq!(burst_verdict(&[]), None);
    }

    #[test]
    fn export_writes_one_line_per_name() {
        let dir = std::env::temp_dir().join(format!("rpc-preflight-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("github_env");
        let _ = std::fs::remove_file(&f);
        export(&f, "BASE_RPC_URL", &Url("https://a.example".into()));
        export(&f, "RPC_URL_BASE_FORK", &Url("https://a.example".into()));
        let got = std::fs::read_to_string(&f).unwrap();
        assert_eq!(
            got,
            "BASE_RPC_URL=https://a.example\nRPC_URL_BASE_FORK=https://a.example\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn demand_scan_finds_only_referenced_networks() {
        let dir = std::env::temp_dir().join(format!("rpc-preflight-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(st.success(), "git {args:?}");
        };
        git(&["init", "-q"]);
        // foundry.toml alias form, and the raw-secret-name form cyclo.sol uses.
        std::fs::write(
            dir.join("foundry.toml"),
            "[rpc_endpoints]\nbase_sepolia = \"${BASE_SEPOLIA_RPC_URL}\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("Prod.t.sol"),
            "vm.createSelectFork(vm.envString(\"RPC_URL_FLARE_FORK\"), 51262162);",
        )
        .unwrap();
        // Untracked file: not a demand signal.
        std::fs::write(dir.join("scratch.txt"), "ARBITRUM_RPC_URL").unwrap();
        git(&["add", "foundry.toml", "Prod.t.sol"]);

        let got = demanded(&dir);
        assert!(got.contains("base_sepolia"));
        assert!(got.contains("flare"));
        // base_sepolia must not drag base in, and the untracked mention of
        // arbitrum must not count.
        assert!(!got.contains("base"));
        assert!(!got.contains("arbitrum"));
        // Nor may it drag SEPOLIA in. BASE_SEPOLIA_RPC_URL literally contains
        // SEPOLIA_RPC_URL, so under the old substring scan this repo — and most
        // of the org — would have demanded a network it never touches, and been
        // failed by a probe for it.
        assert!(!got.contains("sepolia"));
        assert_eq!(got.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plan_gating_is_a_quota_rejection_not_a_pruning_node() {
        // Live from https://sepolia.drpc.org today: a free-plan gate, HTTP 400.
        // The message contains "is not available", which the archive arm also
        // matches — so the plan arm has to win, or a billing wall is reported
        // as a pruning node and the actionable fact is lost.
        assert_eq!(
            classify(
                35,
                "chain is not available on free plan, please upgrade to paid plan",
                Some(38_000_000)
            ),
            Reason::Quota { code: 35 }
        );
    }

    #[test]
    fn free_plan_throttle_wording_is_quota() {
        // The body rainix#340 captured off eth-pokt.nodies.app under fork load.
        assert_eq!(
            classify(
                30,
                "Request timeout on the free plan, please upgrade to paid plan",
                None
            ),
            Reason::Quota { code: 30 }
        );
        // Live from https://1rpc.io/sepolia under a 16-way burst.
        assert_eq!(
            classify(0, "cu limit exceeded; Method \"eth_call\" is not available for unregistered accounts. Please register", None),
            Reason::Quota { code: 0 }
        );
        // Live from https://eth.drpc.org under an 8-way burst.
        assert_eq!(
            classify(
                15,
                "You reached Public endpoint rate limit, please upgrade to paid plan",
                None
            ),
            Reason::Quota { code: 15 }
        );
        // The same drpc body cut to the half that names the plan — the shape it
        // arrives in when the provider omits the remedy clause, and the exact
        // string `error_fields` is tested against above.
        //
        // Every other case here says "upgrade to paid plan" as well, so all of
        // them stay quota on the strength of those two needles alone and NONE
        // of them pins "free plan". This one carries no "upgrade to", no "paid
        // plan", no "quota", no "rate limit" and no "exceeded": strike the
        // "free plan" needle and it falls all the way through to a bare
        // RpcError, which reports a billing wall as an unexplained failure and
        // fails the network over for no stated reason. That makes this the one
        // case that fails when the needle is removed.
        assert_eq!(
            classify(30, "Request timeout on the free plan", None),
            Reason::Quota { code: 30 }
        );
    }
}
