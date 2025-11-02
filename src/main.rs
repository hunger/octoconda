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

    let temporary_directory = cli.temporary_directory()?;
    eprintln!("temporary dir: {}", temporary_directory.path().display());

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let repo_packages = conda::get_conda_package_versions(
                &config.conda.channel,
                config.all_platforms().iter().copied(),
                config.packages.iter().map(|p| p.name.as_str()),
            )
            .await?;

            let gh = github::Github::new()?;

            let mut result = HashMap::new();

            for package in &config.packages {
                let repo_packages = &repo_packages;

                let Ok(releases) = gh.query_releases(&package.repository).await else {
                    result.insert(
                        package.name.clone(),
                        package_generation::PackagingStatus::github_failed(),
                    );
                    continue;
                };

                let package_dir = temporary_directory.path().join(&package.name);
                let Ok(_) = std::fs::create_dir(&package_dir) else {
                    result.insert(
                        package.name.clone(),
                        package_generation::PackagingStatus::package_dir_failed(),
                    );
                    continue;
                };

                result.insert(
                    package.name.clone(),
                    package_generation::generate_packaging_data(
                        package,
                        &releases,
                        repo_packages,
                        &package_dir,
                    )?,
                );
            }

            package_generation::report_results(&result);

            Ok(())
        })
}
