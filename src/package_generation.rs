// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::{
    collections::BTreeMap,
    io::Write as _,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Context as _;
use rattler_conda_types::{Platform, VersionWithSource};

use crate::config_file::Package;

#[derive(PartialEq, Eq)]
pub enum StatusReason {
    RecipeGenerated,
    InvalidVersion,
    NotOnGithub,
    AlreadyInConda,
    NoBinaryForPlatformFound,
    RecipeGenerationFailed { message: String },
}

impl std::fmt::Display for StatusReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let output = match self {
            StatusReason::RecipeGenerated => "ok",
            StatusReason::InvalidVersion => "failed to parse version",
            StatusReason::NotOnGithub => "release not found on Github",
            StatusReason::AlreadyInConda => "release already in conda channel",
            StatusReason::NoBinaryForPlatformFound => "no binary for platform found",
            StatusReason::RecipeGenerationFailed { message } => {
                &format!("failed to generate recipe: {message}")
            }
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
    pub reason: StatusReason,
}

pub struct VersionPackagingStatus {
    pub version: Option<String>,
    pub status: Vec<PackagingStatus>,
}

pub enum PackageResult {
    GithubFailed {
        repository: String,
        message: String,
    },
    Ok {
        repository: String,
        name: String,
        versions: Vec<VersionPackagingStatus>,
    },
}

impl PackageResult {
    fn display_name(&self) -> String {
        match self {
            PackageResult::GithubFailed { repository, .. } => repository.clone(),
            PackageResult::Ok {
                repository, name, ..
            } => display_name(repository, name),
        }
    }

    fn repository(&self) -> &str {
        match self {
            PackageResult::GithubFailed { repository, .. } => repository,
            PackageResult::Ok { repository, .. } => repository,
        }
    }
}

fn display_name(repository: &str, name: &str) -> String {
    let repo_part = repository.rsplit('/').next().unwrap_or(repository);
    if name.eq_ignore_ascii_case(repo_part) {
        repository.to_string()
    } else {
        format!("{} ({})", repository, name)
    }
}

impl PackagingStatus {
    pub fn recipe_generation_failed(platform: Platform, message: String) -> Self {
        Self {
            platform,
            reason: StatusReason::RecipeGenerationFailed { message },
        }
    }

    pub fn invalid_version() -> Self {
        Self {
            platform: Platform::Unknown,
            reason: StatusReason::InvalidVersion,
        }
    }

    pub fn already_in_conda(platform: Platform) -> Self {
        Self {
            platform,
            reason: StatusReason::AlreadyInConda,
        }
    }

    pub fn no_platform_binary(platform: Platform) -> Self {
        Self {
            platform,
            reason: StatusReason::NoBinaryForPlatformFound,
        }
    }

    pub fn success(platform: Platform) -> Self {
        Self {
            platform,
            reason: StatusReason::RecipeGenerated,
        }
    }

    pub fn in_conda_not_on_github(platform: Platform) -> Self {
        Self {
            platform,
            reason: StatusReason::NotOnGithub,
        }
    }
}

#[derive(Clone)]
struct RecipeErrorMessage {
    package: String,
    platform: String,
    version: String,
    message: String,
}

#[derive(Clone, Default)]
struct ReportData {
    github_errors: BTreeMap<String, Vec<String>>, // message -> repositories
    no_recipe: Vec<RecipeErrorMessage>,

    recipe_generated: BTreeMap<(String, String), Vec<String>>, // display, platform -> [version]
    not_on_github: BTreeMap<String, Vec<String>>,              // display -> [version]
    invalid_version: BTreeMap<String, Vec<String>>,            // display -> [version]
    already_in_conda: BTreeMap<(String, String), Vec<String>>, // display, platform -> [version]
    no_binary_found: BTreeMap<(String, String), Vec<String>>,  // display, platform -> [version]
}

fn categorize_package_version(
    result: &mut ReportData,
    display: &str,
    version: &VersionPackagingStatus,
) {
    let ver = version.version.as_deref().unwrap_or("?");
    for platform_status in &version.status {
        match &platform_status.reason {
            StatusReason::RecipeGenerated => result
                .recipe_generated
                .entry((display.to_string(), platform_status.platform.to_string()))
                .or_default()
                .push(ver.to_string()),
            StatusReason::InvalidVersion => result
                .invalid_version
                .entry(display.to_string())
                .or_default()
                .push(ver.to_string()),
            StatusReason::NotOnGithub => result
                .not_on_github
                .entry(display.to_string())
                .or_default()
                .push(ver.to_string()),
            StatusReason::AlreadyInConda => result
                .already_in_conda
                .entry((display.to_string(), platform_status.platform.to_string()))
                .or_default()
                .push(ver.to_string()),
            StatusReason::NoBinaryForPlatformFound => result
                .no_binary_found
                .entry((display.to_string(), platform_status.platform.to_string()))
                .or_default()
                .push(ver.to_string()),
            StatusReason::RecipeGenerationFailed { message } => {
                result.no_recipe.push(RecipeErrorMessage {
                    package: display.to_string(),
                    platform: platform_status.platform.to_string(),
                    version: ver.to_string(),
                    message: message.to_string(),
                })
            }
        }
    }
}

fn categorize_package_result(report_data: &mut ReportData, pkg: &PackageResult) {
    let display = pkg.display_name();

    match pkg {
        PackageResult::GithubFailed {
            repository,
            message,
        } => {
            let message = if let Some(index) = message.find('\n') {
                // Multiline error, e.g. from Hyper. Ignore all but the real message in
                // the first line.
                message[..index].to_string()
            } else {
                message.clone()
            };

            report_data
                .github_errors
                .entry(message)
                .or_default()
                .push(repository.clone());
        }
        PackageResult::Ok { versions, .. } => {
            for v in versions {
                categorize_package_version(report_data, &display, v);
            }
        }
    }
}

fn collect_result_data(input: &[PackageResult]) -> ReportData {
    let mut result = ReportData::default();

    // Sort by display name for the sections
    let mut sorted_indices: Vec<usize> = (0..input.len()).collect();
    sorted_indices.sort_by_key(|&i| input[i].display_name());

    for &i in &sorted_indices {
        categorize_package_result(&mut result, &input[i]);
    }

    result
}

