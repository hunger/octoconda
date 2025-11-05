// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::collections::HashMap;

mod cli;
mod conda;
mod config_file;
mod github;
mod package_generation;
mod types;

fn main() -> Result<(), anyhow::Error> {
    let cli = cli::parse_cli();
    eprintln!("{cli:#?}");

    let config = config_file::parse_config(&cli.config_file)?;
    eprintln!("{config:#?}");

    let temporary_directory = cli.work_directory()?;
    eprintln!("temporary dir: {}", temporary_directory.path().display());

    package_generation::generate_build_script(temporary_directory.path())?;
    package_generation::generate_env_file(temporary_directory.path(), &config)?;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let repo_packages = conda::get_conda_package_versions(
                &config.conda.full_channel()?,
                config.all_platforms().iter().copied(),
                config.packages.iter().map(|p| p.name.as_str()),
            )
            .await?;

            let gh = github::Github::new()?;

            let mut result = HashMap::new();

            for package in &config.packages {
                let repo_packages = &repo_packages;

                let Ok((repository, releases)) = gh.query_releases(&package.repository).await
                else {
                    result.insert(
                        package.name.clone(),
                        package_generation::PackagingStatus::github_failed(),
                    );
                    continue;
                };

                result.insert(
                    package.name.clone(),
                    package_generation::generate_packaging_data(
                        package,
                        &repository,
                        &releases,
                        repo_packages,
                        temporary_directory.path(),
                    )?,
                );
            }

            package_generation::report_results(&result);

            Ok(())
        })
}
