# Octoconda

Octoconda automates the creation of [Conda](https://conda.io/) packages from
GitHub release binaries. It queries the GitHub API for releases, detects
platform-specific binaries using regex pattern matching, and generates
[rattler-build](https://prefix-dev.github.io/rattler-build/) recipes ready
for building.

The tool checks an existing Conda channel for already-published versions to
avoid duplicates and caps each run at roughly 250 packages.

For best results: Use the github action runner and do not run this directly!

## Configuration File

The configuration file is TOML. It has two sections: a `[conda]` table and one
or more `[[packages]]` entries.

### `[conda]`

| Key | Required | Description |
|---|---|---|
| `channel` | yes | Conda channel used to check for existing versions. Can be a short name (e.g. `github-releases`) or a full `https://prefix.dev/...` URL. |

### `[[packages]]`

Each `[[packages]]` entry describes a GitHub repository whose releases should
be packaged.

| Key | Required | Description |
|---|---|---|
| `repository` | yes | GitHub repository in `owner/repo` format. |
| `name` | no | Package name used in the Conda channel. Defaults to the repository name (the part after `/`). |
| `platforms` | no | Override the default platform detection patterns. See [Platform Patterns](#platform-patterns) below. |

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

**Replace with a single regex** (when `name` is *not* set):

```toml
[[packages]]
repository = "owner/repo"
platforms = { linux-64 = "my-custom-regex-.*linux" }
```

**Prepend the package name** to default patterns (when `name` *is* set).
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

| Variable | Description |
|---|---|
| `GITHUB_TOKEN` | Personal access token for GitHub API authentication (preferred). |
| `GITHUB_ACCESS_TOKEN` | Alternative user access token for GitHub API authentication. |

Without either token, API calls are made anonymously and subject to GitHub's
unauthenticated rate limit (~60 requests/hour).

## License

GPL-3.0-or-later
