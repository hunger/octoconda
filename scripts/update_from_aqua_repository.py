#!/usr/bin/env python3
"""Extract github_release packages from aqua registry and add them to a TOML config file.

Ensures no duplicates and keeps repositories sorted.
"""

import sys
import tomllib
from pathlib import Path

import yaml


def extract_aqua_repos(aqua_registry: Path) -> set[str]:
    """Extract all github_release package repos from the aqua registry."""
    repos = set()
    pkgs_dir = aqua_registry / "pkgs"

    for registry_file in pkgs_dir.rglob("registry.yaml"):
        with open(registry_file) as f:
            data = yaml.safe_load(f)

        if not data or "packages" not in data:
            continue

        for pkg in data["packages"]:
            if pkg.get("type") == "github_release":
                repo_owner = pkg.get("repo_owner", "")
                repo_name = pkg.get("repo_name", "")
                if repo_owner and repo_name:
                    repos.add(f"{repo_owner}/{repo_name}")

    return repos


def update_config(config_path: Path, aqua_repos: set[str]) -> None:
    """Update the config TOML with new repos, deduplicating and sorting."""
    with open(config_path, "rb") as f:
        config = tomllib.load(f)

    existing_packages = config.get("packages", [])

    # Build a dict of (repo, name) -> package entry
    # Use (repository, name) as key to allow multiple entries for same repo with different names
    packages_by_key: dict[tuple[str, str | None], dict] = {}
    for pkg in existing_packages:
        repo = pkg.get("repository", "")
        if repo:
            name = pkg.get("name")
            key = (repo.lower(), name)
            if key not in packages_by_key:
                packages_by_key[key] = pkg

    # Add new repos from aqua (only if not already present)
    for repo in aqua_repos:
        if repo:
            key = (repo.lower(), None)
            if key not in packages_by_key:
                packages_by_key[key] = {"repository": repo}

    # Sort by repository name (case-insensitive), then by name
    sorted_packages = sorted(
        packages_by_key.values(),
        key=lambda p: (p.get("repository", "").lower(), p.get("name") or ""),
    )

    # Write back the config
    with open(config_path, "w") as f:
        # Write the conda section with all its fields
        conda_section = config.get("conda", {})
        if conda_section:
            f.write("[conda]\n")
            for key, value in conda_section.items():
                if isinstance(value, str):
                    f.write(f'{key} = "{value}"\n')
                else:
                    f.write(f"{key} = {value}\n")
            f.write("\n")

        for pkg in sorted_packages:
            f.write("[[packages]]\n")
            f.write(f'repository = "{pkg["repository"]}"\n')
            if "name" in pkg:
                f.write(f'name = "{pkg["name"]}"\n')
            # Write any other fields
            for key, value in pkg.items():
                if key not in ("repository", "name"):
                    if isinstance(value, str):
                        f.write(f'{key} = "{value}"\n')
                    else:
                        f.write(f"{key} = {value}\n")
            f.write("\n")

    print(f"Updated {config_path} with {len(sorted_packages)} packages")


def main() -> int:
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <path-to-aqua-registry> <config.toml>", file=sys.stderr)
        return 1

    aqua_registry = Path(sys.argv[1]).resolve()
    config_path = Path(sys.argv[2])

    # Validate aqua registry
    if not aqua_registry.is_dir():
        print(f"Error: Directory '{sys.argv[1]}' does not exist", file=sys.stderr)
        return 1

    if not (aqua_registry / "aqua-policy.yaml").is_file():
        print(f"Error: '{aqua_registry}' does not contain aqua-policy.yaml", file=sys.stderr)
        return 1

    # Validate config file
    if not config_path.is_file():
        print(f"Error: Config file '{config_path}' does not exist", file=sys.stderr)
        return 1

    # Extract repos from aqua and update config
    aqua_repos = extract_aqua_repos(aqua_registry)
    update_config(config_path, aqua_repos)

    return 0


if __name__ == "__main__":
    sys.exit(main())
