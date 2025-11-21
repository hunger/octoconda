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
#[serde(untagged)]
pub enum StringOrList {
    String(String),
    List(Vec<String>),
}

#[derive(Deserialize)]
pub struct TomlPackage {
    pub name: Option<String>,
    pub repository: String,
    pub platforms: Option<HashMap<Platform, StringOrList>>,
}

#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,
    pub repository: Repository,
    pub platforms: HashMap<Platform, Vec<regex::Regex>>,
}

fn default_platforms() -> HashMap<Platform, Vec<String>> {
    HashMap::from([
        (
            Platform::Linux32,
            vec![
                "[\\.-]i686-(unknown-)?linux-musl(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                    .to_string(),
                "[\\.-]i686-(unknown-)?linux(-gnu)?(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                    .to_string(),
                "[\\.-]linux-(i686|x86)(-unknown)?(-gnu|-musl)?(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                    .to_string(),
            ],
        ),
        (
            Platform::Linux64,
            vec![
            "[\\.-](x86_64|amd64)-(unknown-)?linux-musl(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
            "[\\.-](x86_64|amd64)-(unknown-)?linux(-gnu)?(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
            "[\\.-]linux-(x86_64|amd64)(-unknown)?(-gnu|-musl)?(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
            ],
        ),
        (
            Platform::LinuxAarch64,
            vec![
            "[\\.-](arm64|aarch64)-(unknown-)?linux-musl(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
            "[\\.-](arm64|aarch64)-(unknown-)?linux(-gnu)?(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
            "[\\.-]linux-(arm64|aarch64|arm64)-(unknown-)?(-gnu|-musl)?(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                .to_string(),
        ],
        ),
        (
            Platform::Osx64,
            vec![
                "[\\.-]x86_64-(apple-)?darwin(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                    .to_string(),
                "[\\.-]darwin-(amd64|x86_64)(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                    .to_string(),
            ],
        ),
        (
            Platform::OsxArm64,
            vec![
                "[\\.-](arm64|aarch64)-(apple-)?darwin(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                    .to_string(),
                "[\\.-]darwin-(arm64|aarch64)(\\.tar\\.gz|\\.tar\\.xz|\\.tgz|\\.txz|\\.zip)?$"
                    .to_string(),
            ],
        ),
        (
            Platform::Win32,
            vec![
                "[\\.-](x86|i686)-(pc)?-windows(-msvc)?(\\.zip)?$".to_string(),
                "[\\.-]windows-(i686|x86)(\\.zip)?$".to_string(),
            ],
        ),
        (
            Platform::Win64,
            vec![
                "[\\.-](amd_64|x86_64)-(pc)?-windows(-msvc)?(\\.zip)?$".to_string(),
                "[\\.-]windows-(amd64|x86_64)(\\.zip)?$".to_string(),
            ],
        ),
        (
            Platform::WinArm64,
            vec![
                "[\\.-](arm64|aarch64)(-pc)?-windows(-msvc)?(\\.zip)?$".to_string(),
                "[\\.-]windows-(arm64|aarch64)(\\.zip)?$".to_string(),
            ],
        ),
    ])
}

impl TryFrom<TomlPackage> for Package {
    type Error = anyhow::Error;

    fn try_from(value: TomlPackage) -> Result<Self, Self::Error> {
        let repository = Repository::try_from(value.repository.as_str())?;
        let name = value
            .name
            .clone()
            .unwrap_or_else(|| repository.repo.clone());

        let n = &value.name;

        let platforms = {
            let mut result = default_platforms();
            for (k, v) in value.platforms.unwrap_or_default().drain() {
                let strings = match v {
                    StringOrList::String(s) => {
                        if s == "null" {
                            result.remove(&k);
                            continue;
                        }

                        if let Some(n) = n.as_ref() {
                            let Some(current) = result.get(&k) else {
                                return Err(anyhow::anyhow!(format!(
                                    "Can not prepend to default platform key {k}"
                                )));
                            };
                            result.insert(
                                k,
                                current
                                    .iter()
                                    .map(|c| {
                                        let mut r = n.to_string();
                                        r.push_str(&format!(".*{c}"));
                                        r
                                    })
                                    .collect::<Vec<_>>(),
                            );
                            continue;
                        }

                        vec![s]
                    }
                    StringOrList::List(items) => items,
                };
                result.insert(k, strings);
            }

            result
                .drain()
                .map(|(k, v)| {
                    let re = v
                        .iter()
                        .map(|r| {
                            let pattern = if let Some(n) = n {
                                format!("^{n}.*{r}")
                            } else {
                                r.to_string()
                            };
                            regex::Regex::new(&pattern)
                                .context(format!("failed to parse regex for platform {k}"))
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
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
