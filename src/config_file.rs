// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    path::Path,
};

use anyhow::Context;
use rattler_conda_types::Platform;
use serde::Deserialize;

use crate::types::Repository;

#[derive(Deserialize)]
pub struct TomlPackage {
    pub name: Option<String>,
    pub repository: String,
    pub platforms: Option<HashMap<Platform, String>>,
}

#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,
    pub repository: Repository,
    pub platforms: HashMap<Platform, regex::Regex>,
}

fn default_platforms() -> HashMap<Platform, String> {
    HashMap::from([
        (
            Platform::Linux32,
            "[\\.-]i686-(unknown-)?linux-musl(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
        ),
        (
            Platform::Linux64,
            "[\\.-]x86_64-(unknown-)?linux-musl(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
        ),
        (
            Platform::LinuxAarch64,
            "[\\.-]aarch64-(unknown-)?linux-musl(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
        ),
        (
            Platform::Osx64,
            "[\\.-]x86_64-(apple-)?darwin(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
        ),
        (
            Platform::OsxArm64,
            "[\\.-]aarch64-(apple-)?darwin(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
        ),
        (
            Platform::Win32,
            "[\\.-]i686-(pc)?-windows(-msvc)?(\\.zip)?$".to_string(),
        ),
        (
            Platform::Win64,
            "[\\.-]x86_64-(pc)?-windows(-msvc)?(\\.zip)?$".to_string(),
        ),
        (
            Platform::WinArm64,
            "[\\.-]arm64(-pc)?-windows(-msvc)?(\\.zip)?$".to_string(),
        ),
    ])
}

impl TryFrom<TomlPackage> for Package {
    type Error = anyhow::Error;

    fn try_from(value: TomlPackage) -> Result<Self, Self::Error> {
        let repository = Repository::try_from(value.repository.as_str())?;
        let name = value.name.unwrap_or_else(|| repository.repo.clone());
        let platforms = {
            let mut result = default_platforms();
            for (k, v) in value.platforms.unwrap_or_default().drain() {
                if v == "null" {
                    result.remove(&k);
                } else if let Some(v) = v.strip_suffix("+++") {
                    let Some(current) = result.get(&k) else {
                        return Err(anyhow::anyhow!(format!(
                            "Can not prepend to default platform key {k}"
                        )));
                    };
                    let mut v = v.to_string();
                    v.push_str(current);
                    result.insert(k, v);
                } else {
                    result.insert(k, v);
                }
            }
            result
                .drain()
                .map(|(k, v)| {
                    let re = regex::Regex::new(&v)
                        .context(format!("failed to parse regex for platform {k}"))?;
                    Ok((k, re))
                })
                .collect::<anyhow::Result<HashMap<_, _>>>()?
        };

        Ok(Package {
            name,
            repository,
            platforms,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Conda {
    pub channel: String,
}

impl Conda {
    pub fn short_channel(&self) -> anyhow::Result<String> {
        if let Ok(channel_url) = url::Url::parse(&self.channel) {
            if channel_url.host_str() != Some("prefix.dev") {
                return Err(anyhow::anyhow!(
                    "Not a prefix channel, can not generate a channel name from this URL"
                ));
            }
            Ok(channel_url.path().to_string())
        } else {
            Ok(self.channel.clone())
        }
    }

    pub fn full_channel(&self) -> anyhow::Result<String> {
        let short_channel = self.short_channel()?;
        Ok(format!("https://prefix.dev/{short_channel}"))
    }
}

#[derive(serde::Deserialize)]
pub struct TomlConfig {
    pub packages: Vec<TomlPackage>,
    pub conda: Conda,
}

impl TryFrom<TomlConfig> for Config {
    type Error = anyhow::Error;

    fn try_from(mut value: TomlConfig) -> Result<Self, Self::Error> {
        Ok(Config {
            packages: value
                .packages
                .drain(..)
                .map(|tp| tp.try_into())
                .collect::<anyhow::Result<Vec<_>>>()?,
            conda: value.conda,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub packages: Vec<Package>,
    pub conda: Conda,
}

impl Config {
    pub fn all_platforms(&self) -> HashSet<Platform> {
        self.packages
            .iter()
            .flat_map(|p| p.platforms.keys())
            .copied()
            .collect()
    }
}

pub fn parse_config(path: &Path) -> anyhow::Result<Config> {
    let contents = std::fs::read_to_string(path).context(format!(
        "Failed to read configuration file {}",
        path.display()
    ))?;
    let config: TomlConfig = toml::from_str(&contents).context(format!(
        "Failed to parse configuration file {}",
        path.display()
    ))?;

    config.try_into()
}
