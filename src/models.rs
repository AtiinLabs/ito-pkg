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
    /// The plugin's primary kind, "manga", "anime" or "novel". Older hosts read only this.
    #[serde(rename = "type")]
    pub plugin_type: String,
    /// Every kind the plugin serves. Written by `pack` from the module's `ito_supported_kinds`
    /// (ABI 2 and later); hosts read `type` alone without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
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

impl PluginManifest {
    /// Records the kinds the module serves. `type` stays the author's primary kind when it is one
    /// of them, and becomes the first otherwise.
    pub fn set_kinds(&mut self, kinds: &[&str]) {
        if !kinds.contains(&self.plugin_type.as_str())
            && let Some(first) = kinds.first()
        {
            tracing::info!(
                "Manifest type \"{}\" is not a kind the plugin serves; using \"{first}\".",
                self.plugin_type
            );
            self.plugin_type = (*first).to_string();
        }
        self.kinds = Some(kinds.iter().map(|kind| kind.to_string()).collect());
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
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

    #[test]
    fn kinds_keep_the_authors_primary_kind_when_it_is_served() -> Result<(), serde_json::Error> {
        let json = r#"{"id": "p", "name": "P", "version": "1.0.0", "min_app_version": "1.0.0", "type": "novel"}"#;
        let mut manifest: PluginManifest = serde_json::from_str(json)?;
        assert!(!serde_json::to_string(&manifest)?.contains("kinds"));

        manifest.set_kinds(&["manga", "novel"]);
        assert_eq!(manifest.plugin_type, "novel");
        assert_eq!(
            manifest.kinds.as_deref(),
            Some(&["manga".to_string(), "novel".to_string()][..])
        );

        manifest.set_kinds(&["anime"]);
        assert_eq!(manifest.plugin_type, "anime");
        let written: serde_json::Value = serde_json::from_str(&serde_json::to_string(&manifest)?)?;
        assert_eq!(written["type"], "anime");
        assert_eq!(written["kinds"], serde_json::json!(["anime"]));
        Ok(())
    }
}
