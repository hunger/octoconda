# Octoconda: Automating Conda Packages from GitHub Releases

If you've ever wanted to install a tool from GitHub via Conda, you know the pain. Someone releases a great CLI tool with pre-built binaries, but there's no Conda package. You either download it manually, build from source, or hope someone maintains a conda-forge recipe. What if you could automate the entire process?

That's exactly what Octoconda does.

## The Problem

GitHub is full of useful command-line tools distributed as pre-compiled binaries. Developers release them for multiple platforms—Linux, macOS, Windows, across different architectures. But getting these into a Conda channel requires:

1. Downloading each release manually
2. Writing a recipe file with URLs and checksums
3. Creating build scripts to extract and install binaries
4. Repeating this for every new version
5. Doing it all again for each platform variant

This quickly becomes tedious, especially when tracking dozens or hundreds of tools.

## The Solution

Octoconda automates this entire workflow. Point it at a GitHub repository, and it:

- Fetches all releases via the GitHub API
- Matches platform-specific binaries using configurable patterns
- Generates Rattler Build recipes with correct URLs and SHA256 checksums
- Creates build scripts that handle various archive formats
- Skips versions already present in your target Conda channel

Configure once, run periodically, and your Conda channel stays up to date.

## How It Works

Octoconda operates as a pipeline:

```
TOML Config → GitHub API → Platform Detection → Recipe Generation → Build Scripts
```

You define repositories in a simple TOML configuration:

```toml
[conda]
channel = "github-releases"

[[packages]]
repository = "ajeetdsouza/zoxide"

[[packages]]
repository = "BurntSushi/ripgrep"

[[packages]]
repository = "sharkdp/bat"
```

Run Octoconda, and it queries GitHub for releases, determines which binaries match which platforms (linux-64, osx-arm64, win-64, etc.), and generates complete package recipes ready for building.

## Platform Detection

One of Octoconda's key features is intelligent platform matching. GitHub releases use inconsistent naming: some say `x86_64-linux`, others `linux-amd64`, and still others `Linux_x86_64`. Octoconda handles this through regex patterns that recognize common conventions:

- `x86_64-unknown-linux-musl` → linux-64
- `aarch64-apple-darwin` → osx-arm64
- `x86_64-pc-windows-msvc.zip` → win-64

For repositories with unusual naming, you can specify custom patterns in the configuration.

## Archive Handling

The generated build scripts handle the reality that different projects package their releases differently:

- `.tar.gz`, `.tar.xz`, `.tar.zst` archives
- `.zip` files
- Raw compressed binaries (`.gz`, `.xz`, `.zst`)
- Nested directory structures
- Executables with version numbers in the name

Octoconda's build scripts extract these, organize executables into `bin/`, move supporting files to `extras/`, and clean up version suffixes.

## Duplicate Prevention

Before generating a recipe, Octoconda checks if that version already exists in your target Conda channel. This makes it safe to run repeatedly—it only generates recipes for new releases.

## Real-World Scale

The project includes a configuration file with hundreds of repositories from the aqua registry, demonstrating its ability to handle package generation at scale. A self-imposed limit of ~250 packages per run keeps things manageable while still processing a substantial batch.

## Technology

Octoconda is written in Rust, using:

- **octocrab** for GitHub API interaction
- **rattler** for Conda channel queries
- **tokio** for async parallel processing
- **clap** for the CLI interface

The choice of Rust provides fast execution and reliable async handling for the many GitHub API calls required when processing large package lists.

## Getting Started

Clone the repository and build:

```bash
git clone https://github.com/hunger/octoconda
cd octoconda
cargo build --release
```

Create a configuration file:

```toml
[conda]
channel = "my-channel"

[[packages]]
repository = "owner/repo"
```

Run it:

```bash
./target/release/octoconda --config-file config.toml --work-dir ./output
```

The `output` directory will contain generated recipes ready for building with rattler-build.

## Conclusion

Octoconda removes the drudgery of packaging GitHub releases for Conda. Whether you maintain a private channel with a handful of tools or want to track hundreds of projects, it provides a straightforward path from GitHub release to Conda package.

The project is open source and available at [github.com/hunger/octoconda](https://github.com/hunger/octoconda).
