// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
};

use rattler_conda_types::{Platform, VersionWithSource};

use crate::config_file::Package;

pub enum Status {
    Failed,
    Succeeded,
    Skipped,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let output = match self {
            Status::Failed => "❌",
            Status::Succeeded => "✔ ",
            Status::Skipped => "❓",
        };
        write!(f, "{output}")
    }
}

pub struct PackagingStatus {
    platform: Platform,
    version: Option<String>,
    status: Status,
    message: String,
    package_file: Option<PathBuf>,
}

impl PackagingStatus {
    pub fn github_failed() -> Vec<Self> {
        vec![Self {
            version: None,
            platform: rattler_conda_types::Platform::Unknown,
            status: Status::Failed,
            message: "could not retrieve release information from Github".to_string(),
            package_file: None,
        }]
    }

    pub fn package_dir_failed() -> Vec<Self> {
        vec![Self {
            version: None,
            platform: rattler_conda_types::Platform::Unknown,
            status: Status::Failed,
            message: "could not create package directory".to_string(),
            package_file: None,
        }]
    }

    pub fn platform_dir_failed(platform: Platform, version: String) -> Self {
        Self {
            version: Some(version),
            platform,
            status: Status::Failed,
            message: "could not create platform directory".to_string(),
            package_file: None,
        }
    }

    pub fn invalid_version(version: String) -> Self {
        Self {
            version: Some(version),
            platform: Platform::Unknown,
            status: Status::Failed,
            message: "could not parse version number from github release".to_string(),
            package_file: None,
        }
    }

    pub fn skip_platform(platform: Platform, version: String) -> Self {
        Self {
            version: Some(version),
            platform,
            status: Status::Skipped,
            message: "skipped, already in conda".to_string(),
            package_file: None,
        }
    }

    pub fn missing_platform(platform: Platform, version: String) -> Self {
        Self {
            version: Some(version),
            platform,
            status: Status::Failed,
            message: "platform file not found".to_string(),
            package_file: None,
        }
    }

    pub fn success(platform: Platform, version: String, package_file: PathBuf) -> Self {
        Self {
            version: Some(version),
            platform,
            status: Status::Succeeded,
            message: "ok".to_string(),
            package_file: Some(package_file),
        }
    }
}

pub fn report_results(result: &HashMap<String, Vec<PackagingStatus>>) {
    for (package, sub_status) in result {
        let package_status =
            sub_status
                .iter()
                .fold(Status::Succeeded, |acc, s| match (&s.status, acc) {
                    (&Status::Failed, _) => Status::Failed,
                    (&Status::Succeeded, Status::Failed) => Status::Failed,
                    (&Status::Succeeded, Status::Succeeded) => Status::Succeeded,
                    (&Status::Succeeded, Status::Skipped) => Status::Succeeded,
                    (&Status::Skipped, Status::Failed) => Status::Failed,
                    (&Status::Skipped, Status::Succeeded) => Status::Succeeded,
                    (&Status::Skipped, Status::Skipped) => Status::Skipped,
                });

        eprintln!(
            "{package_status}: {} ({} packages)",
            package,
            sub_status.len()
        );
        for s in sub_status {
            let sep = match (&s.version, s.platform) {
                (None, Platform::NoArch) => String::new(),
                (None, p) => format!("{p}: "),
                (Some(v), Platform::NoArch) => format!("{v}: "),
                (Some(v), p) => format!("{v} on {p}: "),
            };
            eprintln!("    {} {}{}", s.status, sep, s.message);
        }
    }
}

pub fn generate_packaging_data(
    package: &Package,
    releases: &[octocrab::models::repos::Release],
    repo_packages: &[rattler_conda_types::RepoDataRecord],
    package_dir: &Path,
) -> anyhow::Result<Vec<PackagingStatus>> {
    let mut result = vec![];

    eprintln!("Looking at package {package:#?}");

    for r in releases {
        let version_string = r
            .tag_name
            .strip_prefix("v")
            .map(|s| s.to_string())
            .unwrap_or_else(|| r.tag_name.clone());

        let Ok(version) = rattler_conda_types::Version::from_str(&version_string) else {
            result.push(PackagingStatus::invalid_version(version_string));
            continue;
        };
        let version = VersionWithSource::new(version, &version_string);

        let mut found_platforms = HashSet::new();

        for a in &r.assets {
            for (platform, pattern) in &package.platforms {
                if pattern.is_match(&a.name) {
                    found_platforms.insert(platform);

                    if repo_packages.iter().any(|r| {
                        r.package_record.subdir == platform.to_string()
                            && r.package_record.name.as_normalized() == package.name
                            && r.package_record.version == version
                    }) {
                        result.push(PackagingStatus::skip_platform(
                            *platform,
                            version_string.clone(),
                        ));
                        continue;
                    }

                    let package_file = package_dir.join("{version}-{platform}.toml");
                    result.push(PackagingStatus::success(
                        *platform,
                        version_string.clone(),
                        package_file,
                    ));
                }
            }
        }

        for platform in package.platforms.keys() {
            if !found_platforms.contains(platform) {
                result.push(PackagingStatus::missing_platform(
                    *platform,
                    version_string.clone(),
                ));
            }
        }
    }

    Ok(result)
}
