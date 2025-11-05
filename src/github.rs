// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use anyhow::Context;

pub struct Github {
    octocrab: std::sync::Arc<octocrab::Octocrab>,
}

impl Github {
    pub fn new() -> anyhow::Result<Self> {
        let octocrab = if let Ok(token) = std::env::var("GITHUB_ACCESS_TOKEN") {
            octocrab::initialise(
                octocrab::Octocrab::default()
                    .user_access_token(token)
                    .context("failed to se github access token")?,
            )
        } else {
            octocrab::instance()
        };
        Ok(Github { octocrab })
    }

    pub async fn query_releases(
        &self,
        repository: &crate::types::Repository,
    ) -> Result<
        (
            octocrab::models::Repository,
            Vec<octocrab::models::repos::Release>,
        ),
        anyhow::Error,
    > {
        use tokio_stream::StreamExt;

        let mut releases_result = Vec::new();

        let repo = self.octocrab.repos(&repository.owner, &repository.repo);
        let repo_result = repo.get().await?;

        let stream = repo
            .releases()
            .list()
            .send()
            .await?
            .into_stream(&self.octocrab);

        tokio::pin!(stream);
        while let Some(release) = stream.try_next().await? {
            releases_result.push(release);
        }

        Ok((repo_result, releases_result))
    }
}
