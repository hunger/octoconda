// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::types::Repository;

fn deserialize_repository<'de, D>(deserializer: D) -> Result<Repository, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Repository::try_from(s.as_str()).map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, Deserialize)]
pub struct Package {
    pub name: Option<String>,
    #[serde(deserialize_with = "deserialize_repository")]
    pub repository: Repository,
}

impl Package {
    pub fn name(&self) -> &str {
        if let Some(name) = self.name.as_ref() {
            name
        } else {
            &self.repository.repo
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Config {
    pub packages: Vec<Package>,
}

pub fn parse_config(path: &Path) -> Result<Config, anyhow::Error> {
    let contents = std::fs::read_to_string(path).context(format!(
        "Failed to read configuration file {}",
        path.display()
    ))?;
    let config = toml::from_str(&contents).context(format!(
        "Failed to parse configuration file {}",
        path.display()
    ))?;
    Ok(config)
}
