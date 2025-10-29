// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use octocrab::models::repos::Release;

use crate::types::Repository;

pub struct Github {
    octocrab: std::sync::Arc<octocrab::Octocrab>,
}

impl Github {
    pub fn new() -> Self {
        Github {
            octocrab: octocrab::instance(),
        }
    }

    pub async fn query_releases<F>(
        &self,
        repository: &Repository,
        callback: impl Fn(Release) -> F,
    ) -> Result<(), anyhow::Error>
    where
        F: Future<Output = Result<(), anyhow::Error>>,
    {
        use tokio_stream::StreamExt;

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
            callback(release).await?;
        }
        Ok(())
    }
}
