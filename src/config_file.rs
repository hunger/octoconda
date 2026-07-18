// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    path::Path,
};

use anyhow::{Context, anyhow};
use rattler_conda_types::Platform;
use serde::Deserialize;

use crate::types::Repository;

/// Derive the conda package name from the config: use the explicit name if
/// provided, otherwise fall back to the repository name. The result is
/// lowercased because conda package names are case-insensitive.
pub fn conda_package_name(name: Option<&str>, repo: &str) -> String {
    name.unwrap_or(repo).to_lowercase()
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum StringOrList {
    String(String),
    List(Vec<String>),
}

#[derive(Deserialize)]
pub struct TomlSubPackage {
    pub name: String,
    #[serde(rename = "release-prefix")]
    pub release_prefix: Option<String>,
    #[serde(rename = "tag-prefix")]
    pub tag_prefix: Option<String>,
    pub platforms: Option<HashMap<Platform, StringOrList>>,
    #[serde(default)]
    pub expose: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct TomlPackage {
    pub name: Option<String>,
    #[serde(rename = "release-prefix")]
    pub release_prefix: Option<String>,
    #[serde(rename = "tag-prefix")]
    pub tag_prefix: Option<String>,
    pub repository: String,
    pub platforms: Option<HashMap<Platform, StringOrList>>,
    #[serde(default)]
    pub deprecated: bool,
    pub packages: Option<Vec<TomlSubPackage>>,
    #[serde(default)]
    pub expose: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,
    pub repository: Repository,
    release_prefix: Option<String>,
    pub tag_prefix: Option<String>,
    platform_pattern: HashMap<Platform, Vec<String>>,
    pub expose: Vec<String>,
}

impl Package {
    pub fn platform_pattern(&self) -> anyhow::Result<HashMap<Platform, Vec<regex::Regex>>> {
        self.platform_pattern
            .iter()
            .map(|(k, v)| {
                let re = v
                    .iter()
                    .map(|r| {
                        let pattern = if let Some(rp) = &self.release_prefix {
                            format!("^{rp}([\\._-].+)?[\\._-]{r}")
                        } else {
                            format!("(^|[\\._-]){r}")
                        };

                        regex::RegexBuilder::new(&pattern)
                            .case_insensitive(true)
                            .build()
                            .context(format!("failed to parse regex for platform {k}"))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                Ok((*k, re))
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()
    }
}

const ARCHIVE: &str =
    "\\.tar\\.gz|\\.tar\\.xz|\\.tar\\.bz2|\\.tar\\.zstd?|\\.tgz|\\.txz|\\.tbz|\\.zip";
const COMPRESSED: &str = "\\.gz|\\.xz|\\.zstd?|\\.bz2";
const VERSION: &str = "v?\\d+([\\.[^\\.]+])*";
const VER: &str = "([\\._-]v\\d+([\\.[^\\.]+])*)?";

const X86: &str = "(intel[_-]?32|i?[3-6]86|32[_-]?bit)";
const X64: &str = "(intel[_-]?64|x86[_-]64|amd[_-]?64|x64|64[_-]?bit)";
const ARM: &str = "(arm[_-]?64|aarch[_-]?64)";

const APPLE: &str = "(apple|darwin|mac([\\._-]?os([\\._-]?x)?)?|os[\\._-]?x)";
const WINDOWS: &str = "(windows|win(32|64)?)";

fn default_platforms() -> HashMap<Platform, Vec<String>> {
    fn linux_patterns(arch: &str, width: usize) -> Vec<String> {
        let linux = format!("linux({width})?");
        let extra = format!("([._-]({VERSION}|x11|unknown|gh|bin))*");
        let gnu = "gnu|glibc\\d*";
        let mut result = vec![
            format!(
                "{arch}[\\._-](unknown[\\._-])?{linux}[\\._-]musl{extra}({COMPRESSED}|{ARCHIVE})?$"
            ),
            format!("{arch}[\\._-]musl[\\._-]{linux}{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{linux}[\\._-]{arch}[\\._-]musl{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!(
                "{arch}([\\._-](unknown|{gnu}))?[\\._-]{linux}([\\._-]({gnu}))?{extra}({COMPRESSED}|{ARCHIVE})?$"
            ),
            format!(
                "{linux}([\\._-](unknown|{gnu}))?[\\._-]{arch}([\\._-]({gnu}))?{extra}({COMPRESSED}|{ARCHIVE})?$"
            ),
        ];
        if arch == X64 {
            result.push(format!(
                "{linux}([\\._-](unknown|gnu))?{extra}({COMPRESSED}|{ARCHIVE})?$"
            ));
        }
        if arch == ARM {
            result.push(format!(
                "{linux}[\\._-]arm([\\._-](unknown|gnu))?{extra}({COMPRESSED}|{ARCHIVE})?$"
            ));
        }
        result
    }

    fn mac_patterns(arch: &str) -> Vec<String> {
        let extra = format!("([._-]({VERSION}|unknown|gh|bin))*");
        vec![
            format!("{arch}([\\._-]apple)?[\\._-]{APPLE}-15{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{APPLE}-15[\\._-]{arch}{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{APPLE}-15[\\._-](universal|all){extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{arch}([\\._-]apple)?[\\._-]{APPLE}-14{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{APPLE}-14[\\._-]{arch}{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{APPLE}-14[\\._-](universal|all){extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{arch}([\\._-]apple)?[\\._-]{APPLE}-13{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{APPLE}-13[\\._-]{arch}{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{APPLE}-13[\\._-](universal|all){extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{arch}([\\._-]apple)?[\\._-]{APPLE}{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{APPLE}[\\._-]{arch}{extra}({COMPRESSED}|{ARCHIVE})?$"),
            format!("{APPLE}[\\._-](universal|all){extra}({COMPRESSED}|{ARCHIVE})?$"),
        ]
    }

    fn win_patterns(arch: &str, width: usize) -> Vec<String> {
        let mut result = vec![
            format!(
                "{arch}([\\._-]pc)?[\\._-]{WINDOWS}([\\._-]msvc)?{VER}([\\._-]exe)?({ARCHIVE}|\\.exe)?$"
            ),
            format!(
                "{arch}([\\._-]pc)?[\\._-]{WINDOWS}([\\._-]msvc)?{VER}([\\._-]exe)?({ARCHIVE}|\\.exe)?$"
            ),
            format!("{WINDOWS}([\\._-]msvc)?[\\._-]{arch}{VER}([\\._-]bin)?([\\._-]exe)?({ARCHIVE}|\\.exe)?$"),
            format!(
                "{arch}([\\._-]pc)?[\\._-]{WINDOWS}[\\._-]gnu(llvm)?{VER}([\\._-]exe)?({ARCHIVE}|\\.exe)?$"
            ),
            format!("{arch}.exe"),
        ];
        if arch != ARM {
            result.push(format!("win{width}{VER}([\\._-]exe)?({ARCHIVE}|\\.exe)?$"));
        } else {
            result.push(format!(
                "arm([\\._-]pc)?[\\._-]{WINDOWS}([\\._-]gnu(llvm)?)?{VER}([\\._-]exe)?({ARCHIVE}|\\.exe)?$"
            ));
            result.push(format!(
                "{WINDOWS}([\\._-]msvc)?[\\._-]arm{VER}([\\._-]exe)?({ARCHIVE}|\\.exe)?$"
            ));
        }
        result
    }

    HashMap::from([
        (Platform::Linux32, linux_patterns(X86, 32)),
        (Platform::Linux64, linux_patterns(X64, 64)),
        (Platform::LinuxAarch64, linux_patterns(ARM, 64)),
        (Platform::Osx64, mac_patterns(X64)),
        (Platform::OsxArm64, mac_patterns(ARM)),
        (Platform::Win32, win_patterns(X86, 32)),
        (Platform::Win64, win_patterns(X64, 64)),
        (Platform::WinArm64, win_patterns(ARM, 64)),
    ])
}

fn resolve_platforms(
    overrides: Option<HashMap<Platform, StringOrList>>,
) -> HashMap<Platform, Vec<String>> {
    let mut result = default_platforms();
    for (k, v) in overrides.unwrap_or_default().into_iter() {
        let strings = match v {
            StringOrList::String(s) => {
                if s == "null" {
                    result.remove(&k);
                    continue;
                }
                vec![s]
            }
            StringOrList::List(items) => items,
        };
        result.insert(k, strings);
    }
    result
}

/// Expand a `TomlPackage` into one or more `Package` values.
///
/// When the entry has a `packages` sub-list, each sub-package produces a
/// separate `Package` sharing the same repository. Otherwise a single
/// `Package` is returned (the original behaviour).
fn expand_toml_package(value: TomlPackage) -> anyhow::Result<Vec<Package>> {
    let repository = Repository::try_from(value.repository.as_str())?;

    if let Some(sub_packages) = value.packages {
        if value.name.is_some() || value.release_prefix.is_some() || value.tag_prefix.is_some() {
            anyhow::bail!(
                "Repository \"{}\": top-level \"name\", \"release-prefix\", \
                 and \"tag-prefix\" cannot be combined with a \"packages\" list",
                value.repository,
            );
        }
        if value.platforms.is_some() {
            anyhow::bail!(
                "Repository \"{}\": top-level \"platforms\" \
                 cannot be combined with a \"packages\" list — \
                 set platforms on individual sub-packages instead",
                value.repository,
            );
        }
        if sub_packages.is_empty() {
            anyhow::bail!(
                "Repository \"{}\": \"packages\" list must not be empty",
                value.repository,
            );
        }

        sub_packages
            .into_iter()
            .map(|sp| {
                let name = conda_package_name(Some(&sp.name), &repository.repo);
                Ok(Package {
                    name,
                    repository: repository.clone(),
                    release_prefix: sp.release_prefix,
                    tag_prefix: sp.tag_prefix,
                    platform_pattern: resolve_platforms(sp.platforms),
                    expose: sp.expose.unwrap_or_default(),
                })
            })
            .collect()
    } else {
        let name = conda_package_name(value.name.as_deref(), &repository.repo);
        Ok(vec![Package {
            name,
            repository,
            release_prefix: value.release_prefix,
            tag_prefix: value.tag_prefix,
            platform_pattern: resolve_platforms(value.platforms),
            expose: value.expose.unwrap_or_default(),
        }])
    }
}

fn max_import_releases_default() -> usize {
    usize::MAX
}

#[derive(Clone, Debug, Deserialize)]
pub struct Conda {
    pub channel: String,
    #[serde(
        rename = "max-import-releases",
        default = "max_import_releases_default"
    )]
    pub max_import_releases: usize,
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
        if value.conda.max_import_releases < 1 {
            return Err(anyhow!("max-import-releases must be >= 1"));
        }

        // Check for duplicate package names across ALL entries (including deprecated).
        {
            let mut seen: HashMap<String, (&str, bool)> = HashMap::new();
            for tp in &value.packages {
                let repo = Repository::try_from(tp.repository.as_str())?;
                let names: Vec<String> = if let Some(sub) = &tp.packages {
                    sub.iter()
                        .map(|sp| conda_package_name(Some(&sp.name), &repo.repo))
                        .collect()
                } else {
                    vec![conda_package_name(tp.name.as_deref(), &repo.repo)]
                };
                for name in names {
                    if let Some((prev_repo, prev_deprecated)) = seen.get(&name) {
                        if tp.deprecated || *prev_deprecated {
                            eprintln!(
                                "Note: Duplicate package name \"{name}\": \
                                 produced by both \"{prev_repo}\" and \"{}\"\
                                 (at least one is deprecated)",
                                tp.repository,
                            );
                        } else {
                            anyhow::bail!(
                                "Duplicate package name \"{name}\": \
                                 produced by both \"{prev_repo}\" and \"{}\"",
                                tp.repository,
                            );
                        }
                    }
                    seen.insert(name, (&tp.repository, tp.deprecated));
                }
            }
        }

        let packages: Vec<Package> = value
            .packages
            .drain(..)
            .filter(|tp| !tp.deprecated)
            .map(expand_toml_package)
            .collect::<anyhow::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();

        Ok(Config {
            packages,
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
            .flat_map(|p| p.platform_pattern.keys())
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

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn get_patterns_for(release_prefix: &str) -> HashMap<Platform, Vec<regex::Regex>> {
        let rp = if release_prefix.is_empty() {
            None
        } else {
            Some(release_prefix.to_string())
        };

        let toml = TomlPackage {
            name: None,
            release_prefix: rp,
            tag_prefix: None,
            repository: "foo/bar".to_string(),
            platforms: None,
            deprecated: false,
            packages: None,
            expose: None,
        };
        let mut packages = expand_toml_package(toml).unwrap();
        packages.remove(0).platform_pattern().unwrap()
    }

    fn toml_config(packages: Vec<TomlPackage>) -> TomlConfig {
        TomlConfig {
            packages,
            conda: Conda {
                channel: "test-channel".to_string(),
                max_import_releases: 5,
            },
        }
    }

    #[test]
    fn test_duplicate_package_names_rejected() {
        let config = toml_config(vec![
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "alice/foo".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "bob/foo".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
        ]);
        let err = Config::try_from(config).unwrap_err();
        assert!(
            err.to_string().contains("Duplicate package name"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_duplicate_package_names_case_insensitive() {
        let config = toml_config(vec![
            TomlPackage {
                name: Some("Foo".to_string()),
                release_prefix: None,
                tag_prefix: None,
                repository: "alice/something".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
            TomlPackage {
                name: Some("foo".to_string()),
                release_prefix: None,
                tag_prefix: None,
                repository: "bob/other".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
        ]);
        let err = Config::try_from(config).unwrap_err();
        assert!(
            err.to_string().contains("Duplicate package name"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_explicit_name_conflicts_with_repo_name() {
        let config = toml_config(vec![
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "alice/foo".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
            TomlPackage {
                name: Some("foo".to_string()),
                release_prefix: None,
                tag_prefix: None,
                repository: "bob/bar".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
        ]);
        let err = Config::try_from(config).unwrap_err();
        assert!(
            err.to_string().contains("Duplicate package name"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_unique_package_names_accepted() {
        let config = toml_config(vec![
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "alice/foo".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "bob/bar".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
        ]);
        Config::try_from(config).unwrap();
    }

    #[test]
    fn test_duplicate_with_deprecated_is_not_an_error() {
        let config = toml_config(vec![
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "alice/foo".to_string(),
                platforms: None,
                deprecated: true,
                packages: None,
                expose: None,
            },
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "bob/foo".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
        ]);
        let cfg = Config::try_from(config).unwrap();
        assert_eq!(cfg.packages.len(), 1);
        assert_eq!(cfg.packages[0].repository.owner, "bob");
    }

    #[test]
    fn test_duplicate_both_deprecated_is_not_an_error() {
        let config = toml_config(vec![
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "alice/foo".to_string(),
                platforms: None,
                deprecated: true,
                packages: None,
                expose: None,
            },
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "bob/foo".to_string(),
                platforms: None,
                deprecated: true,
                packages: None,
                expose: None,
            },
        ]);
        let cfg = Config::try_from(config).unwrap();
        assert!(cfg.packages.is_empty());
    }

    #[test]
    fn test_multi_package_entry_expands() {
        let config = toml_config(vec![TomlPackage {
            name: None,
            release_prefix: None,
            tag_prefix: None,
            repository: "oxc-project/oxc".to_string(),
            platforms: None,
            deprecated: false,
            packages: Some(vec![
                TomlSubPackage {
                    name: "oxfmt".to_string(),
                    release_prefix: Some("oxfmt".to_string()),
                    tag_prefix: None,
                    platforms: None,
                    expose: None,
                },
                TomlSubPackage {
                    name: "oxlint".to_string(),
                    release_prefix: Some("oxlint".to_string()),
                    tag_prefix: None,
                    platforms: None,
                    expose: None,
                },
            ]),

            expose: None,
        }]);
        let cfg = Config::try_from(config).unwrap();
        assert_eq!(cfg.packages.len(), 2);
        assert_eq!(cfg.packages[0].name, "oxfmt");
        assert_eq!(cfg.packages[1].name, "oxlint");
        assert_eq!(cfg.packages[0].repository.repo, "oxc");
        assert_eq!(cfg.packages[1].repository.repo, "oxc");
    }

    #[test]
    fn test_multi_package_rejects_top_level_name() {
        let config = toml_config(vec![TomlPackage {
            name: Some("bad".to_string()),
            release_prefix: None,
            tag_prefix: None,
            repository: "owner/repo".to_string(),
            platforms: None,
            deprecated: false,
            packages: Some(vec![TomlSubPackage {
                name: "pkg".to_string(),
                release_prefix: None,
                tag_prefix: None,
                platforms: None,
                expose: None,
            }]),

            expose: None,
        }]);
        let err = Config::try_from(config).unwrap_err();
        assert!(
            err.to_string().contains("cannot be combined"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_multi_package_duplicate_names_rejected() {
        let config = toml_config(vec![
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "alice/foo".to_string(),
                platforms: None,
                deprecated: false,
                packages: None,
                expose: None,
            },
            TomlPackage {
                name: None,
                release_prefix: None,
                tag_prefix: None,
                repository: "oxc-project/oxc".to_string(),
                platforms: None,
                deprecated: false,
                packages: Some(vec![TomlSubPackage {
                    name: "foo".to_string(),
                    release_prefix: None,
                    tag_prefix: None,
                    platforms: None,
                    expose: None,
                }]),

                expose: None,
            },
        ]);
        let err = Config::try_from(config).unwrap_err();
        assert!(
            err.to_string().contains("Duplicate package name"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_expose_dirs_parses() {
        let config = toml_config(vec![TomlPackage {
            name: Some("graalvm".to_string()),
            release_prefix: None,
            tag_prefix: None,
            repository: "graalvm/graalvm-ce-builds".to_string(),
            platforms: None,
            deprecated: false,
            packages: None,
            expose: Some(vec!["conf".to_string(), "jmods".to_string()]),
        }]);
        let cfg = Config::try_from(config).unwrap();
        assert_eq!(cfg.packages.len(), 1);
        assert_eq!(
            cfg.packages[0].expose,
            vec!["conf".to_string(), "jmods".to_string()]
        );
    }

    #[test]
    fn test_expose_standalone_accepted() {
        let config = toml_config(vec![TomlPackage {
            name: Some("mypkg".to_string()),
            release_prefix: None,
            tag_prefix: None,
            repository: "owner/repo".to_string(),
            platforms: None,
            deprecated: false,
            packages: None,
            expose: Some(vec!["conf".to_string()]),
        }]);
        let cfg = Config::try_from(config).unwrap();
        assert_eq!(cfg.packages[0].expose, vec!["conf".to_string()]);
    }
}
