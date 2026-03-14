// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::str::FromStr;

use futures::stream::{self, StreamExt};
use rand::seq::SliceRandom;
use rattler_conda_types::VersionWithSource;

mod cli;
mod conda;
mod config_file;
mod github;
mod package_generation;
mod state;
mod types;

fn report_status(
    temporary_directory: &cli::WorkDir,
    result: &[package_generation::PackageResult],
    total_configured: usize,
    unknown_in_conda: &[String],
    max_releases_to_import: usize,
    platforms_count: usize,
) -> anyhow::Result<()> {
    let report = package_generation::report_results(
        result,
        total_configured,
        unknown_in_conda,
        max_releases_to_import,
        platforms_count,
    );
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
    let platform_count = config.all_platforms().len();
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

            let max_releases = config.conda.max_import_releases;

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

            // Load persistent state (empty if no --state-file or first run).
            let cached_state = cli
                .state_file
                .as_ref()
                .map(|p| state::State::load(p))
                .unwrap_or_default();

            // Partition into packages that need checking (new or incomplete)
            // vs fully-imported ones.
            packages.shuffle(&mut rand::rng());
            let (mut needs_check, mut fully_imported): (Vec<_>, Vec<_>) =
                packages.into_iter().partition(|p| {
                    let n = conda::find_by_name(&repo_packages, &p.name)
                        .iter()
                        .map(|r| &r.package_record.version)
                        .collect::<std::collections::HashSet<_>>()
                        .len();
                    n < max_releases
                });

            // Sort needs_check so brand-new packages (0 versions) come first.
            needs_check.sort_by_key(|p| {
                conda::find_by_name(&repo_packages, &p.name)
                    .iter()
                    .map(|r| &r.package_record.version)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            });

            let total_packages = needs_check.len() + fully_imported.len();

            // For fully-imported packages, sort by staleness (oldest checked
            // first) so we cycle through all of them over successive runs
            // instead of randomly sampling.  Take a limited batch per run.
            fully_imported.sort_by_key(|p| {
                let key = format!("{}/{}", p.repository.owner, p.repository.repo);
                cached_state.last_checked(&key)
            });
            let sample_count = (fully_imported.len() / 20)
                .max(10)
                .min(fully_imported.len());
            fully_imported.truncate(sample_count);
            eprintln!(
                "Processing {} packages ({} need updates, {} fully-imported spot-checks)",
                needs_check.len() + fully_imported.len(),
                needs_check.len(),
                fully_imported.len(),
            );

            // Process packages that need updates first, then spot-checks.
            needs_check.extend(fully_imported);
            let packages = needs_check;

            let result: Vec<package_generation::PackageResult> = stream::iter(packages)
                .map(|package| {
                    let gh = &gh;
                    let repo_packages = &repo_packages;
                    let work_dir = temporary_directory.path();
                    async move {
                        let repo_string =
                            format!("{}/{}", package.repository.owner, package.repository.repo);

                        let releases = match gh
                            .query_releases(&package.repository, &package.name, max_releases)
                            .await
                        {
                            Ok(r) => r,
                            Err(e) => {
                                return Ok(package_generation::PackageResult::GithubFailed {
                                    repository: package.repository.to_string(),
                                    message: format!("{e}"),
                                });
                            }
                        };

                        // Check if any release version is not yet in conda.
                        // If everything is already imported, skip the extra
                        // repo.get() API call.
                        let pkg_records = conda::find_by_name(repo_packages, &package.name);
                        let has_new = releases.iter().any(|(_, (vs, _))| {
                            let Ok(v) = rattler_conda_types::Version::from_str(vs) else {
                                return false;
                            };
                            let vws = VersionWithSource::new(v, vs);
                            !pkg_records.iter().any(|r| r.package_record.version == vws)
                        });

                        if !has_new && !releases.is_empty() {
                            return Ok(package_generation::PackageResult::Ok {
                                repository: repo_string,
                                name: package.name.clone(),
                                versions: vec![],
                            });
                        }

                        let repository = match gh.get_repository(&package.repository).await {
                            Ok(r) => r,
                            Err(e) => {
                                return Ok(package_generation::PackageResult::GithubFailed {
                                    repository: package.repository.to_string(),
                                    message: format!("{e}"),
                                });
                            }
                        };

                        if matches!(repository.archived, Some(true)) {
                            eprintln!(
                                "Note: Repository \"{}\" is *ARCHIVED*. \
                                 Consider to deprecate it.",
                                package.repository,
                            );
                        }

                        let versions = package_generation::generate_packaging_data(
                            package,
                            &repository,
                            &releases,
                            repo_packages,
                            work_dir,
                        )?;

                        Ok::<_, anyhow::Error>(package_generation::PackageResult::Ok {
                            repository: repo_string,
                            name: package.name.clone(),
                            versions,
                        })
                    }
                })
                .buffer_unordered(10)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<anyhow::Result<Vec<_>>>()?;

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
                config.conda.max_import_releases,
                platform_count,
            )?;

            // Persist state so the next run knows which packages were checked.
            if let Some(state_path) = &cli.state_file {
                let mut new_state = cached_state;
                for pkg in &result {
                    let key = match pkg {
                        package_generation::PackageResult::GithubFailed {
                            repository, ..
                        } => repository,
                        package_generation::PackageResult::Ok { repository, .. } => repository,
                    };
                    new_state.mark_checked(key);
                }
                new_state.save(state_path)?;
                eprintln!(
                    "State saved to {} ({} entries)",
                    state_path.display(),
                    result.len(),
                );
            }

            Ok(())
        })
}
