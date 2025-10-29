// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::path::PathBuf;

use clap::Parser;

#[derive(Clone, Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(long)]
    pub conda_channel: String,
    #[arg(long, default_value = "./config.toml")]
    pub config_file: PathBuf,
    #[arg(long)]
    pub work_dir: Option<PathBuf>,
}

pub fn parse_cli() -> Cli {
    Cli::parse()
}
