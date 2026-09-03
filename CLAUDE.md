# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Octoconda is a Rust CLI that turns GitHub release binaries into
[rattler-build](https://prefix-dev.github.io/rattler-build/) Conda recipes. It
queries the GitHub API for releases, matches platform-specific assets by
regex, checks a Conda channel for already-published versions, and writes a
recipe per (package, version, platform) into a work directory. `README.md`
documents the config-file schema — don't duplicate it here.

## Common commands

| Task | Command |
|---|---|
| Build | `cargo build` (release: `cargo build --release`) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` |
| Format | `cargo fmt` |
| Tests | `cargo test` |
| Single test | `cargo test <name>` (e.g. `cargo test test_multi_package_entry_expands`) |
| Generate + build one repo's recipes locally | `pixi run build-one owner/repo` (outputs under `test-output/`, no upload) |
| Discover new repos and append to `config.toml` | `pixi run add-repo <url>` (`--dry-run` to preview, `--full-test` to build and validate every platform first) |
| Find the most-starred repos octoconda can handle | `pixi run top-repos --count N [--language rust] [--query "topic:cli"] [--full-test] [--add]` (`--add` implies `--full-test`) |
| Validate a built `.conda` file | `pixi run -- python scripts/validate_conda_package.py <file>.conda` |
| Script self-tests | `pixi run test-add-repo`, `pixi run test-top-repos`, `pixi run test-validate-package` |
| Rebalance CI shard filter regexes | `pixi run rebalance-repos` |
| Workflow lint | `pixi run -- actionlint .github/workflows/` and `pixi run -- zizmor .github/workflows/` |

`pixi` provides Python, `rattler-build`, `actionlint`, and `zizmor` (see
`pixi.toml`). The Rust toolchain is not managed by pixi — use whatever
`rustup`/system toolchain is configured.

## Architecture

Entry point is `src/main.rs`, which drives a single end-to-end run:

1. Parse CLI (`cli.rs`) and config (`config_file.rs`).
2. Set up a work directory — either a user-supplied path or a `tempfile::TempDir` (auto-deleted unless `--keep-temporary-data`).
3. Write `build.sh` (embedded from `scripts/build.sh` via `include_str!`) and `env.sh` into the work dir.
4. Fetch **all** existing packages from the target Conda channel once (`conda.rs` → `rattler_repodata_gateway`).
5. Partition configured packages into "needs check" (fewer versions in the channel than `max-import-releases`) vs. "fully imported". Shuffle, then sort `needs_check` so brand-new packages come first; spot-check a sample of `fully_imported` using `state.rs` to rotate through stale ones over successive runs.
6. Group packages by repository and fan out with `stream::buffer_unordered(10)`. Per repo: list releases (`github.rs::fetch_releases` strips prereleases), filter per-package via `filter_releases_for_package`, and — only if something is new — fetch repo metadata once per group.
7. For each (package, release) pair with a platform match, `package_generation.rs` writes a rattler-build recipe. Status is collected into `PackageResult` and rendered into `status.txt` / `report.txt` (consumed by the GitHub Actions job summary).
8. If `--state-file` was supplied, persist last-checked timestamps.

### Module responsibilities

- `cli.rs` — `clap` parsing, `WorkDir` abstraction over temp vs. permanent dir.
- `config_file.rs` — TOML parsing **and** the built-in platform regex patterns (`default_platforms`). A `TomlPackage` may expand into multiple `Package` values when it has a `[[packages.packages]]` sub-list (sub-packages share a repo but have distinct names/prefixes/platform overrides). Duplicate package names are rejected unless at least one side is `deprecated = true`.
- `types.rs` — `Repository { owner, repo }` with `TryFrom<&str>`.
- `github.rs` — thin `octocrab` wrapper. `fetch_releases` does **not** call `repo.get()`; `get_repository` is a deliberately separate API call invoked only when new versions exist.
- `conda.rs` — one-shot query of the target channel via `rattler_repodata_gateway`. Results are sorted so `find_by_name` can use binary search.
- `state.rs` — JSON map of `owner/repo → unix timestamp`. Failures loading (missing file, corrupt JSON) silently fall back to empty state.
- `package_generation.rs` — the large file: regex-match release assets to platforms, render rattler-build recipes, and format the human-readable report. Embeds `scripts/build.sh` at compile time.

### Non-obvious behaviour

- **Platform override semantics** depend on whether `name` is set. See the README "Platform Patterns" section; `resolve_platforms` and `Package::platform_pattern` implement it.
- **Prerelease filtering** is a substring check on the tag (`prerelease`/`alpha`/`beta`/`rc`) — not semver-aware.
- **Version parsing for deduplication** (`filter_releases_for_package`) only accepts tags where the version is dots-and-digits after stripping an optional `{package_name}_` or `v` prefix. Anything else is skipped silently.
- **Conda channel query** disables sharded repodata (`sharded_enabled: false`) because prefix.dev channels don't serve sharded data for these use cases.
- **CI sharding**: `.github/workflows/octoconda.yaml` runs ~20 matrix jobs, each with a `--filter` regex over a slice of the alphabet. `scripts/rebalance_repos.py` regenerates those regex boundaries when the config grows.

### Recipe output layout

Recipes land at `<work-dir>/<platform>/<package>-<version>-<build>/recipe.yaml`. `build_one.sh`
discovers them with `find ... -name recipe.yaml` and runs
`rattler-build build` per recipe; the production workflow uses
`scripts/package_and_upload_all.sh` instead, which also uploads.

## Conventions specific to this repo

- GPL-3.0-or-later SPDX headers at the top of every Rust source file.
- `anyhow` for error propagation throughout (no `thiserror`).
- Tests live alongside the code as `#[cfg(test)] mod tests` — see `config_file.rs` for the pattern.
- Python scripts under `scripts/` target Python ≥3.14 (`compression.zstd` is needed to read `.conda` files) and use `requests` + `beautifulsoup4` (declared in `pixi.toml`, not a separate `requirements.txt`).
- Discovery scripts (`add_repo.py`, `mastodon_repos.py`, `top_repos.py`) never match asset names themselves; they call `check_with_octoconda` in `add_repo.py`, which runs the octoconda binary on a temporary config.
- `scripts/validate_conda_package.py` is the byte-level `.conda` validator used by `add_repo.py --full-test`; extend `PLATFORM_TARGETS` there when a platform is added to `config_file.rs::default_platforms`.
- When changing the built-in platform patterns, update `config_file.rs::default_platforms` and add a test using `tests::get_patterns_for`.
