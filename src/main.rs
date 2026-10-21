mod cli;
mod commands;
mod models;
mod wasm;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use commands::{new, pack, repo, serve, verify};

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Pack { path } => pack::pack_plugin(path),
        Commands::Verify { path } => verify::verify_plugin(path),
        Commands::Repo {
            input,
            output,
            name,
            url,
            description,
        } => repo::build_repo(input, output, &name, &url, &description),
        Commands::Serve { path, port } => serve::serve_repo(path, port),
        Commands::New { name, plugin_type } => new::scaffold_plugin(&name, &plugin_type),
    }
}
