use crate::models::{PluginManifest, RepoIndex, RepoPackage};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The manifest of an `.ito` file, or None if it can't be read; the build below warns about those.
fn read_manifest(path: &Path) -> Option<PluginManifest> {
    let mut archive = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
    let mut manifest_file = archive.by_name("manifest.json").ok()?;
    let mut content = String::new();
    manifest_file.read_to_string(&mut content).ok()?;
    serde_json::from_str(&content).ok()
}

/// Fails if any package in `input` has a manifest without an `abi`.
fn require_abi_in_every_package(input: &Path) -> Result<()> {
    for entry in std::fs::read_dir(input).context("Failed to read input directory")? {
        let path = entry.context("Failed to read directory entry")?.path();
        if path.extension().is_some_and(|ext| ext == "ito")
            && read_manifest(&path).is_some_and(|manifest| manifest.abi.is_none())
        {
            bail!("{path:?} has a manifest without an abi; build it with `ito-pkg pack`");
        }
    }
    Ok(())
}

pub fn build_repo(
    input: PathBuf,
    output: PathBuf,
    name: &str,
    url: &str,
    description: &str,
) -> Result<()> {
    // Refuse before writing anything, so a refusal leaves no partial repository.
    require_abi_in_every_package(&input)?;

    if !output.exists() {
        std::fs::create_dir_all(&output).context("Failed to create output directory")?;
    }

    let packages_dir = output.join("packages");
    let icons_dir = output.join("icons");
    std::fs::create_dir_all(&packages_dir).context("Failed to create packages directory")?;
    std::fs::create_dir_all(&icons_dir).context("Failed to create icons directory")?;

    let mut repo_packages = Vec::new();

    let entries = std::fs::read_dir(&input).context("Failed to read input directory")?;
    for entry in entries {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "ito") {
            tracing::info!("Processing {:?}", path);

            let mut file = File::open(&path).context("Failed to open .ito file")?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)
                .context("Failed to read .ito file")?;

            // Hash the .ito file
            let mut hasher = Sha256::new();
            hasher.update(&buffer);
            let result = hasher.finalize();
            let sha256_hash = hex::encode(result);

            // Extract manifest and icon
            let file_for_zip = File::open(&path).context("Failed to reopen .ito file for ZIP")?;
            let mut archive = match zip::ZipArchive::new(file_for_zip) {
                Ok(a) => a,
                Err(_) => {
                    tracing::warn!("Could not read {:?} as ZIP.", path);
                    continue;
                }
            };

            let manifest: PluginManifest = {
                let mut manifest_file = match archive.by_name("manifest.json") {
                    Ok(f) => f,
                    Err(_) => {
                        tracing::warn!("manifest.json not found in {:?}", path);
                        continue;
                    }
                };
                let mut s = String::new();
                manifest_file
                    .read_to_string(&mut s)
                    .context("Failed to read manifest.json")?;
                match serde_json::from_str(&s) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("Invalid manifest in {:?}: {}", path, e);
                        continue;
                    }
                }
            };

            let pkg_filename = format!("{}-v{}.ito", manifest.id, manifest.version);
            let dest_pkg_path = packages_dir.join(&pkg_filename);
            std::fs::copy(&path, &dest_pkg_path).context("Failed to copy .ito file")?;

            let mut icon_url = None;
            if let Ok(mut icon_file) = archive.by_name("icon.png") {
                let icon_filename = format!("{}-v{}.png", manifest.id, manifest.version);
                let dest_icon_path = icons_dir.join(&icon_filename);
                let mut out_icon =
                    File::create(&dest_icon_path).context("Failed to create icon file")?;
                std::io::copy(&mut icon_file, &mut out_icon)
                    .context("Failed to copy icon content")?;
                icon_url = Some(format!("icons/{}", icon_filename));
            }

            repo_packages.push(RepoPackage {
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                min_app_version: manifest.min_app_version,
                download_url: format!("packages/{}", pkg_filename),
                icon_url,
                sha256: sha256_hash,
                plugin_type: manifest.plugin_type.clone(),
                kinds: manifest.kinds.clone(),
                abi: manifest.abi,
                sdk: manifest.sdk.clone(),
                imports: manifest.imports.clone(),
                archived: manifest.archived,
                archived_reason: manifest.archived_reason.clone(),
                archived_date: manifest.archived_date.clone(),
            });
        }
    }

    let index = RepoIndex {
        repo_name: name.to_string(),
        repo_url: url.to_string(),
        description: description.to_string(),
        packages: repo_packages,
    };

    let index_path = output.join("index.json");
    let mut index_file = File::create(&index_path).context("Failed to create index.json")?;
    let index_json = serde_json::to_string_pretty(&index).context("Failed to serialize index")?;
    index_file
        .write_all(index_json.as_bytes())
        .context("Failed to write index.json")?;

    let min_index_path = output.join("index.min.json");
    let mut min_index_file =
        File::create(&min_index_path).context("Failed to create index.min.json")?;
    let min_index_json = serde_json::to_string(&index).context("Failed to serialize min index")?;
    min_index_file
        .write_all(min_index_json.as_bytes())
        .context("Failed to write index.min.json")?;

    tracing::info!("Repository built successfully at {:?}", output);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory removed on drop, unique per test and process.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Result<Self> {
            let dir = std::env::temp_dir().join(format!("ito-pkg-{}-{name}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes `<dir>/<file>.ito` holding only the manifest, which is all `repo` reads besides the
    /// icon.
    fn write_package(dir: &Path, file: &str, manifest: &serde_json::Value) -> Result<()> {
        let mut zip = zip::ZipWriter::new(File::create(dir.join(format!("{file}.ito")))?);
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())?;
        zip.write_all(manifest.to_string().as_bytes())?;
        zip.finish()?;
        Ok(())
    }

    fn manifest(id: &str, abi: Option<u32>) -> serde_json::Value {
        let mut manifest = serde_json::json!({
            "id": id, "name": "P", "version": "1.0.0", "min_app_version": "1.5.0",
            "type": "manga", "kinds": ["manga"], "imports": []
        });
        if let (Some(abi), Some(fields)) = (abi, manifest.as_object_mut()) {
            fields.insert("abi".to_string(), abi.into());
        }
        manifest
    }

    #[test]
    fn a_package_without_an_abi_is_refused_and_not_published() -> Result<()> {
        let scratch = Scratch::new("repo-no-abi")?;
        let (input, output) = (scratch.0.join("in"), scratch.0.join("out"));
        std::fs::create_dir_all(&input)?;
        write_package(&input, "p", &manifest("p", None))?;

        let error = build_repo(input, output.clone(), "R", "https://example.test", "d")
            .err()
            .map(|error| error.to_string());

        assert!(
            error.is_some_and(|message| message.contains("abi")),
            "expected a refusal naming the missing abi"
        );
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn one_package_without_an_abi_leaves_nothing_written_for_the_others() -> Result<()> {
        let scratch = Scratch::new("repo-mixed")?;
        let (input, output) = (scratch.0.join("in"), scratch.0.join("out"));
        std::fs::create_dir_all(&input)?;
        write_package(&input, "a", &manifest("a", Some(3)))?;
        write_package(&input, "b", &manifest("b", None))?;

        assert!(build_repo(input, output.clone(), "R", "https://example.test", "d").is_err());

        // Whichever order the directory lists them in, the valid package is not copied either.
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn a_package_with_an_abi_is_indexed_with_it() -> Result<()> {
        let scratch = Scratch::new("repo-abi")?;
        let (input, output) = (scratch.0.join("in"), scratch.0.join("out"));
        std::fs::create_dir_all(&input)?;
        write_package(&input, "p", &manifest("p", Some(3)))?;

        build_repo(input, output.clone(), "R", "https://example.test", "d")?;

        let index: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(output.join("index.json"))?)?;
        assert_eq!(index["packages"][0]["abi"], 3);
        Ok(())
    }
}
