use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
pub struct CargoToml {
    pub package: CargoPackage,
}

#[derive(Deserialize, Debug)]
pub struct CargoPackage {
    pub name: String,
}

#[derive(Deserialize, Debug)]
pub struct CargoLock {
    #[serde(default)]
    pub package: Vec<LockedPackage>,
}

#[derive(Deserialize, Debug)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String, // String instead of Int to match semver
    pub min_app_version: String,
    pub url: Option<String>,
    #[serde(rename = "sourceUrl")]
    pub source_url: Option<String>,
    #[serde(rename = "contentRating")]
    pub content_rating: Option<i32>,
    pub nsfw: Option<i32>,
    pub language: Option<String>,
    pub languages: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub plugin_type: String, // "manga" or "anime"
    pub author: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub archived: Option<bool>,
    pub archived_reason: Option<String>,
    pub archived_date: Option<String>,
    /// The ABI the plugin was built for. Written by `pack` from the module's `ito_abi_version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<u32>,
    /// The SDK the plugin was built with, such as "ito-rs 0.1.0". Written by `pack`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk: Option<String>,
    /// Every function the plugin imports, as "module.name". Written by `pack`; hosts compare it
    /// with the functions they provide before downloading the plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imports: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RepoPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub min_app_version: String,
    pub download_url: String,
    pub icon_url: Option<String>,
    pub sha256: String,
    #[serde(rename = "type")]
    pub plugin_type: String,
    pub archived: Option<bool>,
    pub archived_reason: Option<String>,
    pub archived_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imports: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RepoIndex {
    pub repo_name: String,
    pub repo_url: String,
    pub description: String,
    pub packages: Vec<RepoPackage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_manifest_deserialization() -> Result<(), serde_json::Error> {
        let json = r#"{
            "id": "com.example.plugin",
            "name": "Example Plugin",
            "version": "1.0.0",
            "min_app_version": "1.0.0",
            "type": "manga",
            "author": "Me"
        }"#;

        let manifest: PluginManifest = serde_json::from_str(json)?;
        assert_eq!(manifest.id, "com.example.plugin");
        assert_eq!(manifest.name, "Example Plugin");
        assert_eq!(manifest.plugin_type, "manga");
        assert_eq!(manifest.author, Some("Me".to_string()));
        Ok(())
    }
}
