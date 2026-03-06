// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use rand::random_range;

use crate::package_generation::VersionPackagingStatus;

mod cli;
mod conda;
mod config_file;
mod github;
mod package_generation;
mod types;

fn report_status(
    temporary_directory: &cli::WorkDir,
    result: &[package_generation::PackageResult],
    total_configured: usize,
    unknown_in_conda: &[String],
) -> anyhow::Result<()> {
    let report = package_generation::report_results(result, total_configured, unknown_in_conda);
    eprintln!("{report}");

    let report = format!(
        r#"## Status

```
{report}
```

"#
    );

    std::fs::write(temporary_directory.status_file(), report.as_bytes())?;

    Ok(())
}

fn main() -> Result<(), anyhow::Error> {
    let cli = cli::parse_cli();

    let config = config_file::parse_config(&cli.config_file)?;
    let temporary_directory = cli.work_directory()?;

    package_generation::generate_build_script(temporary_directory.path())?;
    package_generation::generate_env_file(temporary_directory.path(), &config)?;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let repo_packages = conda::get_all_conda_packages(
                &config.conda.full_channel()?,
                config.all_platforms().iter().copied(),
            )
            .await?;

            let gh = github::Github::new()?;

            let mut result: Vec<package_generation::PackageResult> = vec![];

            let mut packages: Vec<_> = config
                .packages
                .iter()
                .filter(|p| {
                    cli.filter.as_ref().is_none_or(|re| {
                        let full_name = format!("{}/{}", p.repository.owner, p.repository.repo);
                        re.is_match(&full_name)
                    })
                })
                .collect();
            if !packages.is_empty() {
                let start = random_range(0..packages.len());
                packages.rotate_left(start);
            }

            let total_packages = packages.len();

            for package in packages {
                let repo_packages = &repo_packages;
                let repo_string =
                    format!("{}/{}", package.repository.owner, package.repository.repo);

                let (repository, releases) = match gh
                    .query_releases(
                        &package.repository,
                        &package.name,
                        config.conda.max_import_releases,
                    )
                    .await
                {
                    Ok((repository, releases)) => (repository, releases),
                    Err(e) => {
                        result.push(package_generation::PackageResult {
                            repository: repo_string,
                            name: package.name.clone(),
                            versions: vec![VersionPackagingStatus {
                                version: None,
                                status: package_generation::PackagingStatus::github_failed(
                                    &format!("{e:#}"),
                                ),
                            }],
                        });
                        continue;
                    }
                };

                if matches!(repository.archived, Some(true)) {
                    eprintln!(
                        "Note: Repository \"{}\" is *ARCHIVED*. Consider to deprecate it.",
                        package.repository,
                    );
                }

                let versions = package_generation::generate_packaging_data(
                    package,
                    &repository,
                    &releases,
                    repo_packages,
                    temporary_directory.path(),
                )?;

                result.push(package_generation::PackageResult {
                    repository: repo_string,
                    name: package.name.clone(),
                    versions,
                });
            }

            let configured_names: std::collections::HashSet<&str> =
                config.packages.iter().map(|p| p.name.as_str()).collect();
            let mut unknown_in_conda: Vec<String> = repo_packages
                .iter()
                .map(|r| r.package_record.name.as_normalized().to_string())
                .filter(|name| !configured_names.contains(name.as_str()))
                .collect();
            unknown_in_conda.sort();
            unknown_in_conda.dedup();

            report_status(
                &temporary_directory,
                &result,
                total_packages,
                &unknown_in_conda,
            )?;

            Ok(())
        })
}
