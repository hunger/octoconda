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
    let content = r#"#!/bin/sh

WORK_DIR="${PWD}"

SRC="${PKG_NAME}-${PKG_VERSION}-${target_platform}"

if test -f "${SRC}.zip"; then
    ( cd $PREFIX && unzip -n "${WORK_DIR}/${SRC}.zip" )
elif test -f "${SRC}.tar.gz"; then
    ( cd $PREFIX && tar -xzf "${WORK_DIR}/${SRC}.tar.gz" )
else
    echo "Failed to find binary package file to repackage"
    exit 1
fi

cd "$PREFIX"

# Move everything out of a "foo-arch-version" folder
DIRECTORY_COUNT=$(find . -mindepth 1 -maxdepth 1 -type d -not -name conda-meta| wc -l)

if [ "$DIRECTORY_COUNT" -eq 1 ]; then
    if test -d "bin"; then
        echo "Found only a bin subdir, this looks good"
    else
        # move everything up a level
        SUBDIR=$(find . -mindepth 1 -maxdepth 1 -type d -not -name conda-meta)

        mv "${SUBDIR}"/* .
        rmdir "${SUBDIR}"
    fi
fi

# Move all executable files into bin
mkdir -p bin
mkdir -p extras

for f in *; do
    if test -f "${f}"; then
        if test -x "${f}"; then
            mv "${f}" bin
        else
            case "$f" in
            *.exe|*.bat|*.com)
                mv "${f}" bin
                ;;
            *)
                mv "${f}" extras
                ;;
            esac
        fi
    elif test -d "${f}"; then
        case "${f}" in
        conda-meta|bin|etc|include|lib|man|ssl|extras)
            ;;
        *)
            mv "${f}" extras
        esac
    fi
done
"#;
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
    platform: Platform,
    version: Option<String>,
    status: Status,
    message: String,
}

impl PackagingStatus {
    pub fn github_failed() -> Vec<Self> {
        vec![Self {
            version: None,
            platform: rattler_conda_types::Platform::Unknown,
            status: Status::Failed,
            message: "could not retrieve release information from Github".to_string(),
        }]
    }

    pub fn recipe_generation_failed(platform: Platform, version: String) -> Self {
        Self {
            version: Some(version),
            platform,
            status: Status::Failed,
            message: "could nod generate package recipe".to_string(),
        }
    }

    pub fn invalid_version(version: String) -> Self {
        Self {
            version: Some(version),
            platform: Platform::Unknown,
            status: Status::Failed,
            message: "could not parse version number from github release".to_string(),
        }
    }

    pub fn skip_platform(platform: Platform, version: String) -> Self {
        Self {
            version: Some(version),
            platform,
            status: Status::Succeeded,
            message: "already in conda".to_string(),
        }
    }

    pub fn missing_platform(platform: Platform, version: String) -> Self {
        Self {
            version: Some(version),
            platform,
            status: Status::Skipped,
            message: "platform file not found".to_string(),
        }
    }

    pub fn success(platform: Platform, version: String) -> Self {
        Self {
            version: Some(version),
            platform,
            status: Status::Succeeded,
            message: "ok".to_string(),
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
    repository: &octocrab::models::Repository,
    releases: &[octocrab::models::repos::Release],
    repo_packages: &[rattler_conda_types::RepoDataRecord],
    work_dir: &Path,
) -> anyhow::Result<Vec<PackagingStatus>> {
    let mut result = vec![];

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

        for asset in &r.assets {
            for (platform, pattern) in &package.platforms {
                if pattern.is_match(&asset.name) {
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

                    result.push(generate_package(
                        work_dir,
                        package,
                        &version_string,
                        platform,
                        repository,
                        asset,
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
  repository: {0}
  description:|
    Repackaged binaries found at
    {2}{3}

This is version {1} of the repository {0} on github"#,
        repository
            .html_url
            .as_ref()
            .map(|u| u.path().to_string())
            .unwrap(),
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
        result.push_str(&format!("\n  license: \"{}\"", license.spdx_id));
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
            "zip"
        } else if let Some(pos) = file_name.find(".tar.") {
            &file_name[pos + 1..]
        } else {
            panic!("unexpected file format")
        };
        format!("{pn}-{package_version}-{target_platform}.{full_ext}")
    };

    let content = format!(
        r#"
package:
  name: {pn}
  version: "{package_version}"
  
source:
  url: "{url}{digest}"
  file_name: "{archive}"

build:
  dynamic_linking:
    binary_relocation: false
  prefix_detection:
    ignore: true

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
    let Ok(_) = generate_rattler_build_recipe(
        work_dir,
        &package.name,
        package_version,
        target_platform,
        repository,
        asset,
    ) else {
        return PackagingStatus::recipe_generation_failed(
            *target_platform,
            package_version.to_string(),
        );
    };

    PackagingStatus::success(*target_platform, package_version.to_string())
}
