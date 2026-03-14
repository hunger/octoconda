// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use anyhow::Context;

pub struct Github {
    octocrab: octocrab::Octocrab,
}

impl Github {
    pub fn new() -> anyhow::Result<Self> {
        let octocrab = if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            octocrab::OctocrabBuilder::default()
                .personal_token(token.clone())
                .build()
                .context("failed to set GITHUB_TOKEN")?
        } else if let Ok(token) = std::env::var("GITHUB_ACCESS_TOKEN") {
            octocrab::OctocrabBuilder::default()
                .user_access_token(token.clone())
                .build()
                .context("failed to set GITHUB_TOKEN")?
        } else {
            octocrab::OctocrabBuilder::default()
                .build()
                .context("Failed to build without authentication")?
        };

        Ok(Github { octocrab })
    }

    /// Fetch repository metadata (license, description, archived status, etc.).
    /// This is a separate API call — only invoke when you actually need to
    /// generate recipes, not just to check whether new versions exist.
    pub async fn get_repository(
        &self,
        repository: &crate::types::Repository,
    ) -> anyhow::Result<octocrab::models::Repository> {
        self.octocrab
            .repos(&repository.owner, &repository.repo)
            .get()
            .await
            .context("Failed to get repository data")
    }

    /// Fetch releases for a repository. Does **not** call `repo.get()` — use
    /// [`get_repository`] separately when repository metadata is needed.
    pub async fn query_releases(
        &self,
        repository: &crate::types::Repository,
        package_name: &str,
        max_import_releases: usize,
    ) -> anyhow::Result<Vec<(octocrab::models::repos::Release, (String, u32))>> {
        use std::collections::HashSet;
        use tokio_stream::StreamExt;

        let mut releases_result = Vec::new();
        // GitHub's offset-based pagination can return the same release on
        // consecutive pages when new releases are published during the fetch.
        // Deduplicate by (version, build_number) to avoid generating the same
        // recipe twice.
        let mut seen_versions: HashSet<(String, u32)> = HashSet::new();

        let repo = self.octocrab.repos(&repository.owner, &repository.repo);

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
            if tag.contains("prerelease")
                || tag.contains("alpha")
                || tag.contains("beta")
                || tag.contains("rc")
            {
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
                if seen_versions.insert((version.clone(), build_number)) {
                    releases_result.push((release, (version.clone(), build_number)));
                    if releases_result.len() >= max_import_releases {
                        return Ok(releases_result);
                    }
                }
            } else {
                continue;
            }
        }

        Ok(releases_result)
    }
}
