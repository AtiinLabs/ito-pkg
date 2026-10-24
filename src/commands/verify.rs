use crate::models::PluginManifest;
use crate::wasm::WasmInterface;
use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

pub fn verify_plugin(path: PathBuf) -> Result<()> {
    if !path.exists() {
        bail!("File not found: {:?}", path);
    }

    let file = File::open(&path).context("Failed to open file")?;
    let mut archive = zip::ZipArchive::new(file).context("Failed to read as ZIP archive")?;

    let mut has_manifest = false;
    let mut has_wasm = false;

    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .context("Failed to access file in archive")?;
        match file.name() {
            "manifest.json" => has_manifest = true,
            "main.wasm" => has_wasm = true,
            _ => {}
        }
    }

    if !has_manifest || !has_wasm {
        bail!("Verification failed: Missing manifest.json or main.wasm in the archive.");
    }

    // Read manifest
    let mut manifest_file = archive
        .by_name("manifest.json")
        .context("Failed to open manifest.json inside archive")?;
    let mut manifest_str = String::new();
    manifest_file
        .read_to_string(&mut manifest_str)
        .context("Failed to read manifest.json")?;

    let manifest: PluginManifest = serde_json::from_str(&manifest_str)
        .context("Verification failed: Invalid manifest JSON")?;
    drop(manifest_file);

    let mut wasm = Vec::new();
    archive
        .by_name("main.wasm")
        .context("Failed to open main.wasm inside archive")?
        .read_to_end(&mut wasm)
        .context("Failed to read main.wasm")?;
    let interface = WasmInterface::read(&wasm).context("Verification failed: Invalid main.wasm")?;

    let unknown_imports = interface.unknown_imports();
    if !unknown_imports.is_empty() {
        bail!(
            "Verification failed: the plugin imports functions no Ito host provides: {}",
            unknown_imports.join(", ")
        );
    }
    if let Some(abi) = manifest.abi
        && abi != interface.abi()
    {
        bail!(
            "Verification failed: manifest says ABI {abi}, main.wasm is ABI {}",
            interface.abi()
        );
    }
    if let Some(imports) = &manifest.imports
        && *imports != interface.import_names()
    {
        bail!(
            "Verification failed: manifest imports {:?} don't match main.wasm's {:?}",
            imports,
            interface.import_names()
        );
    }
    check_kinds(
        &manifest,
        interface
            .kinds()
            .context("Verification failed: Invalid main.wasm")?,
    )?;

    tracing::info!(
        "Plugin {} v{} verified successfully (ABI {}).",
        manifest.name,
        manifest.version,
        interface.abi()
    );
    Ok(())
}

/// The manifest's `kinds` and `type` must be what the module serves: its kinds from ABI 2, its
/// `type` alone before.
fn check_kinds(manifest: &PluginManifest, kinds: Option<Vec<&str>>) -> Result<()> {
    let Some(kinds) = kinds else {
        if manifest.kinds.is_some() {
            bail!("Verification failed: kinds are only recorded for ABI 2 plugins");
        }
        if !crate::wasm::KINDS.contains(&manifest.plugin_type.as_str()) {
            bail!(
                "Verification failed: manifest type \"{}\" is not a kind",
                manifest.plugin_type
            );
        }
        return Ok(());
    };
    let listed: Vec<&str> = manifest
        .kinds
        .iter()
        .flatten()
        .map(String::as_str)
        .collect();
    if listed != kinds {
        bail!("Verification failed: manifest kinds {listed:?} don't match main.wasm's {kinds:?}");
    }
    if !kinds.contains(&manifest.plugin_type.as_str()) {
        bail!(
            "Verification failed: manifest type \"{}\" is not one of its kinds {kinds:?}",
            manifest.plugin_type
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(plugin_type: &str, kinds: Option<&[&str]>) -> PluginManifest {
        let mut manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "id": "p", "name": "P", "version": "1.0.0", "min_app_version": "1.0.0", "type": plugin_type
        }))
        .unwrap();
        manifest.kinds = kinds.map(|kinds| kinds.iter().map(|kind| kind.to_string()).collect());
        manifest
    }

    #[test]
    fn an_abi_2_manifest_must_list_exactly_the_modules_kinds() {
        let served = Some(vec!["manga", "novel"]);
        assert!(
            check_kinds(
                &manifest("novel", Some(&["manga", "novel"])),
                served.clone()
            )
            .is_ok()
        );
        assert!(check_kinds(&manifest("novel", Some(&["novel"])), served.clone()).is_err());
        assert!(check_kinds(&manifest("novel", None), served.clone()).is_err());
        assert!(check_kinds(&manifest("anime", Some(&["manga", "novel"])), served).is_err());
    }

    #[test]
    fn an_older_manifest_names_one_known_kind_and_no_kinds_list() {
        assert!(check_kinds(&manifest("anime", None), None).is_ok());
        assert!(check_kinds(&manifest("Manga", None), None).is_err());
        assert!(check_kinds(&manifest("manga", Some(&["manga"])), None).is_err());
    }
}
