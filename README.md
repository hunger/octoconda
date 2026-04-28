# Octoconda

Octoconda is a CLI tool that automates the creation of [Conda](https://conda.io/) packages from
GitHub release binaries. It queries the GitHub API for releases, detects
platform-specific binaries using regex pattern matching, and generates
[rattler-build](https://prefix-dev.github.io/rattler-build/) recipes ready
for building.

This repository also acts as an Octoconda deployment (using the config at
[`config.toml`](./config.toml) to push to the channel
https://prefix.dev/channels/github-releases ). If you would like you
can deploy your own Octoconda by creating your own channel on [prefix.dev](https://prefix.dev/channels)
and pushing to it.

The tool checks an existing Conda channel for already-published versions to
avoid duplicates.

For best results use the GitHub action runner and do not run this directly! See [`.github/workflows/octoconda.yaml`](./.github/workflows/octoconda.yaml) for an example.

## Adding New Repositories

Use the `add-repo` script to discover and add new GitHub repositories to
`config.toml`. It can scan a web page for GitHub links or accept a single
repo URL directly. Repos are filtered: only those with binary release assets
(and not already in `config.toml`) are added.

```sh
# Scan a page for new repos and add them to config.toml
pixi run add-repo https://example.com/cool-cli-tools

# Add a single repo directly
pixi run add-repo https://github.com/owner/repo

# Preview without modifying config.toml
pixi run add-repo --dry-run https://example.com/cool-cli-tools

# Use a different config file
pixi run add-repo -c path/to/config.toml https://example.com/page
```

A `GITHUB_TOKEN` or `GH_TOKEN` environment variable (or `gh auth login`) is
recommended to avoid GitHub API rate limits.

## Testing a Single Repository

Use the `build-one` task to generate and build packages for a single
repository locally, without uploading anything. Results are placed in
`test-output/` (git-ignored).

```sh
pixi run build-one owner/repo
```

This runs Octoconda filtered to the given repo, then builds each generated
recipe with `rattler-build build`. Built packages end up in
`test-output/packages/`.

## Configuration File

The configuration file is TOML. It has two sections: a `[conda]` table and one
or more `[[packages]]` entries.

### `[conda]`

| Key                   | Required | Description                                                                                                                             |
| --------------------- | -------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `channel`             | yes      | Conda channel used to check for existing versions. Can be a short name (e.g. `github-releases`) or a full `https://prefix.dev/...` URL. |
| `max-import-releases` | no       | Maximum number of releases to import initially. Defaults to all releases releases.                                                      |

### `[[packages]]`

Each `[[packages]]` entry describes a GitHub repository whose releases should
be packaged.

| Key              | Required | Description                                                                                                                                                                                                                                                                                         |
| ---------------- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `repository`     | yes      | GitHub repository in `owner/repo` format.                                                                                                                                                                                                                                                           |
| `name`           | no       | Package name used in the Conda channel. Defaults to the repository name (the part after `/`).                                                                                                                                                                                                       |
| `release-prefix` | no       | Expected prefix of release binary filenames. Defaults to the package name. Set to `""` to disable prefix matching.                                                                                                                                                                                  |
| `tag-prefix`     | no       | Custom prefix to strip from release tags before version parsing. When set, only tags starting with this prefix are considered and the prefix is removed to extract the version.                                                                                                                     |
| `platforms`      | no       | Override the default platform detection patterns. See [Platform Patterns](#platform-patterns) below.                                                                                                                                                                                                |
| `expose`         | no       | List of extra top-level directory names to preserve in the conda package. By default only standard conda directories (`bin/`, `lib/`, `include/`, `share/`, `etc/`, `ssl/`) are kept; everything else is moved to `extras/`. Use for packages like JDKs that ship additional directories (e.g. `["conf", "jmods"]`). |

### Minimal Example

```toml
[conda]
channel = "github-releases"

[[packages]]
repository = "ajeetdsouza/zoxide"

[[packages]]
repository = "BurntSushi/ripgrep"
```

### Full Example

```toml
[conda]
channel = "https://prefix.dev/github-releases"

[[packages]]
repository = "oxc-project/oxc"
name = "oxlint"

[[packages]]
repository = "some-org/tool"
platforms = { linux-64 = ["custom-linux-x64-regex"], win-64 = "null" }
```

## Platform Patterns

Octoconda ships with built-in regex patterns that match common binary naming
conventions for each platform. The supported platforms are:

- `linux-32`, `linux-64`, `linux-aarch64`
- `osx-64`, `osx-arm64`
- `win-32`, `win-64`, `win-arm64`

The `platforms` table on a package entry lets you adjust matching per platform.
There are several forms:

**Disable a platform** -- set it to the string `"null"`:

```toml
[[packages]]
repository = "owner/repo"
platforms = { win-64 = "null" }
```

**Replace the default patterns** with a custom regex list:

```toml
[[packages]]
repository = "owner/repo"
platforms = { linux-64 = ["my-custom-regex-.*linux"] }
```

**Replace with a single regex** (when `name` is _not_ set):

```toml
[[packages]]
repository = "owner/repo"
platforms = { linux-64 = "my-custom-regex-.*linux" }
```

**Prepend the package name** to default patterns (when `name` _is_ set).
Providing a plain string while `name` is set prepends `<name>.*` to each
default pattern for that platform, effectively narrowing matching to assets
that start with the package name:

```toml
[[packages]]
repository = "oxc-project/oxc"
name = "oxlint"
platforms = { linux-64 = "" }
```

## Environment Variables

| Variable              | Description                                                      |
| --------------------- | ---------------------------------------------------------------- |
| `GITHUB_TOKEN`        | Personal access token for GitHub API authentication (preferred). |
| `GITHUB_ACCESS_TOKEN` | Alternative user access token for GitHub API authentication.     |

Without either token, API calls are made anonymously and subject to GitHub's
unauthenticated rate limit (~60 requests/hour).

## License

GPL-3.0-or-later
