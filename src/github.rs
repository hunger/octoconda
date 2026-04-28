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

    /// Fetch all releases for a repository, filtering out prereleases.
    /// Does **not** call `repo.get()` — use [`get_repository`] separately
    /// when repository metadata is needed.
    pub async fn fetch_releases(
        &self,
        repository: &crate::types::Repository,
    ) -> anyhow::Result<Vec<octocrab::models::repos::Release>> {
        use tokio_stream::StreamExt;

        let repo = self.octocrab.repos(&repository.owner, &repository.repo);

        let stream = repo
            .releases()
            .list()
            .send()
            .await
            .context("Failed to retrieve list of releases")?
            .into_stream(&self.octocrab);

        let mut releases = Vec::new();
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
            releases.push(release);
        }

        Ok(releases)
    }
}

/// Filter raw releases for a specific package, extracting version info.
///
/// Strips the `{package_name}_` prefix from tags and parses versions.
/// Returns at most `max_import_releases` results, deduplicated by
/// (version, build_number).
pub fn filter_releases_for_package(
    releases: &[octocrab::models::repos::Release],
    package_name: &str,
    tag_prefix: Option<&str>,
    max_import_releases: usize,
) -> Vec<(octocrab::models::repos::Release, (String, u32))> {
    use std::collections::HashSet;

    let mut result = Vec::new();
    let mut seen_versions: HashSet<(String, u32)> = HashSet::new();

    for release in releases {
        let tag = &release.tag_name;

        let tag = if let Some(prefix) = tag_prefix {
            match tag.strip_prefix(prefix) {
                Some(t) => t.to_string(),
                None => continue,
            }
        } else if let Some(t) = tag
            .strip_prefix(&format!("{package_name}_"))
            .or_else(|| tag.strip_prefix(&format!("{package_name}-")))
        {
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
                result.push((release.clone(), (version.clone(), build_number)));
                if result.len() >= max_import_releases {
                    return result;
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_with_tag(tag: &str) -> octocrab::models::repos::Release {
        let value = serde_json::json!({
            "url": "https://example.invalid/",
            "html_url": "https://example.invalid/",
            "assets_url": "https://example.invalid/",
            "upload_url": "https://example.invalid/",
            "tarball_url": null,
            "zipball_url": null,
            "id": 1,
            "node_id": "n",
            "tag_name": tag,
            "target_commitish": "main",
            "name": null,
            "body": null,
            "draft": false,
            "prerelease": false,
            "created_at": null,
            "published_at": null,
            "author": null,
            "assets": [],
        });
        serde_json::from_value(value).expect("build Release")
    }

    #[test]
    fn parses_hyphenated_package_prefix() {
        // oven-sh/bun tags look like `bun-v1.3.13`.
        let releases = vec![release_with_tag("bun-v1.3.13")];
        let result = filter_releases_for_package(&releases, "bun", None, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, ("1.3.13".to_string(), 0));
    }

    #[test]
    fn parses_underscored_package_prefix() {
        let releases = vec![release_with_tag("foo_v2.0.0")];
        let result = filter_releases_for_package(&releases, "foo", None, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, ("2.0.0".to_string(), 0));
    }

    #[test]
    fn parses_bare_v_prefix() {
        let releases = vec![release_with_tag("v0.1.2")];
        let result = filter_releases_for_package(&releases, "whatever", None, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, ("0.1.2".to_string(), 0));
    }

    #[test]
    fn parses_build_suffix() {
        let releases = vec![release_with_tag("v1.2.3-4")];
        let result = filter_releases_for_package(&releases, "pkg", None, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, ("1.2.3".to_string(), 4));
    }

    #[test]
    fn rejects_non_version_tag() {
        let releases = vec![release_with_tag("nightly")];
        let result = filter_releases_for_package(&releases, "pkg", None, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn custom_tag_prefix_strips_and_parses() {
        let releases = vec![
            release_with_tag("jdk-25.0.2"),
            release_with_tag("jdk-23.0.2"),
        ];
        let result = filter_releases_for_package(&releases, "graalvm", Some("jdk-"), 10);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1, ("25.0.2".to_string(), 0));
        assert_eq!(result[1].1, ("23.0.2".to_string(), 0));
    }

    #[test]
    fn custom_tag_prefix_skips_non_matching_tags() {
        let releases = vec![
            release_with_tag("jdk-25.0.2"),
            release_with_tag("vm-22.3.3"),
        ];
        let result = filter_releases_for_package(&releases, "graalvm", Some("jdk-"), 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, ("25.0.2".to_string(), 0));
    }
}
