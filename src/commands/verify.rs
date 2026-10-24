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
    if let Some(kinds) = interface
        .kinds()
        .context("Verification failed: Invalid main.wasm")?
    {
        let listed: Vec<&str> = manifest
            .kinds
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        if listed != kinds {
            bail!(
                "Verification failed: manifest kinds {listed:?} don't match main.wasm's {kinds:?}"
            );
        }
        if !kinds.contains(&manifest.plugin_type.as_str()) {
            bail!(
                "Verification failed: manifest type \"{}\" is not one of its kinds {kinds:?}",
                manifest.plugin_type
            );
        }
    }

    tracing::info!(
        "Plugin {} v{} verified successfully (ABI {}).",
        manifest.name,
        manifest.version,
        interface.abi()
    );
    Ok(())
}
