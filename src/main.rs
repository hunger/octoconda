// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

mod cli;
mod conda;
mod config_file;
mod github;
mod types;

fn main() -> Result<(), anyhow::Error> {
    let cli = cli::parse_cli();
    eprintln!("{cli:#?}");

    let config = config_file::parse_config(&cli.config_file)?;
    eprintln!("{config:#?}");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let _repo_packages = conda::get_conda_package_versions(
                cli.conda_channel,
                config.packages.iter().map(|p| p.name()),
            )
            .await?;

            let gh = github::Github::new();

            for package in config.packages {
                let name = package.name();
                match gh
                    .query_releases(&package.repository, move |release| {
                        eprintln!("{name}@{release:?}",);
                        async { Ok(()) }
                    })
                    .await
                {
                    Ok(_) => eprintln!("✔ {name}"),
                    Err(e) => eprintln!("❌ {name}\n  {e}"),
                }
            }

            Ok(())
        })
}
