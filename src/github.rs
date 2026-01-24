// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use anyhow::Context;

pub struct Github {
    octocrab: octocrab::Octocrab,
}

impl Github {
    pub fn new() -> anyhow::Result<Self> {
        let octocrab = if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            eprintln!("Github with personal token authentication");
            octocrab::OctocrabBuilder::default()
                .personal_token(token.clone())
                .build()
                .context("failed to set GITHUB_TOKEN")?
        } else if let Ok(token) = std::env::var("GITHUB_ACCESS_TOKEN") {
            eprintln!("Github with user access token authentication");
            octocrab::OctocrabBuilder::default()
                .user_access_token(token.clone())
                .build()
                .context("failed to set GITHUB_TOKEN")?
        } else {
            eprintln!("Github without authentication");
            octocrab::OctocrabBuilder::default()
                .build()
                .context("Failed to build without authentication")?
        };

        Ok(Github { octocrab })
    }

    pub async fn query_releases(
        &self,
        repository: &crate::types::Repository,
        package_name: &str,
    ) -> anyhow::Result<(
        octocrab::models::Repository,
        Vec<(octocrab::models::repos::Release, (String, u32))>,
    )> {
        use tokio_stream::StreamExt;

        let mut releases_result = Vec::new();

        let repo = self.octocrab.repos(&repository.owner, &repository.repo);
        let repo_result = repo.get().await.context("Failed to get repository data")?;

        let stream = repo
            .releases()
            .list()
            .send()
            .await
            .context("Failed to retrieve list of releases")?
            .into_stream(&self.octocrab);

        tokio::pin!(stream);
        while let Some(release) = stream.try_next().await? {
            let tag = &release.tag_name;
            if tag.contains("prerelease") || tag.contains("alpha") || tag.contains("beta") {
                eprintln!("pre-release tag: {}", tag);
                continue;
            }

            let tag = if let Some(t) = tag.strip_prefix(&format!("{package_name}_")) {
                t.to_string()
            } else {
                tag.to_string()
            };
            let tag = if let Some(t) = tag.strip_prefix('v') {
                t.to_string()
            } else {
                tag
            };

            let (version, build) = if let Some((version, build)) = tag.split_once('-') {
                (version.to_string(), build.to_string())
            } else {
                (tag, String::new())
            };

            if version.chars().all(|c| c.is_ascii_digit() || c == '.')
                && (build.is_empty() || build.chars().any(|c| c.is_ascii_digit()))
            {
                let build_number: u32 = build.parse().unwrap_or(0);
                releases_result.push((release, (version, build_number)));
            } else {
                eprintln!("Invalid version when looking at {package_name}: {version} ({build})");
                continue;
            }
        }

        Ok((repo_result, releases_result))
    }
}
