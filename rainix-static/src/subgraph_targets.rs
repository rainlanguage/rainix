//! `subgraph-targets` — what `subgraph-deploy` deploys, one line per network.
//!
//! The subgraph lives in `<dir>`, one plain directory name in the repo root
//! (`$SUBGRAPH_DIR`, default `subgraph`), so it can never name a path outside
//! it. Two shapes:
//!
//! - `<dir>/networks.json`: one `subgraph.yaml` built per network with
//!   `--network`, labelled by the address of that network's first data source
//!   in file order.
//! - `<dir>/manifests/<network>.yaml`: one manifest per network, built as
//!   written, for a subgraph whose data sources differ per network (so it has
//!   no single address; its label prefix is `manifest`).
//!
//! Each network deploys to one host: Goldsky for a network in
//! [`GOLDSKY_NETWORKS`] (Ormi does not index it), Ormi for every other.
//!
//! Prints `network TAB host TAB label-prefix TAB manifest` per network, networks in byte
//! order. The manifest is empty in the networks.json shape, so it is the LAST
//! field: tab is IFS whitespace to bash `read`, which collapses an empty field
//! between two tabs and would shift the label into the manifest. Exits 1 when the
//! directory name is not plain, it has neither shape, a networks.json network
//! has no first data source with an address, or a manifest name is not a
//! plain network name.

use crate::fail;
use std::path::Path;

/// One deploy target.
#[derive(Debug, PartialEq)]
pub(crate) struct Target {
    pub network: String,
    /// `ormi` or `goldsky`: see [`host`].
    pub host: &'static str,
    /// Relative to the subgraph directory; empty for the networks.json shape.
    pub manifest: String,
    pub label_prefix: String,
}

/// A JSON object's entries in FILE order. `serde_json::Map` sorts its keys
/// (this crate does not enable `preserve_order`), which would make "the first
/// data source" mean "the alphabetically first".
struct OrderedMap<V>(Vec<(String, V)>);

impl<'de, V: serde::Deserialize<'de>> serde::Deserialize<'de> for OrderedMap<V> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Entries<V>(std::marker::PhantomData<V>);
        impl<'de, V: serde::Deserialize<'de>> serde::de::Visitor<'de> for Entries<V> {
            type Value = OrderedMap<V>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut m: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = m.next_entry::<String, V>()? {
                    out.push(entry);
                }
                Ok(OrderedMap(out))
            }
        }
        d.deserialize_map(Entries(std::marker::PhantomData))
    }
}

/// Graph network names Ormi does not index, so their subgraphs deploy to
/// Goldsky. Every other network deploys to Ormi.
pub(crate) const GOLDSKY_NETWORKS: &[&str] = &["0g"];

/// The host a network's subgraph deploys to.
pub(crate) fn host(network: &str) -> &'static str {
    if GOLDSKY_NETWORKS.contains(&network) {
        "goldsky"
    } else {
        "ormi"
    }
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
        return Err(format!(
            "SUBGRAPH_DIR {dir:?} is not one plain directory name"
        ));
    }
    let base = root.join(dir);
    let networks_json = base.join("networks.json");
    if networks_json.is_file() {
        let text = std::fs::read_to_string(&networks_json)
            .map_err(|e| format!("cannot read {}: {e}", networks_json.display()))?;
        let networks: OrderedMap<OrderedMap<serde_json::Value>> = serde_json::from_str(&text)
            .map_err(|e| {
                format!(
                    "{} is not an object of network objects: {e}",
                    networks_json.display()
                )
            })?;
        let mut out = Vec::new();
        for (network, sources) in networks.0 {
            // The FIRST data source in file order labels the network, as
            // `jq '.[$net] | to_entries[0]'` did before this was Rust.
            let address = sources
                .0
                .first()
                .and_then(|(_, s)| s.get("address"))
                .and_then(|a| a.as_str())
                .ok_or_else(|| format!("networks.json {network:?} has no address"))?;
            out.push(Target {
                host: host(&network),
                network,
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
                return Err(format!(
                    "manifests/{name} is not a plain <network>.yaml file"
                ));
            }
            out.push(Target {
                host: host(network),
                network: network.to_string(),
                manifest: format!("manifests/{name}"),
                label_prefix: "manifest".to_string(),
            });
        }
    }
    if out.is_empty() {
        return Err(format!(
            "{dir} has neither networks.json nor manifests/*.yaml"
        ));
    }
    out.sort_by(|a, b| a.network.cmp(&b.network));
    Ok(out)
}

