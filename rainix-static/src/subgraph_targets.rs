//! `subgraph-targets` — what `subgraph-deploy` deploys, one line per network.
//!
//! The subgraph lives in `<dir>`, one plain directory name in the repo root
//! (`$SUBGRAPH_DIR`, default `subgraph`), so it can never name a path outside
//! it. Two shapes:
//!
//! - `<dir>/networks.json`: one `subgraph.yaml` built per network with
//!   `--network`, labelled by that network's one data source address.
//! - `<dir>/manifests/<network>.yaml`: one manifest per network, built as
//!   written, for a subgraph whose data sources differ per network (so it has
//!   no single address; its label prefix is `manifest`).
//!
//! Prints `network TAB manifest TAB label-prefix` per network (the manifest is
//! empty in the networks.json shape), networks in byte order. Exits 1 when the
//! directory name is not plain, it has neither shape, a networks.json network
//! does not have exactly one data source with an address, or a manifest
//! name is not a plain network name.

use crate::fail;
use std::path::Path;

/// One deploy target.
#[derive(Debug, PartialEq)]
pub(crate) struct Target {
    pub network: String,
    /// Relative to the subgraph directory; empty for the networks.json shape.
    pub manifest: String,
    pub label_prefix: String,
}

/// A plain name: no separators, no leading dot, nothing a shell or a path
/// would read as more than one component.
fn plain(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

pub(crate) fn targets(root: &Path, dir: &str) -> Result<Vec<Target>, String> {
    if !plain(dir) {
        return Err(format!("SUBGRAPH_DIR {dir:?} is not one plain directory name"));
    }
    let base = root.join(dir);
    let networks_json = base.join("networks.json");
    if networks_json.is_file() {
        let text = std::fs::read_to_string(&networks_json)
            .map_err(|e| format!("cannot read {}: {e}", networks_json.display()))?;
        let doc: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("{} is not JSON: {e}", networks_json.display()))?;
        let networks = doc
            .as_object()
            .ok_or_else(|| format!("{} is not an object", networks_json.display()))?;
        let mut out = Vec::new();
        for (network, sources) in networks {
            let sources = sources
                .as_object()
                .ok_or_else(|| format!("networks.json {network:?} is not an object"))?;
            if sources.len() != 1 {
                return Err(format!(
                    "networks.json {network:?} has {} data sources; the label names one address, \
                     so a subgraph with several per network uses manifests/<network>.yaml",
                    sources.len()
                ));
            }
            let address = sources
                .values()
                .next()
                .and_then(|s| s.get("address"))
                .and_then(|a| a.as_str())
                .ok_or_else(|| format!("networks.json {network:?} has no address"))?;
            out.push(Target {
                network: network.clone(),
                manifest: String::new(),
                label_prefix: address.to_string(),
            });
        }
        out.sort_by(|a, b| a.network.cmp(&b.network));
        return Ok(out);
    }
    let manifests = base.join("manifests");
    let mut out = Vec::new();
    if manifests.is_dir() {
        let entries = std::fs::read_dir(&manifests)
            .map_err(|e| format!("cannot read {}: {e}", manifests.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", manifests.display()))?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| format!("{} has a non-UTF-8 name", manifests.display()))?;
            let Some(network) = name.strip_suffix(".yaml") else {
                continue;
            };
            if !plain(network) || !entry.path().is_file() {
                return Err(format!("manifests/{name} is not a plain <network>.yaml file"));
            }
            out.push(Target {
                network: network.to_string(),
                manifest: format!("manifests/{name}"),
                label_prefix: "manifest".to_string(),
            });
        }
    }
    if out.is_empty() {
        return Err(format!("{dir} has neither networks.json nor manifests/*.yaml"));
    }
    out.sort_by(|a, b| a.network.cmp(&b.network));
    Ok(out)
}

pub(crate) fn run(dir: &str) {
    match targets(Path::new("."), dir) {
        Ok(ts) => {
            for t in ts {
                println!("{}\t{}\t{}", t.network, t.manifest, t.label_prefix);
            }
        }
        Err(e) => fail(&format!("subgraph-targets: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "subgraph-targets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn t(network: &str, manifest: &str, prefix: &str) -> Target {
        Target {
            network: network.into(),
            manifest: manifest.into(),
            label_prefix: prefix.into(),
        }
    }

    #[test]
    fn networks_json_labels_each_network_by_its_address_in_network_order() {
        let root = tmp();
        std::fs::create_dir_all(root.join("subgraph")).unwrap();
        std::fs::write(
            root.join("subgraph/networks.json"),
            r#"{"mainnet":{"C":{"address":"0xabc"}},"base":{"C":{"address":"0xbbb"}}}"#,
        )
        .unwrap();
        assert_eq!(
            targets(&root, "subgraph").unwrap(),
            vec![t("base", "", "0xbbb"), t("mainnet", "", "0xabc")]
        );
    }

    #[test]
    fn networks_json_with_two_sources_on_a_network_is_refused() {
        let root = tmp();
        std::fs::create_dir_all(root.join("subgraph")).unwrap();
        std::fs::write(
            root.join("subgraph/networks.json"),
            r#"{"base":{"A":{"address":"0x1"},"B":{"address":"0x2"}}}"#,
        )
        .unwrap();
        assert!(targets(&root, "subgraph").unwrap_err().contains("2 data sources"));
    }

    #[test]
    fn networks_json_without_an_address_is_refused() {
        let root = tmp();
        std::fs::create_dir_all(root.join("subgraph")).unwrap();
        std::fs::write(root.join("subgraph/networks.json"), r#"{"base":{"A":{}}}"#).unwrap();
        assert!(targets(&root, "subgraph").unwrap_err().contains("no address"));
    }

    #[test]
    fn manifests_deploy_each_yaml_as_written_and_ignore_other_files() {
        let root = tmp();
        std::fs::create_dir_all(root.join("tokens/manifests")).unwrap();
        std::fs::write(root.join("tokens/manifests/matic.yaml"), "x").unwrap();
        std::fs::write(root.join("tokens/manifests/arbitrum-one.yaml"), "x").unwrap();
        std::fs::write(root.join("tokens/manifests/README.md"), "x").unwrap();
        assert_eq!(
            targets(&root, "tokens").unwrap(),
            vec![
                t("arbitrum-one", "manifests/arbitrum-one.yaml", "manifest"),
                t("matic", "manifests/matic.yaml", "manifest"),
            ]
        );
    }

    #[test]
    fn networks_json_wins_over_manifests() {
        let root = tmp();
        std::fs::create_dir_all(root.join("s/manifests")).unwrap();
        std::fs::write(root.join("s/manifests/base.yaml"), "x").unwrap();
        std::fs::write(root.join("s/networks.json"), r#"{"mainnet":{"C":{"address":"0x1"}}}"#)
            .unwrap();
        assert_eq!(targets(&root, "s").unwrap(), vec![t("mainnet", "", "0x1")]);
    }

    #[test]
    fn a_directory_with_neither_shape_is_refused() {
        let root = tmp();
        std::fs::create_dir_all(root.join("empty/manifests")).unwrap();
        assert!(targets(&root, "empty").unwrap_err().contains("neither"));
        assert!(targets(&root, "missing").unwrap_err().contains("neither"));
    }

    #[test]
    fn a_dir_or_manifest_name_that_is_not_plain_is_refused() {
        let root = tmp();
        for bad in ["", "..", ".hidden", "a/b", "../x", "a b", "$(x)"] {
            assert!(
                targets(&root, bad).unwrap_err().contains("not one plain"),
                "{bad:?} accepted"
            );
        }
        std::fs::create_dir_all(root.join("s/manifests")).unwrap();
        std::fs::write(root.join("s/manifests/.x.yaml"), "x").unwrap();
        assert!(targets(&root, "s").unwrap_err().contains("not a plain"));
    }
}
