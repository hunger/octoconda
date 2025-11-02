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
    ) -> Result<Vec<octocrab::models::repos::Release>, anyhow::Error> {
        use tokio_stream::StreamExt;

        let mut result = Vec::new();

        let stream = self
            .octocrab
            .repos(&repository.owner, &repository.repo)
            .releases()
            .list()
            .send()
            .await?
            .into_stream(&self.octocrab);

        tokio::pin!(stream);
        while let Some(release) = stream.try_next().await? {
            result.push(release);
        }

        Ok(result)
    }
}