pub(crate) fn run(dir: &str) {
    match targets(Path::new("."), dir) {
        Ok(ts) => {
            for t in ts {
                println!(
                    "{}\t{}\t{}\t{}",
                    t.network, t.host, t.label_prefix, t.manifest
                );
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
            host: host(network),
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
    fn networks_json_with_two_sources_labels_by_the_first_in_file_order() {
        let root = tmp();
        std::fs::create_dir_all(root.join("subgraph")).unwrap();
        // `B` precedes `A` in the file, so file order and sorted order disagree.
        std::fs::write(
            root.join("subgraph/networks.json"),
            r#"{"base":{"B":{"address":"0x2"},"A":{"address":"0x1"}}}"#,
        )
        .unwrap();
        assert_eq!(
            targets(&root, "subgraph").unwrap(),
            vec![t("base", "", "0x2")]
        );
    }

    #[test]
    fn networks_json_that_is_not_an_object_of_objects_is_refused() {
        let root = tmp();
        std::fs::create_dir_all(root.join("subgraph")).unwrap();
        for bad in ["[]", r#"{"base":[]}"#, "nope"] {
            std::fs::write(root.join("subgraph/networks.json"), bad).unwrap();
            assert!(
                targets(&root, "subgraph")
                    .unwrap_err()
                    .contains("not an object"),
                "{bad} accepted"
            );
        }
        std::fs::write(root.join("subgraph/networks.json"), r#"{"base":{}}"#).unwrap();
        assert!(targets(&root, "subgraph")
            .unwrap_err()
            .contains("no address"));
    }

    #[test]
    fn networks_json_without_an_address_is_refused() {
        let root = tmp();
        std::fs::create_dir_all(root.join("subgraph")).unwrap();
        std::fs::write(root.join("subgraph/networks.json"), r#"{"base":{"A":{}}}"#).unwrap();
        assert!(targets(&root, "subgraph")
            .unwrap_err()
            .contains("no address"));
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
    fn manifests_are_listed_in_network_order_whatever_the_directory_order() {
        let root = tmp();
        std::fs::create_dir_all(root.join("m/manifests")).unwrap();
        // Written in reverse, so neither creation order nor a lucky directory
        // order can stand in for the sort.
        let names = ["h", "g", "f", "e", "d", "c", "b", "a"];
        for n in names {
            std::fs::write(root.join(format!("m/manifests/{n}.yaml")), "x").unwrap();
        }
        let got: Vec<String> = targets(&root, "m")
            .unwrap()
            .into_iter()
            .map(|t| t.network)
            .collect();
        let mut want: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn a_goldsky_network_deploys_to_goldsky_and_every_other_to_ormi() {
        let root = tmp();
        std::fs::create_dir_all(root.join("s/manifests")).unwrap();
        std::fs::write(root.join("s/manifests/0g.yaml"), "x").unwrap();
        std::fs::write(root.join("s/manifests/base.yaml"), "x").unwrap();
        let hosts: Vec<(String, &str)> = targets(&root, "s")
            .unwrap()
            .into_iter()
            .map(|t| (t.network, t.host))
            .collect();
        assert_eq!(
            hosts,
            vec![("0g".to_string(), "goldsky"), ("base".to_string(), "ormi")]
        );
        std::fs::write(
            root.join("s/networks.json"),
            r#"{"0g":{"C":{"address":"0x1"}},"base":{"C":{"address":"0x2"}}}"#,
        )
        .unwrap();
        let hosts: Vec<&str> = targets(&root, "s")
            .unwrap()
            .into_iter()
            .map(|t| t.host)
            .collect();
        assert_eq!(hosts, vec!["goldsky", "ormi"]);
    }

    #[test]
    fn networks_json_wins_over_manifests() {
        let root = tmp();
        std::fs::create_dir_all(root.join("s/manifests")).unwrap();
        std::fs::write(root.join("s/manifests/base.yaml"), "x").unwrap();
        std::fs::write(
            root.join("s/networks.json"),
            r#"{"mainnet":{"C":{"address":"0x1"}}}"#,
        )
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
