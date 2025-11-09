// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use anyhow::Context;
use clap::Parser;
use std::path::{Path, PathBuf};

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

pub struct WorkDir(WorkDirInner);

enum WorkDirInner {
    Temporary(tempfile::TempDir),
    Permanent(PathBuf),
}

impl WorkDir {
    pub fn path(&self) -> &Path {
        match &self.0 {
            WorkDirInner::Temporary(temp_dir) => temp_dir.path(),
            WorkDirInner::Permanent(path_buf) => path_buf,
        }
    }
}

impl Cli {
    pub fn work_directory(&self) -> anyhow::Result<WorkDir> {
        if let Some(path) = &self.work_dir {
            let path = std::env::current_dir()
                .context("Could not find the current directory")?
                .join(path);
            std::fs::create_dir_all(&path).context("Could not create work directory")?;
            Ok(WorkDir(WorkDirInner::Permanent(
                std::fs::canonicalize(path).context("Failed to canonicalize work dir")?,
            )))
        } else {
            let mut inner = tempfile::Builder::new()
                .prefix("octoconda.")
                .tempdir()
                .context("Failed to create temporary directory")?;
            if self.keep_temporary_data {
                inner.disable_cleanup(true);
            }

            Ok(WorkDir(WorkDirInner::Temporary(inner)))
        }
    }
}

pub fn parse_cli() -> Cli {
    Cli::parse()
}
