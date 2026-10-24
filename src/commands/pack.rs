use crate::models::{CargoLock, CargoToml, PluginManifest};
use crate::wasm::WasmInterface;
use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::write::SimpleFileOptions;

pub fn pack_plugin(plugin_dir: PathBuf) -> Result<()> {
    let plugin_dir = plugin_dir
        .canonicalize()
        .context("Failed to canonicalize plugin directory path")?;

    let cargo_toml_path = plugin_dir.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        bail!("Cargo.toml not found in directory {:?}", plugin_dir);
    }

    let cargo_toml_content =
        std::fs::read_to_string(&cargo_toml_path).context("Failed to read Cargo.toml")?;
    let cargo_toml: CargoToml =
        toml::from_str(&cargo_toml_content).context("Failed to parse Cargo.toml")?;

    let plugin_name = cargo_toml.package.name;
    tracing::info!("Found plugin: {}", plugin_name);

    tracing::info!("Building plugin...");
    let status = Command::new("cargo")
        .arg("build")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--release")
        .current_dir(&plugin_dir)
        .status()
        .context("Failed to run cargo build")?;

    if !status.success() {
        bail!("Cargo build failed with status {:?}", status);
    }

    let wasm_file = plugin_dir
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(format!("{}.wasm", plugin_name.replace("-", "_")));

    if !wasm_file.exists() {
        bail!("Compiled Wasm file not found at {:?}", wasm_file);
    }

    // Look for manifest.json
    let mut manifest_file = plugin_dir.join("manifest.json");
    if !manifest_file.exists() {
        // Fallback to old name
        manifest_file = plugin_dir.join(format!("{}.json", plugin_name));
        if !manifest_file.exists() {
            bail!("manifest.json not found in plugin root.");
        }
    }

    // Validate manifest
    let manifest_content =
        std::fs::read_to_string(&manifest_file).context("Failed to read manifest.json")?;
    let mut manifest = serde_json::from_str::<PluginManifest>(&manifest_content)
        .context("Invalid manifest.json format")?;

    if manifest.archived.unwrap_or(false) && manifest.archived_date.is_none() {
        manifest.archived_date = Some(chrono::Utc::now().format("%Y-%m-%d").to_string());
        let new_manifest_content = serde_json::to_string_pretty(&manifest)
            .context("Failed to serialize updated manifest")?;
        std::fs::write(&manifest_file, &new_manifest_content)
            .context("Failed to write updated manifest.json back")?;
    }

    // What the module needs from the host is read from the module itself, so the manifest and
    // repository index can't disagree with it.
    let mut wasm_content = Vec::new();
    File::open(&wasm_file)
        .context("Failed to open Wasm file")?
        .read_to_end(&mut wasm_content)
        .context("Failed to read wasm content")?;
    let interface = WasmInterface::read(&wasm_content)
        .with_context(|| format!("Failed to read {}", wasm_file.display()))?;
    let unknown_imports = interface.unknown_imports();
    if !unknown_imports.is_empty() {
        bail!(
            "The plugin imports functions no Ito host provides: {}",
            unknown_imports.join(", ")
        );
    }
    manifest.abi = Some(interface.abi());
    if let Some(kinds) = interface.kinds()? {
        manifest.set_kinds(&kinds);
    } else if !crate::wasm::KINDS.contains(&manifest.plugin_type.as_str()) {
        bail!(
            "Manifest type \"{}\" must be one of {:?}",
            manifest.plugin_type,
            crate::wasm::KINDS
        );
    }
    manifest.imports = Some(interface.import_names());
    manifest.sdk = sdk_version(&plugin_dir, &plugin_name)?;

    let final_manifest_content =
        serde_json::to_string(&manifest).context("Failed to serialize manifest")?;

    let output_file = plugin_dir.join(format!("{}.ito", plugin_name));
    tracing::info!("Creating package: {:?}", output_file);

    let file = File::create(&output_file).context("Failed to create output file")?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("main.wasm", options)
        .context("Failed to start main.wasm in zip")?;
    zip.write_all(&wasm_content)
        .context("Failed to write wasm content to zip")?;

    zip.start_file("manifest.json", options)
        .context("Failed to start manifest.json in zip")?;
    zip.write_all(final_manifest_content.as_bytes())
        .context("Failed to write manifest content to zip")?;

    let icon_file = plugin_dir.join("icon.png");
    if icon_file.exists() {
        zip.start_file("icon.png", options)
            .context("Failed to start icon.png in zip")?;
        let mut icon_content = Vec::new();
        File::open(&icon_file)
            .context("Failed to open icon.png")?
            .read_to_end(&mut icon_content)
            .context("Failed to read icon content")?;
        zip.write_all(&icon_content)
            .context("Failed to write icon content to zip")?;
    }

    zip.finish().context("Failed to finish zip file")?;

    tracing::info!(
        "Successfully packaged plugin into {}!",
        output_file.display()
    );
    Ok(())
}

/// The ito-rs version the plugin was built with, from the nearest Cargo.lock: "ito-rs 0.1.0" for a
/// registry release, "ito-rs 0.1.0 (39749ae)" for a git dependency, "ito-rs 0.1.0 (path)" for a
/// local checkout. The plugin's own dependency entry picks the ito-rs when the lock has several.
/// None when there's no lock or no ito-rs.
fn sdk_version(plugin_dir: &Path, plugin_name: &str) -> Result<Option<String>> {
    let Some(lock_path) = plugin_dir
        .ancestors()
        .map(|dir| dir.join("Cargo.lock"))
        .find(|path| path.exists())
    else {
        return Ok(None);
    };
    let lock_content = std::fs::read_to_string(&lock_path).context("Failed to read Cargo.lock")?;
    let lock: CargoLock = toml::from_str(&lock_content).context("Failed to parse Cargo.lock")?;

    // A dependency entry is "ito-rs", "ito-rs 0.1.0" or "ito-rs 0.1.0 (source)", as specific as
    // the lock needs it to be to tell several apart.
    let dependency = lock
        .package
        .iter()
        .find(|package| package.name == plugin_name)
        .and_then(|plugin| {
            plugin
                .dependencies
                .iter()
                .find(|entry| entry.split(' ').next() == Some("ito-rs"))
        });
    let mut wanted = dependency
        .map(|entry| entry.splitn(3, ' ').skip(1))
        .into_iter()
        .flatten();
    let wanted_version = wanted.next();
    let wanted_source = wanted
        .next()
        .map(|source| source.trim_matches(|c| c == '(' || c == ')'));

    Ok(lock
        .package
        .iter()
        .filter(|package| package.name == "ito-rs")
        .find(|package| {
            wanted_version.is_none_or(|version| version == package.version)
                && wanted_source.is_none_or(|source| package.source.as_deref() == Some(source))
        })
        .map(|package| {
            let origin = match package.source.as_deref() {
                None => "path".to_string(),
                Some(source) => match source.split_once('#') {
                    Some((_, revision)) => revision.chars().take(7).collect(),
                    None => return format!("ito-rs {}", package.version),
                },
            };
            format!("ito-rs {} ({origin})", package.version)
        }))
}
