//! `subgraph-build-hash` — the version label suffix for one network's build.
//!
//! `subgraph-deploy` labels a deployment by what it deploys, so a network whose
//! build is unchanged gets the label it already has on Ormi and is skipped by
//! the probe. Ormi refuses an identical build under a second label (E017), so a
//! label taken from anything other than the build — the repo commit, say —
//! turns every unrelated commit into a failed deploy on every network.
//!
//! The digest is SHA-256 over every file under the `graph build` output
//! directory, in byte order of their `/`-separated relative paths, each framed
//! as `path NUL length(u64 BE) contents` so no two trees share an encoding. The
//! build manifest carries the network and its addresses, so the digest is per
//! network. Anything that is not a regular file or directory is refused: a
//! build never writes one, and following it would hash content outside the
//! build.
//!
//! Prints the first `HEX_LEN` hex digits; exits 1 on any failure.

use crate::fail;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Hex digits printed. 64 bits keeps collisions between two builds of one
/// subgraph out of reach while leaving the label short enough for a URL path.
pub(crate) const HEX_LEN: usize = 16;

/// Every regular file under `root` as (relative `/`-joined path, absolute
/// path), sorted by relative path.
fn files(root: &Path) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .expect("walked path is under root")
                    .components()
                    .map(|c| {
                        c.as_os_str()
                            .to_str()
                            .map(str::to_string)
                            .ok_or_else(|| format!("{} is not a UTF-8 path", path.display()))
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join("/");
                out.push((rel, path));
            } else {
                return Err(format!(
                    "{} is neither a regular file nor a directory",
                    path.display()
                ));
            }
        }
    }
    out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    Ok(out)
}

/// Full hex SHA-256 of the build tree at `root`.
pub(crate) fn digest(root: &Path) -> Result<String, String> {
    let files = files(root)?;
    if files.is_empty() {
        return Err(format!("{} contains no files", root.display()));
    }
    let mut h = Sha256::new();
    for (rel, path) in files {
        let content =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        h.update(rel.as_bytes());
        h.update([0u8]);
        h.update((content.len() as u64).to_be_bytes());
        h.update(&content);
    }
    Ok(h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

pub(crate) fn run(dir: &str) {
    match digest(Path::new(dir)) {
        Ok(hex) => println!("{}", &hex[..HEX_LEN]),
        Err(e) => fail(&format!("subgraph-build-hash: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rainix-static-build-hash-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn tree(files: &[(&str, &str)]) -> std::path::PathBuf {
        let d = tmp();
        for (rel, content) in files {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        d
    }

    #[test]
    fn known_digest() {
        // sha256("a\0" ++ 0x0000000000000001 ++ "x")
        let mut h = Sha256::new();
        h.update(b"a\0\0\0\0\0\0\0\0\x01x");
        let want: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(digest(&tree(&[("a", "x")])).unwrap(), want);
    }

    #[test]
    fn same_tree_in_different_places_and_orders_hashes_equal() {
        let a = tree(&[("subgraph.yaml", "m"), ("C/C.wasm", "w"), ("schema.graphql", "s")]);
        let b = tree(&[("schema.graphql", "s"), ("C/C.wasm", "w"), ("subgraph.yaml", "m")]);
        assert_eq!(digest(&a).unwrap(), digest(&b).unwrap());
    }

    #[test]
    fn content_change_changes_digest() {
        let a = tree(&[("subgraph.yaml", "address: 0xaaa")]);
        let b = tree(&[("subgraph.yaml", "address: 0xbbb")]);
        assert_ne!(digest(&a).unwrap(), digest(&b).unwrap());
    }

    #[test]
    fn rename_changes_digest() {
        let a = tree(&[("A/m.wasm", "w")]);
        let b = tree(&[("B/m.wasm", "w")]);
        assert_ne!(digest(&a).unwrap(), digest(&b).unwrap());
    }

    #[test]
    fn added_empty_file_changes_digest() {
        let a = tree(&[("m", "w")]);
        let b = tree(&[("m", "w"), ("n", "")]);
        assert_ne!(digest(&a).unwrap(), digest(&b).unwrap());
    }

    #[test]
    fn path_content_boundary_is_framed() {
        // Unframed, both would hash "ab" ++ "c".
        let a = tree(&[("ab", "c")]);
        let b = tree(&[("a", "bc")]);
        assert_ne!(digest(&a).unwrap(), digest(&b).unwrap());
        // Unframed by length, both would hash "a\0" "x" "b\0" "" vs "a\0" "xb\0".
        let c = tree(&[("a", "x"), ("b", "")]);
        let d = tree(&[("a", "xb\0")]);
        assert_ne!(digest(&c).unwrap(), digest(&d).unwrap());
    }

    #[test]
    fn empty_or_missing_dir_is_an_error() {
        assert!(digest(&tmp()).unwrap_err().contains("no files"));
        assert!(digest(&tmp().join("nope"))
            .unwrap_err()
            .contains("cannot read"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_refused() {
        let d = tree(&[("m", "w")]);
        std::os::unix::fs::symlink("/etc/hostname", d.join("link")).unwrap();
        assert!(digest(&d)
            .unwrap_err()
            .contains("neither a regular file nor a directory"));
    }
}
