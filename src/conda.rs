// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use rattler_conda_types::{Channel, ChannelConfig, PackageName, Platform, RepoDataRecord};
use rattler_repodata_gateway::Gateway;

use std::path::PathBuf;

pub async fn get_conda_package_versions(
    channel: &str,
    platforms: impl Iterator<Item = Platform> + Clone,

    packages: impl Iterator<Item = &str>,
) -> Result<Vec<RepoDataRecord>, anyhow::Error> {
    let channel = Channel::from_str(
        channel,
        &ChannelConfig::default_with_root_dir(PathBuf::from(".")),
    )?;

    let specs = packages.map(|p| PackageName::try_from(p).expect("Invalid package name"));

    let repo_data = Gateway::new()
        .query(std::iter::once(channel), platforms, specs)
        .await?;

    let mut result = Vec::new();
    for rd in repo_data {
        for rdi in rd.iter() {
            result.push(rdi.clone())
        }
    }
    Ok(result)
}