pub fn report_results(
    results: &[PackageResult],
    total_configured: usize,
    unknown_in_conda: &[String],
    max_releases_to_import: usize,
    platforms_count: usize,
) -> String {
    let mut output = String::new();

    if let Some(first) = results.first() {
        output.push_str(&format!(
            "Processed {}/{total_configured} repositories: {}\n\n",
            results.len(),
            first.repository(),
        ));
    }

    // Sort by display name for the sections
    let mut sorted_indices: Vec<usize> = (0..results.len()).collect();
    sorted_indices.sort_by_key(|&i| results[i].display_name());

    let report_data = collect_result_data(results);

    // Build report sections
    if !report_data.github_errors.is_empty() {
        let pkg_count: usize = report_data.github_errors.values().map(Vec::len).sum();
        output.push_str(&format!("GitHub errors ({pkg_count} packages):\n",));
        for (error, repos) in &report_data.github_errors {
            output.push_str(&format!("  {}:\n    {}\n", error, repos.join(", ")));
        }
        output.push('\n');
    }
    // Build report sections
    if !report_data.no_recipe.is_empty() {
        let packages: std::collections::HashSet<&str> = report_data
            .no_recipe
            .iter()
            .map(|e| e.package.as_str())
            .collect();
        output.push_str(&format!(
            "Recipe generation failures ({} files, {} packages):\n",
            report_data.no_recipe.len(),
            packages.len(),
        ));
        for error in &report_data.no_recipe {
            output.push_str(&format!(
                "  {}/{}@{}: {}\n",
                error.platform, error.package, error.version, error.message,
            ));
        }
        output.push('\n');
    }

    if !report_data.no_binary_found.is_empty() {
        let mut no_binary_report = String::new();
        let mut last_name = String::new();
        let mut last_versions = String::new();
        let mut last_count = 0_usize;

        for ((name, _), versions) in &report_data.no_binary_found {
            if name != &last_name {
                if last_count >= platforms_count && !last_name.is_empty() {
                    no_binary_report.push_str(&format!("  {last_name}: {last_versions}\n",));
                }
                last_count = 0;
                last_name = name.clone();
                last_versions = versions.join(", ");
            }
            if versions.len() >= max_releases_to_import {
                last_count += 1;
            }
        }
        if last_count >= platforms_count && !last_name.is_empty() {
            no_binary_report.push_str(&format!("  {last_name}: {last_versions}\n",));
        }

        if !no_binary_report.is_empty() {
            let packages: std::collections::HashSet<&str> = report_data
                .no_binary_found
                .keys()
                .map(|(d, _)| d.as_str())
                .collect();
            let file_count: usize = report_data.no_binary_found.values().map(Vec::len).sum();
            output.push_str(&format!(
                "No platform binary in release \
                 ({file_count} files, {} packages):\n{no_binary_report}\n",
                packages.len(),
            ));
        }
    }

    if !report_data.not_on_github.is_empty() {
        output.push_str(&format!(
            "Package versions in conda, not on GitHub ({} packages):\n",
            report_data.not_on_github.len(),
        ));
        for (name, versions) in &report_data.not_on_github {
            output.push_str(&format!("  {name}: {}\n", versions.join(", ")));
        }
        output.push('\n');
    }

    if !unknown_in_conda.is_empty() {
        output.push_str(&format!(
            "Unknown packages in conda ({}):\n",
            unknown_in_conda.len()
        ));
        for name in unknown_in_conda {
            output.push_str(&format!("  {name}\n"));
        }
        output.push('\n');
    }

    if !report_data.recipe_generated.is_empty() {
        let packages: std::collections::HashSet<&str> = report_data
            .recipe_generated
            .keys()
            .map(|(d, _)| d.as_str())
            .collect();
        let file_count: usize = report_data.recipe_generated.values().map(Vec::len).sum();
        output.push_str(&format!(
            "OK (generated recipe) ({file_count} files, {} packages):\n",
            packages.len(),
        ));
        for ((name, platform), versions) in &report_data.recipe_generated {
            output.push_str(&format!("  {name}/{platform}: {}\n", versions.join(", ")));
        }
    }

    output
}

fn match_platform<'a>(
    patterns: &[regex::Regex],
    assets: &'a [octocrab::models::repos::Asset],
) -> Option<&'a octocrab::models::repos::Asset> {
    let asset_names = assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>();
    match_platform_names(patterns, &asset_names).map(|index| &assets[index])
}

fn match_platform_names<'a>(patterns: &[regex::Regex], assets: &'a [&'a str]) -> Option<usize> {
    for r in patterns {
        for (index, a) in assets.iter().enumerate() {
            if r.is_match(a) {
                return Some(index);
            }
        }
    }
    None
}

