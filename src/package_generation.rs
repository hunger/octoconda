// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::{
    collections::{HashMap, HashSet},
    io::Write as _,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Context as _;
use rattler_conda_types::{Platform, VersionWithSource};

use crate::config_file::Package;

#[derive(PartialEq, Eq)]
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

pub fn generate_build_script(work_dir: &Path) -> anyhow::Result<()> {
    let build_script = work_dir.join("build.sh");
    let mut file =
        std::fs::File::create_new(build_script).context("Failed to create the build script")?;
    let content = include_str!("../scripts/build.sh");
    file.write_all(content.as_bytes())
        .context("Failed to write build script")?;
    Ok(())
}

pub fn generate_env_file(
    work_dir: &Path,
    config: &crate::config_file::Config,
) -> anyhow::Result<()> {
    let env_file = work_dir.join("env.sh");
    let mut file = std::fs::File::create_new(env_file).context("Failed to create the env file")?;
    let content = format!(
        r#"
TARGET_CHANNEL="{}"
"#,
        config.conda.short_channel()?,
    );
    file.write_all(content.as_bytes())
        .context("Failed to write env.sh")?;
    Ok(())
}

pub struct PackagingStatus {
    pub platform: Platform,
    pub status: Status,
    pub message: String,
}

pub struct VersionPackagingStatus {
    pub version: Option<String>,
    pub status: Vec<PackagingStatus>,
}

impl PackagingStatus {
    pub fn github_failed() -> Vec<Self> {
        vec![Self {
            platform: rattler_conda_types::Platform::Unknown,
            status: Status::Failed,
            message: "could not retrieve release information from Github".to_string(),
        }]
    }

    pub fn recipe_generation_failed(platform: Platform) -> Self {
        Self {
            platform,
            status: Status::Failed,
            message: "could not generate package recipe".to_string(),
        }
    }

    pub fn invalid_version() -> Self {
        Self {
            platform: Platform::Unknown,
            status: Status::Failed,
            message: "could not parse version number from github release".to_string(),
        }
    }

    pub fn skip_platform(platform: Platform) -> Self {
        Self {
            platform,
            status: Status::Succeeded,
            message: "already in conda".to_string(),
        }
    }

    pub fn missing_platform(platform: Platform) -> Self {
        Self {
            platform,
            status: Status::Skipped,
            message: "platform file not found".to_string(),
        }
    }

    pub fn success(platform: Platform) -> Self {
        Self {
            platform,
            status: Status::Succeeded,
            message: "ok".to_string(),
        }
    }
}

pub fn report_results(status: &HashMap<String, Vec<VersionPackagingStatus>>) -> String {
    let mut result = String::new();
    for (package, sub_status) in status {
        let package_status = sub_status.iter().flat_map(|v| v.status.iter()).fold(
            Status::Succeeded,
            |acc, s| match (&s.status, acc) {
                (&Status::Failed, _) => Status::Failed,
                (&Status::Succeeded, Status::Failed) => Status::Failed,
                (&Status::Succeeded, Status::Succeeded) => Status::Succeeded,
                (&Status::Succeeded, Status::Skipped) => Status::Succeeded,
                (&Status::Skipped, Status::Failed) => Status::Failed,
                (&Status::Skipped, Status::Succeeded) => Status::Succeeded,
                (&Status::Skipped, Status::Skipped) => Status::Skipped,
            },
        );

        result.push_str(&format!(
            "{package_status}: {} ({} packages)\n",
            package,
            sub_status.len()
        ));

        for vs in sub_status {
            let mut version = vs.version.clone().unwrap_or_default();

            let skipped = {
                let skipped = vs
                    .status
                    .iter()
                    .filter_map(|s| (s.status == Status::Skipped).then_some(s.platform))
                    .fold(String::new(), |acc, p| {
                        if acc.is_empty() {
                            format!("{p}")
                        } else {
                            format!("{acc}, {p}")
                        }
                    });
                if skipped.is_empty() {
                    skipped
                } else {
                    format!(" skipped: {skipped}")
                }
            };

            result.push_str(&format!("    {version}{skipped}\n"));

            for s in &vs.status {
                if s.status == Status::Skipped {
                    continue;
                }
                result.push_str(&format!(
                    "        {}: {} {}\n",
                    s.status, s.platform, s.message
                ));
                version = version.chars().map(|_| ' ').collect()
            }
        }
    }
    result
}

fn match_platform<'a>(
    patterns: &[regex::Regex],
    assets: &'a [octocrab::models::repos::Asset],
) -> Option<&'a octocrab::models::repos::Asset> {
    for r in patterns {
        for a in assets {
            if r.is_match(&a.name) {
                return Some(a);
            }
        }
    }
    None
}

pub fn generate_packaging_data(
    package: &Package,
    repository: &octocrab::models::Repository,
    releases: &[octocrab::models::repos::Release],
    repo_packages: &[rattler_conda_types::RepoDataRecord],
    work_dir: &Path,
) -> anyhow::Result<Vec<VersionPackagingStatus>> {
    let mut result = vec![];

    for r in releases {
        let version_string = r
            .tag_name
            .strip_prefix("v")
            .map(|s| s.to_string())
            .unwrap_or_else(|| r.tag_name.clone());

        let Ok(version) = rattler_conda_types::Version::from_str(&version_string) else {
            result.push(VersionPackagingStatus {
                version: Some(version_string.clone()),
                status: vec![PackagingStatus::invalid_version()],
            });
            continue;
        };
        let version = VersionWithSource::new(version, &version_string);
        let mut version_result = vec![];

        let mut found_platforms = HashSet::new();

        for (platform, pattern) in &package.platforms {
            if let Some(asset) = match_platform(&pattern[..], &r.assets[..]) {
                found_platforms.insert(platform);

                if repo_packages.iter().any(|r| {
                    r.package_record.subdir == platform.to_string()
                        && r.package_record.name.as_normalized() == package.name
                        && r.package_record.version == version
                }) {
                    version_result.push(PackagingStatus::skip_platform(*platform));
                    continue;
                }

                version_result.push(generate_package(
                    work_dir,
                    package,
                    &version_string,
                    platform,
                    repository,
                    asset,
                ));
            }
        }

        for platform in package.platforms.keys() {
            if !found_platforms.contains(platform) {
                version_result.push(PackagingStatus::missing_platform(*platform));
            }
        }

        result.push(VersionPackagingStatus {
            version: Some(version_string),
            status: version_result,
        });
    }

    Ok(result)
}

