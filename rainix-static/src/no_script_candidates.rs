//! `no-script-candidates` — fail if a `script/**/*.s.sol` reaches a candidate.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Resolution is solc's `referencedDeclaration`, so import aliases cannot dodge it.
pub(crate) fn check(build_info_dir: &Path) -> Result<Vec<String>, String> {
    let mut offenders = Vec::new();
    let entries = std::fs::read_dir(build_info_dir)
        .map_err(|e| format!("cannot read {}: {e}", build_info_dir.display()))?;
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no build-info in {}", build_info_dir.display()));
    }
    for f in files {
        let text = std::fs::read_to_string(&f)
            .map_err(|e| format!("cannot read {}: {e}", f.display()))?;
        let info: Value = serde_json::from_str(&text)
            .map_err(|e| format!("cannot parse {}: {e}", f.display()))?;
        offenders.extend(check_build_info(&info)?);
    }
    offenders.sort();
    offenders.dedup();
    if !offenders.is_empty() {
        offenders.insert(
            0,
            "Scripts are prod and must reference frozen release constants, never a candidate:"
                .to_string(),
        );
    }
    Ok(offenders)
}

#[derive(Default)]
struct Graph {
    names: HashMap<u64, String>,
    edges: HashMap<u64, HashSet<u64>>,
    uses: Vec<(String, u64, u64)>,
}

pub(crate) fn check_build_info(info: &Value) -> Result<Vec<String>, String> {
    let sources = info
        .pointer("/output/sources")
        .and_then(Value::as_object)
        .ok_or("build-info has no output.sources")?;
    let mut g = Graph::default();
    for (path, src) in sources {
        let ast = src.get("ast").ok_or(format!("{path}: no ast"))?;
        let is_script = path.starts_with("script/") && path.ends_with(".s.sol");
        walk(ast, None, is_script.then_some(path.as_str()), &mut g);
    }
    let mut tainted: HashSet<u64> = g
        .names
        .iter()
        .filter(|(_, n)| n.contains("_CANDIDATE"))
        .map(|(&id, _)| id)
        .collect();
    loop {
        let before = tainted.len();
        for (&unit, refs) in &g.edges {
            if !tainted.contains(&unit) && refs.iter().any(|r| tainted.contains(r)) {
                tainted.insert(unit);
            }
        }
        if tainted.len() == before {
            break;
        }
    }
    let mut out = Vec::new();
    for (path, offset, id) in &g.uses {
        if tainted.contains(id) {
            let content = info
                .pointer(&format!(
                    "/input/sources/{}/content",
                    path.replace('~', "~0").replace('/', "~1")
                ))
                .and_then(Value::as_str)
                .unwrap_or("");
            let line = content
                .as_bytes()
                .iter()
                .take(*offset as usize)
                .filter(|&&c| c == b'\n')
                .count()
                + 1;
            let name = g.names.get(id).map(String::as_str).unwrap_or("?");
            out.push(format!("  {path}:{line}: {name}"));
        }
    }
    Ok(out)
}

fn is_unit(node: &Value) -> bool {
    match node.get("nodeType").and_then(Value::as_str) {
        Some("FunctionDefinition" | "ModifierDefinition") => true,
        Some("VariableDeclaration") => {
            node.get("stateVariable").and_then(Value::as_bool) == Some(true)
                || node.get("constant").and_then(Value::as_bool) == Some(true)
        }
        _ => false,
    }
}

fn walk(node: &Value, unit: Option<u64>, script: Option<&str>, g: &mut Graph) {
    match node {
        Value::Array(a) => a.iter().for_each(|n| walk(n, unit, script, g)),
        Value::Object(o) => {
            let id = o.get("id").and_then(Value::as_u64);
            if let (Some(id), Some(name)) = (id, o.get("name").and_then(Value::as_str)) {
                g.names.insert(id, name.to_string());
            }
            let unit = if is_unit(node) { id.or(unit) } else { unit };
            if let Some(r) = o.get("referencedDeclaration").and_then(Value::as_u64) {
                if let Some(u) = unit {
                    g.edges.entry(u).or_default().insert(r);
                }
                if let Some(path) = script {
                    let offset = o
                        .get("src")
                        .and_then(Value::as_str)
                        .and_then(|s| s.split(':').next())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    g.uses.push((path.to_string(), offset, r));
                }
            }
            o.values().for_each(|v| walk(v, unit, script, g));
        }
        _ => {}
    }
}
