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
    ) -> anyhow::Result<(
        octocrab::models::Repository,
        Vec<octocrab::models::repos::Release>,
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
            if release.tag_name.contains("prerelease")
                || release.tag_name.contains("alpha")
                || release.tag_name.contains("beta")
                || release.tag_name.contains('-')
            {
                eprintln!("pre-release tag: {}", release.tag_name);
                continue;
            }
            if (release.tag_name.as_bytes()[0] == b'v'
                && release.tag_name.as_bytes()[1] >= b'0'
                && release.tag_name.as_bytes()[1] <= b'9')
                || (release.tag_name.as_bytes()[0] >= b'0'
                    && release.tag_name.as_bytes()[0] <= b'9')
            {
                releases_result.push(release);
            } else {
                eprintln!("invalid tag: {}", release.tag_name);
                continue;
            }
        }

        Ok((repo_result, releases_result))
    }
}
