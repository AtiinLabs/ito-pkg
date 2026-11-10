use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// The environment variable that supplies `--expect-abi` when the flag is absent.
const EXPECTED_ABI_ENV: &str = "ITO_EXPECTED_ABI";

/// The ABI to require: the flag if given, else `ITO_EXPECTED_ABI`. An unset or empty variable
/// means no requirement, so a workflow can write `ITO_EXPECTED_ABI: ${VAR:+...}` safely.
pub fn expected_abi(flag: Option<u32>) -> Result<Option<u32>> {
    resolve_expected_abi(flag, std::env::var(EXPECTED_ABI_ENV).ok().as_deref())
}

fn resolve_expected_abi(flag: Option<u32>, env: Option<&str>) -> Result<Option<u32>> {
    if flag.is_some() {
        return Ok(flag);
    }
    match env.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => value
            .parse()
            .map(Some)
            .with_context(|| format!("{EXPECTED_ABI_ENV} must be an ABI number, got {value:?}")),
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Build and package a plugin into an .ito file
    Pack {
        /// Path to the plugin directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
        /// Fail unless the module is built for this ABI version [env: ITO_EXPECTED_ABI]
        #[arg(long, value_name = "ABI")]
        expect_abi: Option<u32>,
    },
    /// Verify an .ito plugin
    Verify {
        /// Path to the .ito file
        path: PathBuf,
        /// Fail unless the module is built for this ABI version [env: ITO_EXPECTED_ABI]
        #[arg(long, value_name = "ABI")]
        expect_abi: Option<u32>,
    },
    /// Build a static repository from a folder of .ito files
    Repo {
        /// Path to the directory containing .ito files
        #[arg(short, long, default_value = ".")]
        input: PathBuf,
        /// Path to the output directory for the repository
        #[arg(short, long, default_value = "public")]
        output: PathBuf,
        /// Name of the repository
        #[arg(long, default_value = "Ito Repository")]
        name: String,
        /// URL of the repository
        #[arg(long)]
        url: String,
        /// Description of the repository
        #[arg(
            long,
            default_value = "Community sourced and maintained plugins for Ito."
        )]
        description: String,
    },
    /// Serve a directory over HTTP (for dev usage)
    Serve {
        /// Path to the directory to serve
        #[arg(long, default_value = "public")]
        path: PathBuf,
        /// Port to bind to
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    /// Scaffold a new plugin
    New {
        /// Name of the plugin project (e.g. 'my-novel-plugin')
        #[arg(required = true)]
        name: String,
        /// Type of the plugin: manga, anime, or novel
        #[arg(short = 't', long = "type", default_value = "manga")]
        plugin_type: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_or_empty_variable_means_no_expectation() -> Result<()> {
        assert_eq!(resolve_expected_abi(None, None)?, None);
        assert_eq!(resolve_expected_abi(None, Some(""))?, None);
        assert_eq!(resolve_expected_abi(None, Some("  "))?, None);
        Ok(())
    }

    #[test]
    fn the_variable_supplies_the_abi_and_the_flag_wins() -> Result<()> {
        assert_eq!(resolve_expected_abi(None, Some("3"))?, Some(3));
        assert_eq!(resolve_expected_abi(Some(2), Some("3"))?, Some(2));
        // The flag decides, so a bad variable doesn't matter once it is given.
        assert_eq!(resolve_expected_abi(Some(2), Some("x"))?, Some(2));
        Ok(())
    }

    #[test]
    fn a_non_numeric_variable_is_an_error_naming_it() {
        let error = resolve_expected_abi(None, Some("three")).err();
        assert!(
            error.is_some_and(|e| e.to_string().contains("ITO_EXPECTED_ABI")),
            "expected an error naming the variable"
        );
    }
}