fn extract_digest(asset: &octocrab::models::repos::Asset) -> Option<(String, String)> {
    asset.digest.as_ref().map(|d| {
        let digest = d.strip_prefix("sha256:").unwrap();
        ("sha256".to_string(), digest.to_string())
    })
}

fn extract_about(
    package_version: &str,
    repository: &octocrab::models::Repository,
    asset: &octocrab::models::repos::Asset,
) -> String {
    let digest = extract_digest(asset)
        .map(|(algo, value)| format!(" with\n    {algo}: {value}"))
        .unwrap_or_default();
    let mut result = format!(
        r#"about:
  repository: {1}
  description: |
    Repackaged binaries found at
    {3}{4}

    This is version {2} of the repository {0} on github"#,
        repository
            .html_url
            .as_ref()
            .map(|u| u.path().to_string())
            .unwrap(),
        repository.html_url.as_ref().unwrap(),
        package_version,
        asset.browser_download_url,
        digest
    );
    if let Some(homepage) = &repository.homepage
        && !homepage.is_empty()
    {
        result.push_str(&format!("\n  homepage: \"{homepage}\""));
    }
    if let Some(license) = &repository.license {
        // Fix outdated licenses
        let license_info = match license.spdx_id.as_str() {
            "GPL-3.0" => "GPL-3.0-only",
            l => l,
        };
        result.push_str(&format!("\n  license: \"{}\"", license_info));
    }
    if let Some(description) = &repository.description {
        result.push_str(&format!("\n  summary: \"{description}\""));
    }
    result
}

fn generate_rattler_build_recipe(
    work_dir: &Path,
    package_name: &str,
    package_version: &str,
    target_platform: &Platform,
    repository: &octocrab::models::Repository,
    asset: &octocrab::models::repos::Asset,
) -> anyhow::Result<PathBuf> {
    let platform_dir = work_dir.join(format!("{target_platform}",));
    let recipe_dir = platform_dir.join(format!("{package_name}-{package_version}",));
    std::fs::create_dir_all(&recipe_dir).context("Failed to create recipe directory")?;

    let build_script_source = work_dir.join("build.sh");
    let build_script_destination = recipe_dir.join("build.sh");
    #[cfg(not(target_os = "windows"))]
    std::os::unix::fs::symlink(build_script_source, build_script_destination)
        .context("Failed to soft link build script")?;
    #[cfg(target_os = "windows")]
    std::os::windows::fs::symlink_file(build_script_source, build_script_destination)
        .context("Failed to soft link build script")?;

    let recipe_file = recipe_dir.join("recipe.yaml");
    let mut file = std::fs::File::create_new(&recipe_file).context(format!(
        "Failed to create recipe file \"{}\"",
        recipe_file.display()
    ))?;

    let url = asset.browser_download_url.to_string();
    let digest = extract_digest(asset)
        .map(|(algo, value)| format!("\n  {algo}: {value}"))
        .unwrap_or_default();

    let about = extract_about(package_version, repository, asset);
    let pn = package_name.to_lowercase();

    let archive = {
        let path = PathBuf::from(asset.browser_download_url.path());
        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();
        let full_ext = if file_name.ends_with(".zip") {
            ".zip"
        } else if let Some(pos) = file_name.find(".tar.") {
            &file_name[pos..]
        } else if file_name.ends_with(".tgz") {
            ".tar.gz"
        } else if file_name.ends_with(".txz") {
            ".tar.xz"
        } else if file_name.ends_with(".gz") {
            ".gz"
        } else if file_name.ends_with(".xz") {
            ".xz"
        } else if file_name.ends_with(".zst") {
            ".zst"
        } else {
            ""
        };
        format!("{pn}-{package_version}-{target_platform}{full_ext}")
    };

    let content = format!(
        r#"package:
  name: {pn}
  version: "{package_version}"
  
source:
  url: "{url}"{digest}
  file_name: "{archive}"

build:
  dynamic_linking:
    binary_relocation: false
  prefix_detection:
    ignore: true

tests:
  - package_contents:
      files:
        not_exists:
          - .*
      bin:
        - "*"

{about}"#,
    );

    file.write_all(content.as_bytes()).context(format!(
        "Failed to populate recipe file \"{}\"",
        recipe_file.display(),
    ))?;

    Ok(recipe_dir)
}

fn generate_package(
    work_dir: &Path,
    package: &Package,
    package_version: &str,
    target_platform: &Platform,
    repository: &octocrab::models::Repository,
    asset: &octocrab::models::repos::Asset,
) -> PackagingStatus {
    match generate_rattler_build_recipe(
        work_dir,
        &package.name,
        package_version,
        target_platform,
        repository,
        asset,
    ) {
        Ok(_) => PackagingStatus::success(*target_platform),
        Err(e) => {
            eprintln!(
                "Error in {}@{package_version}-{target_platform},\n using {asset:#?}: {e}",
                package.name
            );
            PackagingStatus::recipe_generation_failed(*target_platform)
        }
    }
}
