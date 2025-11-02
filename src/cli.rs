// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;

#[derive(Clone, Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(long, default_value = "./config.toml")]
    pub config_file: PathBuf,
    #[arg(long)]
    pub work_dir: Option<PathBuf>,
    #[arg(long, default_value = "false")]
    pub keep_temporary_data: bool,
}

impl Cli {
    pub fn temporary_directory(&self) -> anyhow::Result<tempfile::TempDir> {
        let mut result = if let Some(path) = &self.work_dir {
            tempfile::tempdir_in(path).context(format!(
                "Failed to create temporary directory in {}",
                path.display()
            ))?
        } else {
            tempfile::tempdir().context("Failed to create temporary directory")?
        };

        if self.keep_temporary_data {
            result.disable_cleanup(true);
        }

        Ok(result)
    }
}

pub fn parse_cli() -> Cli {
    Cli::parse()
}
