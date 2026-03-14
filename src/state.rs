// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Per-package state persisted across runs to reduce GitHub API calls.
///
/// The file is a simple JSON map of `"owner/repo"` → unix timestamp (seconds)
/// recording when we last queried the GitHub API for that package.
#[derive(Serialize, Deserialize, Default)]
pub struct State {
    packages: HashMap<String, i64>,
}

impl State {
    /// Load state from a JSON file.  Returns empty state on any error
    /// (missing file, corrupt JSON, permission denied, …).
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist state to a JSON file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json.as_bytes())?;
        Ok(())
    }

    /// Unix timestamp when this package was last checked, or 0 if unknown.
    pub fn last_checked(&self, repo_key: &str) -> i64 {
        self.packages.get(repo_key).copied().unwrap_or(0)
    }

    /// Record that we just checked this package.
    pub fn mark_checked(&mut self, repo_key: &str) {
        self.packages.insert(repo_key.to_string(), Self::now());
    }

    /// Current unix timestamp (seconds).
    pub fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }
}