pub fn generate_packaging_data(
    package: &Package,
    repository: &octocrab::models::Repository,
    releases: &[(octocrab::models::repos::Release, (String, u32))],
    repo_packages: &[rattler_conda_types::RepoDataRecord],
    work_dir: &Path,
) -> anyhow::Result<Vec<VersionPackagingStatus>> {
    let mut result = vec![];
    let pkg_records = crate::conda::find_by_name(repo_packages, &package.name);
    let platform_patterns = package.platform_pattern()?;

    for (r, (version_string, build_number)) in releases {
        let Ok(version) = rattler_conda_types::Version::from_str(version_string) else {
            result.push(VersionPackagingStatus {
                version: Some(version_string.clone()),
                status: vec![PackagingStatus::invalid_version()],
            });
            continue;
        };
        let version = VersionWithSource::new(version, version_string);
        let mut version_result = vec![];

        for (platform, pattern) in &platform_patterns {
            if let Some(asset) = match_platform(pattern, &r.assets) {
                if pkg_records.iter().any(|r| {
                    r.package_record.subdir == platform.to_string()
                        && r.package_record.version == version
                }) {
                    version_result.push(PackagingStatus::already_in_conda(*platform));
                    continue;
                }

                version_result.push(generate_package(
                    work_dir,
                    package,
                    version_string,
                    *build_number,
                    platform,
                    repository,
                    asset,
                ));
            } else {
                version_result.push(PackagingStatus::no_platform_binary(*platform));
            }
        }

        result.push(VersionPackagingStatus {
            version: Some(format!("{version_string}-{build_number}")),
            status: version_result,
        });
    }

    // Find versions in conda that have no corresponding GitHub release
    let github_versions: Vec<VersionWithSource> = releases
        .iter()
        .filter_map(|(_, (vs, _))| {
            rattler_conda_types::Version::from_str(vs)
                .ok()
                .map(|v| VersionWithSource::new(v, vs))
        })
        .collect();

    let mut conda_only: BTreeMap<String, Vec<PackagingStatus>> = BTreeMap::new();
    for record in pkg_records {
        if github_versions.contains(&record.package_record.version) {
            continue;
        }
        let version_str = record.package_record.version.to_string();
        let platform =
            Platform::from_str(&record.package_record.subdir).unwrap_or(Platform::Unknown);
        conda_only
            .entry(version_str)
            .or_default()
            .push(PackagingStatus::in_conda_not_on_github(platform));
    }

    for (version_str, statuses) in conda_only {
        result.push(VersionPackagingStatus {
            version: Some(version_str),
            status: statuses,
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

/// Map deprecated SPDX license identifiers to their current equivalents.
///
/// Based on <https://spdx.org/licenses/> — covers all 32 deprecated identifiers
/// that have a clear single-expression replacement.
///
/// Omitted (no single-ID replacement, need project-specific SPDX expressions):
///   eCos-2.0, Net-SNMP, Nunit, wxWindows
fn fix_spdx_license(spdx_id: &str) -> &str {
    match spdx_id {
        // GPL family
        "GPL-1.0" => "GPL-1.0-only",
        "GPL-1.0+" => "GPL-1.0-or-later",
        "GPL-2.0" => "GPL-2.0-only",
        "GPL-2.0+" => "GPL-2.0-or-later",
        "GPL-3.0" => "GPL-3.0-only",
        "GPL-3.0+" => "GPL-3.0-or-later",
        // AGPL family
        "AGPL-1.0" => "AGPL-1.0-only",
        "AGPL-1.0+" => "AGPL-1.0-or-later",
        "AGPL-3.0" => "AGPL-3.0-only",
        "AGPL-3.0+" => "AGPL-3.0-or-later",
        // LGPL family
        "LGPL-2.0" => "LGPL-2.0-only",
        "LGPL-2.0+" => "LGPL-2.0-or-later",
        "LGPL-2.1" => "LGPL-2.1-only",
        "LGPL-2.1+" => "LGPL-2.1-or-later",
        "LGPL-3.0" => "LGPL-3.0-only",
        "LGPL-3.0+" => "LGPL-3.0-or-later",
        // GFDL family
        "GFDL-1.1" => "GFDL-1.1-only",
        "GFDL-1.2" => "GFDL-1.2-only",
        "GFDL-1.3" => "GFDL-1.3-only",
        // GPL-with-exception → SPDX WITH expressions
        "GPL-2.0-with-autoconf-exception" => "GPL-2.0-only WITH Autoconf-exception-2.0",
        "GPL-2.0-with-bison-exception" => "GPL-2.0-only WITH Bison-exception-2.2",
        "GPL-2.0-with-classpath-exception" => "GPL-2.0-only WITH Classpath-exception-2.0",
        "GPL-2.0-with-font-exception" => "GPL-2.0-only WITH Font-exception-2.0",
        "GPL-2.0-with-GCC-exception" => "GPL-2.0-only WITH GCC-exception-2.0",
        "GPL-3.0-with-autoconf-exception" => "GPL-3.0-only WITH Autoconf-exception-3.0",
        "GPL-3.0-with-GCC-exception" => "GPL-3.0-only WITH GCC-exception-3.1",
        // BSD consolidation
        "BSD-2-Clause-FreeBSD" => "BSD-2-Clause",
        "BSD-2-Clause-NetBSD" => "BSD-2-Clause",
        // Other renames
        "bzip2-1.0.5" => "bzip2-1.0.6",
        "StandardML-NJ" => "SMLNJ",
        l => l,
    }
}

/// Escape a string for use inside a YAML double-quoted value.
fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn extract_about(
    package_version: &str,
    repository: &octocrab::models::Repository,
    asset: &octocrab::models::repos::Asset,
) -> String {
    let extra_section = {
        let upstream_digest = extract_digest(asset)
            .map(|(algo, digest)| format!("\n  upstream-{algo}: \"{}\"", yaml_escape(&digest)))
            .unwrap_or_default();
        let upstream_version =
            format!("\n  upstream-version: \"{}\"", yaml_escape(package_version));
        let upstream_repository = repository
            .html_url
            .as_ref()
            .map(|u| u.path()[1..].to_string()) // strip leading `/`
            .map(|u| format!("\n  upstream-repository: \"{}\"", yaml_escape(&u)))
            .unwrap_or_default();

        let download_url = format!(
            "\n  release-download-url: \"{}\"",
            yaml_escape(asset.browser_download_url.as_str())
        );
        format!(
            "extra:\n  upstream-forge: github.com{upstream_digest}{upstream_version}{upstream_repository}{download_url}\n"
        )
    };

    let about_section = {
        let homepage = if let Some(homepage) = &repository.homepage
            && !homepage.is_empty()
        {
            format!("  homepage: \"{}\"\n", yaml_escape(homepage))
        } else {
            String::new()
        };
        let repository_line = if let Some(repository_url) = repository.html_url.as_ref() {
            format!("  repository: \"{repository_url}\"\n")
        } else {
            String::new()
        };

        let license = if let Some(license) = &repository.license {
            let license_info = fix_spdx_license(&license.spdx_id);
            format!("\n  license: \"{}\"", yaml_escape(license_info))
        } else {
            String::new()
        };
        let summary_text = if let Some(description) = &repository.description {
            description.trim().to_owned()
        } else {
            String::new()
        };
        let summary = if let Some(description) = &repository.description {
            format!("\n  summary: \"{}\"", yaml_escape(description.trim()))
        } else {
            String::new()
        };

        format!(
            r#"
about:
  description: >
    {summary_text}
{homepage}{repository_line}{license}{summary}"#,
        )
    };

    format!(
        r#"{extra_section}
{about_section}"#
    )
}

fn generate_rattler_build_recipe(
    work_dir: &Path,
    package: &Package,
    package_version: &str,
    build_number: u32,
    target_platform: &Platform,
    repository: &octocrab::models::Repository,
    asset: &octocrab::models::repos::Asset,
) -> anyhow::Result<PathBuf> {
    let package_name = &package.name;
    let platform_dir = work_dir.join(format!("{target_platform}",));
    let recipe_dir = platform_dir.join(format!("{package_name}-{package_version}-{build_number}",));
    std::fs::create_dir_all(&recipe_dir).context("Failed to create recipe directory")?;

    let build_script_source = work_dir.join("build.sh");
    let build_script_destination = recipe_dir.join("build.sh");
    std::fs::copy(&build_script_source, &build_script_destination).context(format!(
        "Failed to copy build script from {build_script_source:?} to {build_script_destination:?}"
    ))?;

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
    let pn = package_name;

    let archive = format!("{pn}-{package_version}-{target_platform}");

    let url = yaml_escape(&url);
    let package_version = yaml_escape(package_version);
    let archive = yaml_escape(&archive);

    let build_extra_section = if !package.expose.is_empty() {
        let standard_dirs = ["bin", "etc", "extras", "include", "lib", "share", "ssl"];
        let mut file_globs: Vec<String> = standard_dirs
            .iter()
            .map(|d| format!("      - {d}/**"))
            .collect();
        for dir in &package.expose {
            file_globs.push(format!("      - {dir}/**"));
        }
        let files_section = format!("  files:\n    include:\n{}", file_globs.join("\n"));

        let expose_val = package.expose.join("|");
        let env_section = format!("  script:\n    env:\n      OCTOCONDA_EXPOSE: \"{expose_val}\"");

        format!("{files_section}\n{env_section}")
    } else {
        String::new()
    };

    let content = format!(
        r#"package:
  name: {pn}
  version: "{package_version}"

source:
  url: "{url}"{digest}
  file_name: "{archive}"

build:
  number: {build_number}
  dynamic_linking:
    binary_relocation: false
  prefix_detection:
    ignore: true
{build_extra_section}

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
    build_number: u32,
    target_platform: &Platform,
    repository: &octocrab::models::Repository,
    asset: &octocrab::models::repos::Asset,
) -> PackagingStatus {
    match generate_rattler_build_recipe(
        work_dir,
        package,
        package_version,
        build_number,
        target_platform,
        repository,
        asset,
    ) {
        Ok(_) => PackagingStatus::success(*target_platform),
        Err(e) => PackagingStatus::recipe_generation_failed(*target_platform, format!("{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config_file::tests::get_patterns_for;

    fn zoxide_names() -> Vec<&'static str> {
        vec![
            "zoxide-0.9.8-aarch64-apple-darwin.tar.gz",
            "zoxide-0.9.8-aarch64-linux-android.tar.gz",
            "zoxide-0.9.8-aarch64-pc-windows-msvc.zip",
            "zoxide-0.9.8-aarch64-unknown-linux-musl.tar.gz",
            "zoxide-0.9.8-arm-unknown-linux-musleabihf.tar.gz",
            "zoxide-0.9.8-armv7-unknown-linux-musleabihf.tar.gz",
            "zoxide-0.9.8-i686-unknown-linux-musl.tar.gz",
            "zoxide-0.9.8-x86_64-apple-darwin.tar.gz",
            "zoxide-0.9.8-x86_64-pc-windows-msvc.zip",
            "zoxide-0.9.8-x86_64-unknown-linux-musl.tar.gz",
            "Source code",
        ]
    }

    fn stripe_names() -> Vec<&'static str> {
        vec![
            "stripe-linux-checksums.txt",
            "stripe-mac-checksums.txt",
            "stripe-windows-checksums.txt",
            "stripe_1.37.2_linux_amd64.deb",
            "stripe_1.37.2_linux_amd64.rpm",
            "stripe_1.37.2_linux_arm64.deb",
            "stripe_1.37.2_linux_arm64.rpm",
            "stripe_1.37.2_linux_arm64.tar.gz",
            "stripe_1.37.2_linux_x86_64.tar.gz",
            "stripe_1.37.2_mac-os_arm64.tar.gz",
            "stripe_1.37.2_mac-os_x86_64.tar.gz",
            "stripe_1.37.2_windows_i386.zip",
            "stripe_1.37.2_windows_x86_64.zip",
            "Source code",
        ]
    }

    fn atuin_names() -> Vec<&'static str> {
        vec![
            "atuin-aarch64-apple-darwin-update",
            "atuin-aarch64-apple-darwin.tar.gz",
            "atuin-aarch64-apple-darwin.tar.gz.sha256",
            "atuin-aarch64-unknown-linux-gnu-update",
            "atuin-aarch64-unknown-linux-gnu.tar.gz",
            "atuin-aarch64-unknown-linux-gnu.tar.gz.sha256",
            "atuin-aarch64-unknown-linux-musl-update",
            "atuin-aarch64-unknown-linux-musl.tar.gz",
            "atuin-aarch64-unknown-linux-musl.tar.gz.sha256",
            "atuin-installer.sh",
            "atuin-x86_64-apple-darwin-update",
            "atuin-x86_64-apple-darwin.tar.gz",
            "atuin-x86_64-apple-darwin.tar.gz.sha256",
            "atuin-x86_64-unknown-linux-gnu-update",
            "atuin-x86_64-unknown-linux-gnu.tar.gz",
            "atuin-x86_64-unknown-linux-gnu.tar.gz.sha256",
            "atuin-x86_64-unknown-linux-musl-update",
            "atuin-x86_64-unknown-linux-musl.tar.gz",
            "atuin-x86_64-unknown-linux-musl.tar.gz.sha256",
            "dist-manifest.json",
            "sha256.sum",
            "source.tar.gz",
            "source.tar.gz.sha256",
            "Source code (zip)",
            "Source code (tar.gz)",
        ]
    }

    fn asm_lsp_names() -> Vec<&'static str> {
        vec![
            "asm-lsp-aarch64-apple-darwin.tar.gz",
            "asm-lsp-x86_64-apple-darwin.tar.gz",
            "asm-lsp-x86_64-unknown-linux-gnu.tar.gz",
        ]
    }

    fn cargo_binstall_names() -> Vec<&'static str> {
        vec![
            "cargo-binstall-aarch64-apple-darwin.full.zip",
            "cargo-binstall-aarch64-apple-darwin.full.zip.sig",
            "cargo-binstall-aarch64-apple-darwin.zip",
            "cargo-binstall-aarch64-apple-darwin.zip.sig",
            "cargo-binstall-aarch64-pc-windows-msvc.full.zip",
            "cargo-binstall-aarch64-pc-windows-msvc.full.zip.sig",
            "cargo-binstall-aarch64-pc-windows-msvc.zip",
            "cargo-binstall-aarch64-pc-windows-msvc.zip.sig",
            "cargo-binstall-aarch64-unknown-linux-gnu.full.tgz",
            "cargo-binstall-aarch64-unknown-linux-gnu.full.tgz.sig",
            "cargo-binstall-aarch64-unknown-linux-gnu.tgz",
            "cargo-binstall-aarch64-unknown-linux-gnu.tgz.sig",
            "cargo-binstall-aarch64-unknown-linux-musl.full.tgz",
            "cargo-binstall-aarch64-unknown-linux-musl.full.tgz.sig",
            "cargo-binstall-aarch64-unknown-linux-musl.tgz",
            "cargo-binstall-aarch64-unknown-linux-musl.tgz.sig",
            "cargo-binstall-armv7-unknown-linux-gnueabihf.full.tgz",
            "cargo-binstall-armv7-unknown-linux-gnueabihf.full.tgz.sig",
            "cargo-binstall-armv7-unknown-linux-gnueabihf.tgz",
            "cargo-binstall-armv7-unknown-linux-gnueabihf.tgz.sig",
            "cargo-binstall-armv7-unknown-linux-musleabihf.full.tgz",
            "cargo-binstall-armv7-unknown-linux-musleabihf.full.tgz.sig",
            "cargo-binstall-armv7-unknown-linux-musleabihf.tgz",
            "cargo-binstall-armv7-unknown-linux-musleabihf.tgz.sig",
            "cargo-binstall-universal-apple-darwin.full.zip",
            "cargo-binstall-universal-apple-darwin.full.zip.sig",
            "cargo-binstall-universal-apple-darwin.zip",
            "cargo-binstall-universal-apple-darwin.zip.sig",
            "cargo-binstall-x86_64-apple-darwin.full.zip",
            "cargo-binstall-x86_64-apple-darwin.full.zip.sig",
            "cargo-binstall-x86_64-apple-darwin.zip",
            "cargo-binstall-x86_64-apple-darwin.zip.sig",
            "cargo-binstall-x86_64-pc-windows-msvc.full.zip",
            "cargo-binstall-x86_64-pc-windows-msvc.full.zip.sig",
            "cargo-binstall-x86_64-pc-windows-msvc.zip",
            "cargo-binstall-x86_64-pc-windows-msvc.zip.sig",
            "cargo-binstall-x86_64-unknown-linux-gnu.full.tgz",
            "cargo-binstall-x86_64-unknown-linux-gnu.full.tgz.sig",
            "cargo-binstall-x86_64-unknown-linux-gnu.tgz",
            "cargo-binstall-x86_64-unknown-linux-gnu.tgz.sig",
            "cargo-binstall-x86_64-unknown-linux-musl.full.tgz",
            "cargo-binstall-x86_64-unknown-linux-musl.full.tgz.sig",
            "cargo-binstall-x86_64-unknown-linux-musl.tgz",
            "cargo-binstall-x86_64-unknown-linux-musl.tgz.sig",
            "minisign.pub",
        ]
    }

    fn bottom_names() -> Vec<&'static str> {
        vec![
            "bottom-0.11.4-1.x86_64.rpm",
            "bottom-musl-0.11.4-1.x86_64.rpm",
            "bottom-musl_0.11.4-1_amd64.deb",
            "bottom-musl_0.11.4-1_arm64.deb",
            "bottom-musl_0.11.4-1_armhf.deb",
            "bottom.desktop",
            "bottom_0.11.4-1_amd64.deb",
            "bottom_0.11.4-1_arm64.deb",
            "bottom_0.11.4-1_armhf.deb",
            "bottom_aarch64-apple-darwin.tar.gz",
            "bottom_aarch64-pc-windows-msvc.tar.gz",
            "bottom_aarch64-unknown-linux-gnu.tar.gz",
            "bottom_aarch64-unknown-linux-musl.tar.gz",
            "bottom_aarch64_installer.msi",
            "bottom_armv7-unknown-linux-gnueabihf.tar.gz",
            "bottom_armv7-unknown-linux-musleabihf.tar.gz",
            "bottom_i686-pc-windows-msvc.zip",
            "bottom_i686-unknown-linux-gnu.tar.gz",
            "bottom_i686-unknown-linux-musl.tar.gz",
            "bottom_powerpc64le-unknown-linux-gnu.tar.gz",
            "bottom_riscv64gc-unknown-linux-gnu.tar.gz",
            "bottom_x86_64-apple-darwin.tar.gz",
            "bottom_x86_64-pc-windows-gnu.zip",
            "bottom_x86_64-pc-windows-msvc.zip",
            "bottom_x86_64-unknown-freebsd-13.5.tar.gz",
            "bottom_x86_64-unknown-freebsd-14.3.tar.gz",
            "bottom_x86_64-unknown-freebsd-15.0.tar.gz",
            "bottom_x86_64-unknown-linux-gnu-2-17.tar.gz",
            "bottom_x86_64-unknown-linux-gnu.tar.gz",
            "bottom_x86_64-unknown-linux-musl.tar.gz",
            "bottom_x86_64_installer.msi",
            "choco.zip",
            "completion.tar.gz",
            "manpage.tar.gz",
        ]
    }

    fn jjui_names() -> Vec<&'static str> {
        vec![
            "jjui-0.9.6-darwin-amd64.zip",
            "jjui-0.9.6-darwin-arm64.zip",
            "jjui-0.9.6-linux-amd64.zip",
            "jjui-0.9.6-linux-arm64.zip",
            "jjui-0.9.6-windows-amd64.zip",
            "jjui-0.9.6-windows-arm64.zip",
        ]
    }

    fn bazelisk_names() -> Vec<&'static str> {
        vec![
            "bazelisk-amd64.deb",
            "bazelisk-arm64.deb",
            "bazelisk-darwin",
            "bazelisk-darwin-amd64",
            "bazelisk-darwin-arm64",
            "bazelisk-linux-amd64",
            "bazelisk-linux-arm64",
            "bazelisk-windows-amd64.exe",
            "bazelisk-windows-arm64.exe",
        ]
    }

    fn caligula_names() -> Vec<&'static str> {
        vec![
            "caligula-aarch64-darwin",
            "caligula-aarch64-linux",
            "caligula-x86_64-darwin",
            "caligula-x86_64-linux",
        ]
    }

    fn neovim_names() -> Vec<&'static str> {
        vec![
            "nvim-linux-arm64.appimage",
            "nvim-linux-arm64.appimage.zsync",
            "nvim-linux-arm64.tar.gz",
            "nvim-linux-x86_64.appimage",
            "nvim-linux-x86_64.appimage.zsync",
            "nvim-linux-x86_64.tar.gz",
            "nvim-macos-arm64.tar.gz",
            "nvim-macos-x86_64.tar.gz",
            "nvim-win-arm64.msi",
            "nvim-win-arm64.zip",
            "nvim-win64.msi",
            "nvim-win64.zip",
        ]
    }

    fn neovim_names_old() -> Vec<&'static str> {
        vec![
            "nvim-linux64.tar.gz",
            "nvim-macos.tar.gz",
            "nvim-win32.zip",
            "nvim-win64.zip",
            "nvim.appimage",
            "nvim.appimage.zsync",
        ]
    }

    fn shellcheck_names() -> Vec<&'static str> {
        vec![
            "shellcheck-v0.11.0.darwin.aarch64.tar.xz",
            "shellcheck-v0.11.0.darwin.x86_64.tar.xz",
            "shellcheck-v0.11.0.linux.aarch64.tar.xz",
            "shellcheck-v0.11.0.linux.armv6hf.tar.xz",
            "shellcheck-v0.11.0.linux.riscv64.tar.xz",
            "shellcheck-v0.11.0.linux.x86_64.tar.xz",
            "shellcheck-v0.11.0.zip",
        ]
    }

    fn glsl_analyzer_names() -> Vec<&'static str> {
        vec![
            "aarch64-linux-musl.zip",
            "aarch64-macos.zip",
            "aarch64-windows.zip",
            "x86_64-linux-musl.zip",
            "x86_64-macos.zip",
            "x86_64-windows.zip",
        ]
    }

    fn lazygit_names() -> Vec<&'static str> {
        vec![
            "lazygit_0.52.0_Darwin_arm64.tar.gz",
            "lazygit_0.52.0_Darwin_x86_64.tar.gz",
            "lazygit_0.52.0_freebsd_32-bit.tar.gz",
            "lazygit_0.52.0_freebsd_arm64.tar.gz",
            "lazygit_0.52.0_freebsd_armv6.tar.gz",
            "lazygit_0.52.0_freebsd_x86_64.tar.gz",
            "lazygit_0.52.0_Linux_32-bit.tar.gz",
            "lazygit_0.52.0_Linux_arm64.tar.gz",
            "lazygit_0.52.0_Linux_armv6.tar.gz",
            "lazygit_0.52.0_Linux_x86_64.tar.gz",
            "lazygit_0.52.0_Windows_32-bit.zip",
            "lazygit_0.52.0_Windows_arm64.zip",
            "lazygit_0.52.0_Windows_armv6.zip",
            "lazygit_0.52.0_Windows_x86_64.zip",
        ]
    }

    #[track_caller]
    fn assert_platform<'a>(
        patterns: &[regex::Regex],
        assets: &'a [&'a str],
        expected: Option<usize>,
    ) -> bool {
        let result = match_platform_names(patterns, assets);

        if let Some(index) = &result {
            eprintln!("    Matched: \"{}\" (index: {index})", assets[*index]);
        } else {
            eprintln!("    No match found");
        }

        if let Some(index) = &expected {
            eprintln!("    Expected: \"{}\"", assets[*index]);
        } else {
            eprintln!("    No match expected");
        }

        if result == expected {
            eprintln!("        OK");
            true
        } else {
            eprintln!("        FAILED");
            false
        }
    }

    fn platform_match_test(platforms: &[(Platform, usize)], names: &[&str], release_prefix: &str) {
        let mut platform_patterns = get_patterns_for(release_prefix);

        let mut result = true;
        for (platform, expected) in platforms {
            eprintln!(
                "Testing for expected matches on platform {platform} (expected index: {expected})"
            );
            result &= assert_platform(
                &platform_patterns.remove(platform).unwrap(),
                names,
                Some(*expected),
            );
        }

        for (platform, patterns) in platform_patterns {
            eprintln!("Testing for unexpected matches on platform {platform}");
            result &= assert_platform(&patterns, names, None);
        }

        if !result {
            panic!("Test failed");
        }
    }

    #[test]
    fn test_zoxide_names() {
        platform_match_test(
            &[
                (Platform::Linux32, 6),
                (Platform::Linux64, 9),
                (Platform::LinuxAarch64, 3),
                (Platform::Osx64, 7),
                (Platform::OsxArm64, 0),
                (Platform::Win64, 8),
                (Platform::WinArm64, 2),
            ],
            &zoxide_names(),
            "zoxide",
        );
    }

    #[test]
    fn test_stripe_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 7),
                (Platform::Linux64, 8),
                (Platform::OsxArm64, 9),
                (Platform::Osx64, 10),
                (Platform::Win32, 11),
                (Platform::Win64, 12),
            ],
            &stripe_names(),
            "stripe",
        );
    }

    #[test]
    fn test_atuin_names() {
        platform_match_test(
            &[
                (Platform::Linux64, 17),
                (Platform::LinuxAarch64, 7),
                (Platform::Osx64, 11),
                (Platform::OsxArm64, 1),
            ],
            &atuin_names(),
            "atuin",
        );
    }

    #[test]
    fn test_asm_lsp_names() {
        platform_match_test(
            &[
                (Platform::Linux64, 2),
                (Platform::Osx64, 1),
                (Platform::OsxArm64, 0),
            ],
            &asm_lsp_names(),
            "asm-lsp",
        );
    }

    #[test]
    fn test_cargo_binstall_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 14),
                (Platform::Linux64, 42),
                (Platform::Osx64, 30),
                (Platform::OsxArm64, 2),
                (Platform::Win64, 34),
                (Platform::WinArm64, 6),
            ],
            &cargo_binstall_names(),
            "cargo-binstall",
        );
    }

    #[test]
    fn test_bottom_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 12),
                (Platform::Linux32, 18),
                (Platform::Linux64, 29),
                (Platform::Osx64, 21),
                (Platform::OsxArm64, 9),
                (Platform::Win32, 16),
                (Platform::Win64, 23),
                (Platform::WinArm64, 10),
            ],
            &bottom_names(),
            "bottom",
        );
    }

    #[test]
    fn test_jjui_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 3),
                (Platform::Linux64, 2),
                (Platform::Osx64, 0),
                (Platform::OsxArm64, 1),
                (Platform::Win64, 4),
                (Platform::WinArm64, 5),
            ],
            &jjui_names(),
            "jjui",
        );
    }

    #[test]
    fn test_bazelisk_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 6),
                (Platform::Linux64, 5),
                (Platform::Osx64, 3),
                (Platform::OsxArm64, 4),
                (Platform::Win64, 7),
                (Platform::WinArm64, 8),
            ],
            &bazelisk_names(),
            "bazelisk",
        );
    }

    #[test]
    fn test_caligula_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 1),
                (Platform::Linux64, 3),
                (Platform::Osx64, 2),
                (Platform::OsxArm64, 0),
            ],
            &caligula_names(),
            "caligula",
        );
    }

    #[test]
    fn test_neovim_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 2),
                (Platform::Linux64, 5),
                (Platform::Osx64, 7),
                (Platform::OsxArm64, 6),
                (Platform::Win64, 11),
                (Platform::WinArm64, 9),
            ],
            &neovim_names(),
            "nvim",
        );
    }

    #[test]
    fn test_neovim_names_old() {
        platform_match_test(
            &[
                (Platform::Linux64, 0),
                (Platform::Win32, 2),
                (Platform::Win64, 3),
            ],
            &neovim_names_old(),
            "nvim",
        );
    }

    #[test]
    fn test_shellcheck_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 2),
                (Platform::Linux64, 5),
                (Platform::OsxArm64, 0),
                (Platform::Osx64, 1),
            ],
            &shellcheck_names(),
            "shellcheck",
        );
    }

    #[test]
    fn test_glsl_analyzer_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 0),
                (Platform::Linux64, 3),
                (Platform::OsxArm64, 1),
                (Platform::Osx64, 4),
                (Platform::WinArm64, 2),
                (Platform::Win64, 5),
            ],
            &glsl_analyzer_names(),
            "",
        );
    }

    #[test]
    fn test_lazygit_names() {
        platform_match_test(
            &[
                (Platform::Linux32, 6),
                (Platform::LinuxAarch64, 7),
                (Platform::Linux64, 9),
                (Platform::OsxArm64, 0),
                (Platform::Osx64, 1),
                (Platform::WinArm64, 11),
                (Platform::Win64, 13),
                (Platform::Win32, 10),
            ],
            &lazygit_names(),
            "lazygit",
        );
    }

    fn oxfmt_names() -> Vec<&'static str> {
        vec![
            "oxfmt-darwin-arm64",
            "oxfmt-darwin-arm64.tar.gz",
            "oxfmt-darwin-x64",
            "oxfmt-darwin-x64.tar.gz",
            "oxfmt-linux-arm64-gnu",
            "oxfmt-linux-arm64-gnu.tar.gz",
            "oxfmt-linux-arm64-musl",
            "oxfmt-linux-arm64-musl.tar.gz",
            "oxfmt-linux-x64-gnu",
            "oxfmt-linux-x64-gnu.tar.gz",
            "oxfmt-linux-x64-musl",
            "oxfmt-linux-x64-musl.tar.gz",
            "oxfmt-win32-arm64.exe",
            "oxfmt-win32-arm64.zip",
            "oxfmt-win32-x64.exe",
            "oxfmt-win32-x64.zip",
        ]
    }

    #[test]
    fn test_oxfmt_names() {
        platform_match_test(
            &[
                (Platform::LinuxAarch64, 6),
                (Platform::Linux64, 10),
                (Platform::OsxArm64, 0),
                (Platform::Osx64, 2),
                (Platform::Win64, 14),
                (Platform::WinArm64, 12),
            ],
            &oxfmt_names(),
            "oxfmt",
        );
    }

    fn hcloud_names() -> Vec<&'static str> {
        vec![
            "checksums.txt",
            "checksums.txt.sig",
            "hcloud-darwin-amd64.tar.gz",
            "hcloud-darwin-arm64.tar.gz",
            "hcloud-freebsd-386.tar.gz",
            "hcloud-freebsd-amd64.tar.gz",
            "hcloud-freebsd-arm64.tar.gz",
            "hcloud-freebsd-armv6.tar.gz",
            "hcloud-freebsd-armv7.tar.gz",
            "hcloud-linux-386.tar.gz",
            "hcloud-linux-amd64.tar.gz",
            "hcloud-linux-arm64.tar.gz",
            "hcloud-linux-armv6.tar.gz",
            "hcloud-linux-armv7.tar.gz",
            "hcloud-windows-386.zip",
            "hcloud-windows-amd64.zip",
            "hcloud-windows-arm64.zip",
        ]
    }

    #[test]
    fn test_hcloud_names() {
        platform_match_test(
            &[
                (Platform::Linux32, 9),
                (Platform::Linux64, 10),
                (Platform::LinuxAarch64, 11),
                (Platform::Osx64, 2),
                (Platform::OsxArm64, 3),
                (Platform::Win32, 14),
                (Platform::Win64, 15),
                (Platform::WinArm64, 16),
            ],
            &hcloud_names(),
            "",
        );
    }

    #[test]
    fn test_yaml_escape_plain_string() {
        assert_eq!(yaml_escape("hello world"), "hello world");
    }

    #[test]
    fn test_yaml_escape_double_quotes() {
        assert_eq!(yaml_escape(r#"say "hello""#), r#"say \"hello\""#);
    }

    #[test]
    fn test_yaml_escape_backslash() {
        assert_eq!(yaml_escape(r"foo\bar"), r"foo\\bar");
    }

    #[test]
    fn test_yaml_escape_newline() {
        assert_eq!(yaml_escape("line1\nline2"), r"line1\nline2");
    }

    #[test]
    fn test_yaml_escape_carriage_return() {
        assert_eq!(yaml_escape("line1\r\nline2"), r"line1\r\nline2");
    }

    #[test]
    fn test_yaml_escape_tab() {
        assert_eq!(yaml_escape("col1\tcol2"), r"col1\tcol2");
    }

    #[test]
    fn test_yaml_escape_combined() {
        assert_eq!(
            yaml_escape("path\\to\\\"file\"\tnotes\nmore"),
            r#"path\\to\\\"file\"\tnotes\nmore"#,
        );
    }

    #[test]
    fn test_fix_spdx_license_deprecated_gpl() {
        assert_eq!(fix_spdx_license("GPL-1.0"), "GPL-1.0-only");
        assert_eq!(fix_spdx_license("GPL-1.0+"), "GPL-1.0-or-later");
        assert_eq!(fix_spdx_license("GPL-2.0"), "GPL-2.0-only");
        assert_eq!(fix_spdx_license("GPL-2.0+"), "GPL-2.0-or-later");
        assert_eq!(fix_spdx_license("GPL-3.0"), "GPL-3.0-only");
        assert_eq!(fix_spdx_license("GPL-3.0+"), "GPL-3.0-or-later");
    }

    #[test]
    fn test_fix_spdx_license_deprecated_agpl() {
        assert_eq!(fix_spdx_license("AGPL-1.0"), "AGPL-1.0-only");
        assert_eq!(fix_spdx_license("AGPL-1.0+"), "AGPL-1.0-or-later");
        assert_eq!(fix_spdx_license("AGPL-3.0"), "AGPL-3.0-only");
        assert_eq!(fix_spdx_license("AGPL-3.0+"), "AGPL-3.0-or-later");
    }

    #[test]
    fn test_fix_spdx_license_deprecated_lgpl() {
        assert_eq!(fix_spdx_license("LGPL-2.0"), "LGPL-2.0-only");
        assert_eq!(fix_spdx_license("LGPL-2.0+"), "LGPL-2.0-or-later");
        assert_eq!(fix_spdx_license("LGPL-2.1"), "LGPL-2.1-only");
        assert_eq!(fix_spdx_license("LGPL-2.1+"), "LGPL-2.1-or-later");
        assert_eq!(fix_spdx_license("LGPL-3.0"), "LGPL-3.0-only");
        assert_eq!(fix_spdx_license("LGPL-3.0+"), "LGPL-3.0-or-later");
    }

    #[test]
    fn test_fix_spdx_license_deprecated_gfdl() {
        assert_eq!(fix_spdx_license("GFDL-1.1"), "GFDL-1.1-only");
        assert_eq!(fix_spdx_license("GFDL-1.2"), "GFDL-1.2-only");
        assert_eq!(fix_spdx_license("GFDL-1.3"), "GFDL-1.3-only");
    }

    #[test]
    fn test_fix_spdx_license_gpl_with_exception() {
        assert_eq!(
            fix_spdx_license("GPL-2.0-with-autoconf-exception"),
            "GPL-2.0-only WITH Autoconf-exception-2.0"
        );
        assert_eq!(
            fix_spdx_license("GPL-2.0-with-bison-exception"),
            "GPL-2.0-only WITH Bison-exception-2.2"
        );
        assert_eq!(
            fix_spdx_license("GPL-2.0-with-classpath-exception"),
            "GPL-2.0-only WITH Classpath-exception-2.0"
        );
        assert_eq!(
            fix_spdx_license("GPL-2.0-with-font-exception"),
            "GPL-2.0-only WITH Font-exception-2.0"
        );
        assert_eq!(
            fix_spdx_license("GPL-2.0-with-GCC-exception"),
            "GPL-2.0-only WITH GCC-exception-2.0"
        );
        assert_eq!(
            fix_spdx_license("GPL-3.0-with-autoconf-exception"),
            "GPL-3.0-only WITH Autoconf-exception-3.0"
        );
        assert_eq!(
            fix_spdx_license("GPL-3.0-with-GCC-exception"),
            "GPL-3.0-only WITH GCC-exception-3.1"
        );
    }

    #[test]
    fn test_fix_spdx_license_bsd_consolidation() {
        assert_eq!(fix_spdx_license("BSD-2-Clause-FreeBSD"), "BSD-2-Clause");
        assert_eq!(fix_spdx_license("BSD-2-Clause-NetBSD"), "BSD-2-Clause");
    }

    #[test]
    fn test_fix_spdx_license_other_renames() {
        assert_eq!(fix_spdx_license("bzip2-1.0.5"), "bzip2-1.0.6");
        assert_eq!(fix_spdx_license("StandardML-NJ"), "SMLNJ");
    }

    #[test]
    fn test_fix_spdx_license_passthrough() {
        assert_eq!(fix_spdx_license("MIT"), "MIT");
        assert_eq!(fix_spdx_license("Apache-2.0"), "Apache-2.0");
        assert_eq!(fix_spdx_license("BSD-3-Clause"), "BSD-3-Clause");
        assert_eq!(fix_spdx_license("GPL-2.0-only"), "GPL-2.0-only");
    }

    /// A combined release from oxc-project/oxc contains assets for both
    /// oxfmt and oxlint. Verify that the "oxfmt" release-prefix only
    /// matches oxfmt binaries, and "oxlint" only matches oxlint binaries.
    #[test]
    fn test_multi_package_repo_matches_disjoint_assets() {
        let combined_assets: Vec<&str> = vec![
            // oxfmt assets
            "oxfmt-darwin-arm64",
            "oxfmt-darwin-arm64.tar.gz",
            "oxfmt-darwin-x64",
            "oxfmt-darwin-x64.tar.gz",
            "oxfmt-linux-arm64-gnu",
            "oxfmt-linux-arm64-gnu.tar.gz",
            "oxfmt-linux-arm64-musl",
            "oxfmt-linux-arm64-musl.tar.gz",
            "oxfmt-linux-x64-gnu",
            "oxfmt-linux-x64-gnu.tar.gz",
            "oxfmt-linux-x64-musl",
            "oxfmt-linux-x64-musl.tar.gz",
            "oxfmt-win32-arm64.exe",
            "oxfmt-win32-arm64.zip",
            "oxfmt-win32-x64.exe",
            "oxfmt-win32-x64.zip",
            // oxlint assets
            "oxlint-darwin-arm64",
            "oxlint-darwin-arm64.tar.gz",
            "oxlint-darwin-x64",
            "oxlint-darwin-x64.tar.gz",
            "oxlint-linux-arm64-gnu",
            "oxlint-linux-arm64-gnu.tar.gz",
            "oxlint-linux-arm64-musl",
            "oxlint-linux-arm64-musl.tar.gz",
            "oxlint-linux-x64-gnu",
            "oxlint-linux-x64-gnu.tar.gz",
            "oxlint-linux-x64-musl",
            "oxlint-linux-x64-musl.tar.gz",
            "oxlint-win32-arm64.exe",
            "oxlint-win32-arm64.zip",
            "oxlint-win32-x64.exe",
            "oxlint-win32-x64.zip",
        ];

        let oxfmt_patterns = get_patterns_for("oxfmt");
        let oxlint_patterns = get_patterns_for("oxlint");

        for (platform, patterns) in &oxfmt_patterns {
            let matched = match_platform_names(patterns, &combined_assets);
            if let Some(idx) = matched {
                assert!(
                    combined_assets[idx].starts_with("oxfmt"),
                    "oxfmt patterns on {platform} matched oxlint asset: \"{}\"",
                    combined_assets[idx],
                );
            }
        }

        for (platform, patterns) in &oxlint_patterns {
            let matched = match_platform_names(patterns, &combined_assets);
            if let Some(idx) = matched {
                assert!(
                    combined_assets[idx].starts_with("oxlint"),
                    "oxlint patterns on {platform} matched oxfmt asset: \"{}\"",
                    combined_assets[idx],
                );
            }
        }
    }
}
