use crate::models::PluginManifest;
use crate::wasm::WasmInterface;
use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::PathBuf;

pub fn verify_plugin(path: PathBuf, expected_abi: Option<u32>) -> Result<()> {
    if !path.exists() {
        bail!("File not found: {:?}", path);
    }

    let file = File::open(&path).context("Failed to open file")?;
    verify_archive(file, expected_abi)
}

fn verify_archive<R: Read + Seek>(reader: R, expected_abi: Option<u32>) -> Result<()> {
    let mut archive = zip::ZipArchive::new(reader).context("Failed to read as ZIP archive")?;

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
    let interface = WasmInterface::inspect(&wasm, expected_abi).context("Verification failed")?;
    let Some(abi) = manifest.abi else {
        bail!("Verification failed: the manifest doesn't record an abi");
    };
    if abi != interface.abi() {
        bail!(
            "Verification failed: manifest says ABI {abi}, main.wasm is ABI {}",
            interface.abi()
        );
    }
    let Some(imports) = &manifest.imports else {
        bail!("Verification failed: the manifest doesn't record its imports");
    };
    if *imports != interface.import_names() {
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

    fn manifest(plugin_type: &str, kinds: Option<&[&str]>) -> Result<PluginManifest> {
        let mut manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "id": "p", "name": "P", "version": "1.0.0", "min_app_version": "1.0.0", "type": plugin_type
        }))?;
        manifest.kinds = kinds.map(|kinds| kinds.iter().map(|kind| kind.to_string()).collect());
        Ok(manifest)
    }

    #[test]
    fn an_abi_2_manifest_must_list_exactly_the_modules_kinds() -> Result<()> {
        let served = Some(vec!["manga", "novel"]);
        assert!(
            check_kinds(
                &manifest("novel", Some(&["manga", "novel"]))?,
                served.clone()
            )
            .is_ok()
        );
        assert!(check_kinds(&manifest("novel", Some(&["novel"]))?, served.clone()).is_err());
        assert!(check_kinds(&manifest("novel", None)?, served.clone()).is_err());
        assert!(check_kinds(&manifest("anime", Some(&["manga", "novel"]))?, served).is_err());
        Ok(())
    }

    #[test]
    fn an_older_manifest_names_one_known_kind_and_no_kinds_list() -> Result<()> {
        assert!(check_kinds(&manifest("anime", None)?, None).is_ok());
        assert!(check_kinds(&manifest("Manga", None)?, None).is_err());
        assert!(check_kinds(&manifest("manga", Some(&["manga"]))?, None).is_err());
        Ok(())
    }

    const WASM: &str = r#"
        (import "ito:core/net" "fetch_v2" (func (param i32 i32 i32 i32) (result i32)))
        (func (export "ito_abi_version") (result i32) i32.const 3)
        (func (export "ito_supported_kinds") (result i32) i32.const 1)
    "#;

    fn manifest_json() -> serde_json::Value {
        serde_json::json!({
            "id": "p", "name": "P", "version": "1.0.0", "min_app_version": "1.5.0",
            "type": "manga", "kinds": ["manga"], "abi": 3, "imports": ["ito:core/net.fetch_v2"]
        })
    }

    fn without(key: &str) -> serde_json::Value {
        let mut manifest = manifest_json();
        if let Some(fields) = manifest.as_object_mut() {
            fields.remove(key);
        }
        manifest
    }

    /// An in-memory .ito with the given manifest and, unless `None`, the module's text.
    fn package(
        manifest: &serde_json::Value,
        wasm: Option<&str>,
    ) -> Result<std::io::Cursor<Vec<u8>>> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        if let Some(wasm) = wasm {
            zip.start_file("main.wasm", options)?;
            zip.write_all(&wat::parse_str(format!("(module {wasm})"))?)?;
        }
        zip.start_file("manifest.json", options)?;
        zip.write_all(manifest.to_string().as_bytes())?;
        Ok(zip.finish()?)
    }

    /// The refusal `verify_archive` gives, with its causes.
    fn refusal(
        manifest: &serde_json::Value,
        wasm: &str,
        expected_abi: Option<u32>,
    ) -> Result<String> {
        match verify_archive(package(manifest, Some(wasm))?, expected_abi) {
            Ok(()) => bail!("expected a refusal"),
            Err(error) => Ok(format!("{error:#}")),
        }
    }

    #[test]
    fn a_package_matching_its_module_verifies() -> Result<()> {
        verify_archive(package(&manifest_json(), Some(WASM))?, None)?;
        verify_archive(package(&manifest_json(), Some(WASM))?, Some(3))?;
        Ok(())
    }

    #[test]
    fn a_package_for_another_abi_than_expected_is_refused() -> Result<()> {
        let message = refusal(&manifest_json(), WASM, Some(2))?;
        assert!(
            message.contains("ABI 3") && message.contains("ABI 2"),
            "{message}"
        );
        Ok(())
    }

    #[test]
    fn the_manifest_must_record_the_modules_abi() -> Result<()> {
        let missing = without("abi");
        assert!(refusal(&missing, WASM, None)?.contains("abi"));

        let mut wrong = manifest_json();
        wrong["abi"] = 2.into();
        assert!(refusal(&wrong, WASM, None)?.contains("manifest says ABI 2"));
        Ok(())
    }

    #[test]
    fn the_manifest_must_record_the_modules_imports() -> Result<()> {
        let missing = without("imports");
        assert!(refusal(&missing, WASM, None)?.contains("imports"));

        let mut wrong = manifest_json();
        wrong["imports"] = serde_json::json!(["ito:core/net.fetch"]);
        assert!(refusal(&wrong, WASM, None)?.contains("don't match"));

        let mut extra = manifest_json();
        extra["imports"] = serde_json::json!(["ito:core/html.parse", "ito:core/net.fetch_v2"]);
        assert!(refusal(&extra, WASM, None)?.contains("don't match"));
        Ok(())
    }

    #[test]
    fn the_manifest_must_record_the_modules_kinds() -> Result<()> {
        let mut wrong = manifest_json();
        wrong["kinds"] = serde_json::json!(["manga", "novel"]);
        assert!(refusal(&wrong, WASM, None)?.contains("kinds"));

        let missing = without("kinds");
        assert!(refusal(&missing, WASM, None)?.contains("kinds"));
        Ok(())
    }

    #[test]
    fn a_module_with_a_start_section_is_refused_even_when_the_manifest_matches() -> Result<()> {
        let wasm = format!("{WASM} (func $init) (start $init)");
        let message = refusal(&manifest_json(), &wasm, None)?;
        assert!(message.contains("start section"), "{message}");
        Ok(())
    }

    #[test]
    fn an_import_no_host_provides_is_refused_even_when_the_manifest_lists_it() -> Result<()> {
        let wasm = format!(
            r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))) {WASM}"#
        );
        let mut manifest = manifest_json();
        manifest["imports"] =
            serde_json::json!(["ito:core/net.fetch_v2", "wasi_snapshot_preview1.fd_write"]);
        assert!(refusal(&manifest, &wasm, None)?.contains("wasi_snapshot_preview1.fd_write"));
        Ok(())
    }

    #[test]
    fn a_package_without_its_module_is_refused() -> Result<()> {
        assert!(verify_archive(package(&manifest_json(), None)?, None).is_err());
        Ok(())
    }
}
