use std::path::Path;

pub(crate) const STAGED_DIR: &str = ".staged-config";

/// Foundry refuses a cheatcode write to the root `foundry.toml`, so a generated
/// block marked there can only be refreshed through the staging dir.
pub(crate) const GENERATED_MARKER: &str = "# rain-deploy:generated:";

pub(crate) fn stages_config(root: &Path) -> Result<bool, String> {
    match std::fs::read_to_string(root.join("foundry.toml")) {
        Ok(text) => Ok(text
            .lines()
            .any(|line| line.trim_start().starts_with(GENERATED_MARKER))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("failed to read foundry.toml: {e}")),
    }
}

/// Copies every regular file in the staging dir onto the root file of the
/// same name, then removes the dir. Returns the installed names, sorted.
pub(crate) fn install(root: &Path) -> Result<Vec<String>, String> {
    let staged = root.join(STAGED_DIR);
    if !staged.is_dir() {
        if stages_config(root)? {
            return Err(format!(
                "{STAGED_DIR}/ does not exist, but foundry.toml has generated blocks only it \
                 can refresh. forge script ./script/Build.sol stages the generated config \
                 there; it did not run, or it wrote nothing."
            ));
        }
        return Ok(Vec::new());
    }
    let entries =
        std::fs::read_dir(&staged).map_err(|e| format!("failed to read {STAGED_DIR}/: {e}"))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read {STAGED_DIR}/: {e}"))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("failed to stat {}: {e}", entry.path().display()))?;
        if kind.is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    if names.is_empty() {
        return Err(format!(
            "{STAGED_DIR}/ holds no files. forge script ./script/Build.sol stages the \
             generated config there; it wrote nothing."
        ));
    }
    names.sort();
    for name in &names {
        std::fs::copy(staged.join(name), root.join(name))
            .map_err(|e| format!("failed to install {STAGED_DIR}/{name}: {e}"))?;
    }
    std::fs::remove_dir_all(&staged)
        .map_err(|e| format!("failed to remove {STAGED_DIR}/: {e}"))?;
    Ok(names)
}

pub(crate) fn run(root: &Path) {
    match install(root) {
        Err(e) => crate::fail(&format!("install-staged-config: {e}")),
        Ok(names) if names.is_empty() => {
            println!("install-staged-config: nothing staged and nothing to stage")
        }
        Ok(names) => println!(
            "install-staged-config: installed {} from {STAGED_DIR}/",
            names.join(", ")
        ),
    }
}
