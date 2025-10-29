// SPDX-License-Identifier: GPL-3.0-or-later
// © Tobias Hunger <tobias.hunger@gmail.com>

#[derive(Clone, Debug)]
pub struct Repository {
    pub owner: String,
    pub repo: String,
}

impl TryFrom<&str> for Repository {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let Some((owner, repo)) = value.split_once('/') else {
            return Err(anyhow::anyhow!(
                "Can not parse {value} into a repository: No '/' to separate the owner from the repository"
            ));
        };
        if repo.contains('/') {
            return Err(anyhow::anyhow!(
                "Can not parse {value} into a repository: Too many '/"
            ));
        }
        Ok(Repository {
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }
}
