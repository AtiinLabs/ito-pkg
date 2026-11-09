use anyhow::{Context, Result, bail};
use minijinja::Environment;
use rust_embed::RustEmbed;
use std::process::Command;

#[derive(RustEmbed)]
#[folder = "templates/"]
struct Assets;

pub fn scaffold_plugin(name: &str, plugin_type: &str) -> Result<()> {
    let valid_types = ["manga", "anime", "novel"];
    let plugin_type = plugin_type.to_lowercase();

    if !valid_types.contains(&plugin_type.as_str()) {
        bail!(
            "Invalid plugin type '{}'. Valid types are: manga, anime, novel.",
            plugin_type
        );
    }

    let project_dir = std::env::current_dir()
        .context("Failed to get current directory")?
        .join(name);

    if project_dir.exists() {
        bail!("Directory '{}' already exists.", name);
    }

    // Initialize minijinja environment
    let mut env = Environment::new();
    for file in Assets::iter() {
        let content = Assets::get(&file).context("Failed to get asset")?;
        let content_str = std::str::from_utf8(&content.data).context("Asset is not valid UTF-8")?;
        env.add_template_owned(file.into_owned(), content_str.to_string())
            .context("Failed to add template")?;
    }

    // Use cargo to create a base lib project
    tracing::info!("Scaffolding {} plugin: {}...", plugin_type, name);
    let status = Command::new("cargo")
        .arg("new")
        .arg("--lib")
        .arg(name)
        .status()
        .context("Failed to run cargo new")?;

    if !status.success() {
        bail!("cargo new failed");
    }

    // Render and write Cargo.toml
    let cargo_toml_template = env
        .get_template("common/Cargo.toml.j2")
        .context("Failed to get Cargo.toml template")?;
    let cargo_toml_rendered = cargo_toml_template
        .render(minijinja::context! {
            name => name,
        })
        .context("Failed to render Cargo.toml")?;

    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml_rendered)
        .context("Failed to write Cargo.toml")?;

    // Render and write manifest.json
    let manifest_template = env
        .get_template("common/manifest.json.j2")
        .context("Failed to get manifest.json template")?;
    let manifest_rendered = manifest_template
        .render(minijinja::context! {
            name => name,
            plugin_type => plugin_type,
        })
        .context("Failed to render manifest.json")?;

    std::fs::write(project_dir.join("manifest.json"), manifest_rendered)
        .context("Failed to write manifest.json")?;

    // Render and write lib.rs
    let lib_rs_path = format!("{}/lib.rs.j2", plugin_type);
    let lib_rs_template = env
        .get_template(&lib_rs_path)
        .context(format!("Failed to get {} template", lib_rs_path))?;
    // lib.rs currently doesn't use variables, but we render it anyway for consistency
    let lib_rs_rendered = lib_rs_template
        .render(minijinja::context! {})
        .context("Failed to render lib.rs")?;

    std::fs::write(project_dir.join("src/lib.rs"), lib_rs_rendered)
        .context("Failed to write src/lib.rs")?;

    // `pack` builds with --locked, so a new plugin needs its lock file from the start.
    let status = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(&project_dir)
        .status()
        .context("Failed to run cargo generate-lockfile")?;
    if !status.success() {
        bail!(
            "Scaffolded {name}, but `cargo generate-lockfile` failed; run it in the project \
             directory before `ito-pkg pack`"
        );
    }

    tracing::info!(
        "Plugin {} scaffolded successfully at {:?}",
        name,
        project_dir
    );
    tracing::info!(
        "\nNext steps:\n  cd {}\n  git add Cargo.lock  # `ito-pkg pack` builds with --locked\n  ito-pkg pack",
        name
    );

    Ok(())
}
