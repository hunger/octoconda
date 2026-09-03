#!/usr/bin/env python3
"""Download a web page and extract all GitHub repository URLs from it."""

import argparse
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

import requests
import tomllib
from bs4 import BeautifulSoup

# Matches github.com/<owner>/<repo> but not deeper paths like
# /owner/repo/issues or /owner/repo/blob/...
GITHUB_REPO_RE = re.compile(
    r"https?://github\.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+?)(?:\.git)?/?$"
)

GITHUB_SPECIAL_PATHS = {
    "about", "enterprise", "features", "login", "join", "pricing",
    "security", "customer-stories", "readme", "explore", "topics",
    "trending", "collections", "events", "sponsors", "settings",
    "marketplace", "notifications", "issues", "pulls", "codespaces",
    "discussions", "organizations", "orgs", "new", "apps", "site",
}


def is_repo_url(url: str) -> bool:
    m = GITHUB_REPO_RE.match(url)
    if not m:
        return False
    owner = m.group(1)
    if owner.lower() in GITHUB_SPECIAL_PATHS:
        return False
    return True


def normalize(url: str) -> str:
    """Normalize a GitHub repo URL to a canonical form."""
    url = url.rstrip("/")
    if url.endswith(".git"):
        url = url[:-4]
    if url.startswith("http://"):
        url = "https://" + url[7:]
    return url


def repo_slug(url: str) -> str:
    """Extract 'owner/repo' from a normalized GitHub URL."""
    m = GITHUB_REPO_RE.match(url)
    return f"{m.group(1)}/{m.group(2)}" if m else ""


def load_known_repos(config_path: str) -> tuple[set[str], set[str]]:
    """Load repository slugs and package names from config.toml.

    Returns (known_slugs, known_names) where names are the effective
    package names (explicit name or the repo part of the slug).
    """
    try:
        with open(config_path, "rb") as f:
            config = tomllib.load(f)
    except FileNotFoundError:
        print(f"Warning: config file '{config_path}' not found, not filtering.", file=sys.stderr)
        return set(), set()

    slugs = set()
    names = set()
    for pkg in config.get("packages", []):
        repo = pkg.get("repository", "")
        if repo:
            slugs.add(repo.lower())
            # Effective name: explicit name, or the repo part of the slug
            name = pkg.get("name") or repo.split("/", 1)[-1]
            names.add(name.lower())
    return slugs, names


def project_root() -> pathlib.Path:
    """Return the octoconda project root (parent of this scripts/ directory)."""
    return pathlib.Path(__file__).resolve().parent.parent


def octoconda_command() -> list[str]:
    """Return the command prefix to invoke octoconda.

    Prefers a prebuilt release binary; falls back to `cargo run --release`.
    """
    binary = project_root() / "target" / "release" / "octoconda"
    if binary.is_file():
        return [str(binary)]
    if shutil.which("cargo"):
        return ["cargo", "run", "--release", "--quiet", "--manifest-path",
                str(project_root() / "Cargo.toml"), "--"]
    print(
        "Error: octoconda binary not found at target/release/octoconda and "
        "`cargo` is not on PATH. Run `cargo build --release` first.",
        file=sys.stderr,
    )
    sys.exit(1)


def conda_channel_from_config(config_path: str) -> str:
    """Read [conda].channel from an existing config.toml."""
    with open(config_path, "rb") as f:
        cfg = tomllib.load(f)
    channel = cfg.get("conda", {}).get("channel")
    if not channel:
        print(
            f"Error: '{config_path}' is missing [conda].channel; "
            "octoconda needs it to check existing versions.",
            file=sys.stderr,
        )
        sys.exit(1)
    return channel


def check_with_octoconda(
    slugs: list[str], conda_channel: str,
) -> tuple[set[str], dict[str, str]]:
    """Run octoconda against the candidate slugs and return those that produce
    at least one recipe.

    Returns (passing_slugs, skip_reasons). `skip_reasons` maps each failing
    slug to a one-line reason from octoconda's report.
    """
    if not slugs:
        return set(), {}

    work_dir = pathlib.Path(tempfile.mkdtemp(prefix="octoconda-add-repo."))
    config_fd, config_path = tempfile.mkstemp(prefix="octoconda-add-repo.", suffix=".toml")
    try:
        with os.fdopen(config_fd, "w") as f:
            _write_check_config(f, slugs, conda_channel)

        cmd = octoconda_command() + [
            "--config-file", config_path,
            "--work-dir", str(work_dir),
        ]
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
        if result.returncode != 0:
            print(
                f"octoconda exited with status {result.returncode}; "
                f"stderr tail:\n{result.stderr[-2000:]}",
                file=sys.stderr,
            )
        status_path = work_dir / "status.txt"

        # Parse the report octoconda writes to status.txt to extract per-repo
        # skip reasons (GitHub errors, no platform binary, etc.). The format
        # comes from src/package_generation.rs::report_results.
        skip_reasons: dict[str, str] = {}
        if status_path.is_file():
            skip_reasons = parse_skip_reasons(status_path.read_text(), slugs)

        # A recipe lands at <work-dir>/<platform>/<package>-<version>-<build>/recipe.yaml.
        # Treat a slug as passing if any platform produced a recipe for its
        # default package name (lowercased repo basename).
        passing: set[str] = set()
        for slug in slugs:
            pkg_name = slug.split("/", 1)[1].lower()
            if any(work_dir.glob(f"*/{pkg_name}-*/recipe.yaml")):
                passing.add(slug)
        return passing, skip_reasons
    finally:
        try:
            os.unlink(config_path)
        except FileNotFoundError:
            pass
        shutil.rmtree(work_dir, ignore_errors=True)


_REPORT_SECTION_RE = re.compile(
    r"^(GitHub errors|Recipe generation failures|No platform binary in release|No GitHub releases)",
)


def parse_skip_reasons(report: str, slugs: list[str]) -> dict[str, str]:
    """Best-effort: scan octoconda's status report for owner/repo mentions
    and tag each one with the section heading it appeared under."""
    slug_set = {s.lower() for s in slugs}
    reasons: dict[str, str] = {}
    current_section: str | None = None
    for raw_line in report.splitlines():
        m = _REPORT_SECTION_RE.match(raw_line.lstrip())
        if m:
            current_section = m.group(1)
            continue
        if not current_section:
            continue
        for token in re.findall(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", raw_line):
            if token.lower() in slug_set and token not in reasons:
                reasons[token] = current_section

    # A repo with no GitHub releases at all is listed under the "No GitHub
    # releases" section (the generic token scan above already tags it, but
    # normalize the reason so it reads clearly).
    for slug in slugs:
        if reasons.get(slug) == "No GitHub releases":
            reasons[slug] = "no GitHub releases"
    return reasons


def _write_check_config(f, slugs: list[str], conda_channel: str) -> None:
    """Write the temporary octoconda config used by check_with_octoconda."""
    f.write("[conda]\n")
    f.write(f'channel = "{conda_channel}"\n')
    f.write("max-import-releases = 1\n\n")
    for slug in slugs:
        repo = slug.split("/", 1)[1]
        # Emit two variants per candidate so the check is agnostic to the tag
        # prefix convention:
        #   A: no tag-prefix -> matches `vX.Y.Z`, `<repo>_vX.Y.Z`,
        #      `<repo>-vX.Y.Z` (default auto-stripping)
        #   B: sub-package with tag-prefix = "<repo>_" and release-prefix ""
        #      -> matches `<repo>_vX.Y.Z` even when the asset names don't start
        #      with the repo name.
        # Variant B's explicit name must differ from variant A's implicit
        # name (the default package name for a bare entry is the repo
        # basename) or octoconda rejects the config as a duplicate.
        f.write("[[packages]]\n")
        f.write(f'repository = "{slug}"\n\n')
        f.write("[[packages]]\n")
        f.write(f'repository = "{slug}"\n')
        f.write("[[packages.packages]]\n")
        f.write(f'name = "{repo}-prefix"\n')
        f.write('release-prefix = ""\n')
        f.write(f'tag-prefix = "{repo}_"\n\n')


def add_repos_to_config(config_path: str, new_slugs: list[str],
                        known_names: set[str]) -> None:
    """Add new repos into config.toml, keeping [[packages]] sorted case-insensitively.

    If a new repo's default package name (the part after '/') collides with
    an existing name, an explicit name = "owner__repo" is added.
    """
    with open(config_path, "r") as f:
        content = f.read()

    # Split the file into a header (everything before the first [[packages]])
    # and individual package blocks.
    first_pkg = content.find("[[packages]]")
    if first_pkg == -1:
        header = content.rstrip("\n") + "\n"
        blocks = []
    else:
        header = content[:first_pkg]
        rest = content[first_pkg:]
        # Split on [[packages]] boundaries, keeping each block together
        raw_blocks = re.split(r"(?=^\[\[packages\]\])", rest, flags=re.MULTILINE)
        blocks = [b for b in raw_blocks if b.strip()]

    # Parse each block to extract its repository slug (for sorting)
    def block_repo(block: str) -> str:
        m = re.search(r'^repository\s*=\s*"([^"]+)"', block, re.MULTILINE)
        return m.group(1) if m else ""

    existing_slugs = {block_repo(b).lower() for b in blocks}

    # Track names as we add, to catch collisions between new repos too
    all_names = set(known_names)

    # Create new blocks for repos not already present
    for slug in new_slugs:
        if slug.lower() in existing_slugs:
            continue
        owner, repo = slug.split("/", 1)
        default_name = repo.lower()
        if default_name in all_names:
            # Name collision: use "owner__repo" as explicit name
            explicit_name = f"{owner}__{repo}"
            print(f"  name collision for '{repo}', using name = \"{explicit_name}\"",
                  file=sys.stderr)
            block = f'[[packages]]\nrepository = "{slug}"\nname = "{explicit_name}"\n'
            all_names.add(explicit_name.lower())
        else:
            block = f'[[packages]]\nrepository = "{slug}"\n'
            all_names.add(default_name)
        blocks.append(block)

    # Normalize trailing whitespace so blank lines don't accumulate
    blocks = [b.rstrip() for b in blocks]

    # Sort all blocks case-insensitively by repository
    blocks.sort(key=lambda b: block_repo(b).lower())

    with open(config_path, "w") as f:
        f.write(header)
        f.write("\n\n".join(blocks))
        f.write("\n")

    print(f"Added {len(new_slugs)} repo(s) to {config_path}", file=sys.stderr)


def _test_check_config_emits_both_prefix_variants():
    import io
    out = io.StringIO()
    _write_check_config(out, ["orhun/git-cliff"], "https://example.invalid/label")
    content = out.getvalue()
    # Variant A: bare entry without tag-prefix.
    assert 'repository = "orhun/git-cliff"\n\n' in content
    # Variant B: sub-package with the `<repo>_` tag prefix and disabled
    # asset-name prefix matching.
    assert 'tag-prefix = "git-cliff_"' in content
    assert 'release-prefix = ""' in content
    assert 'name = "git-cliff-prefix"' in content
    # Both variants must be distinct [[packages]] entries (two per slug).
    assert content.count('repository = "orhun/git-cliff"') == 2


def _test_parse_skip_reasons_sections():
    report = (
        "GitHub errors (1 packages):\n"
        "  https://github.com/owner/no-perm\n"
        "  owner/no-perm: rate limited\n"
        "\n"
        "No GitHub releases (2 packages):\n"
        "  jupyter/nbclassic\n"
        "  orhun/git-cliff\n"
        "\n"
    )
    reasons = parse_skip_reasons(report, ["owner/no-perm", "jupyter/nbclassic"])
    assert reasons["owner/no-perm"] == "GitHub errors", reasons
    assert reasons["jupyter/nbclassic"] == "no GitHub releases", reasons


def ensure_github_token() -> None:
    """Ensure GITHUB_TOKEN is set in this process's environment so the
    octoconda subprocess inherits it. Falls back to GH_TOKEN, then to
    `gh auth token`. Octoconda only reads GITHUB_TOKEN/GITHUB_ACCESS_TOKEN,
    so GH_TOKEN alone is not enough.
    """
    if os.environ.get("GITHUB_TOKEN"):
        return
    token = os.environ.get("GH_TOKEN")
    if not token:
        try:
            result = subprocess.run(
                ["gh", "auth", "token"], capture_output=True, text=True, timeout=5,
            )
            if result.returncode == 0 and result.stdout.strip():
                token = result.stdout.strip()
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass
    if token:
        os.environ["GITHUB_TOKEN"] = token
    else:
        print(
            "Warning: No GitHub token found. Octoconda will hit anonymous "
            "rate limits (~60 requests/hour).\n"
            "  Set GITHUB_TOKEN/GH_TOKEN or log in with: gh auth login",
            file=sys.stderr,
        )


def main():
    parser = argparse.ArgumentParser(
        description="Download a web page and extract GitHub repository URLs."
    )
    parser.add_argument(
        "urls", nargs="+",
        help="One or more URLs; each is either a GitHub repo URL (used directly) "
             "or a web page to scan for GitHub repo URLs.",
    )
    parser.add_argument(
        "-c", "--config",
        default="./config.toml",
        help="Path to config.toml for filtering known repos (default: ./config.toml)",
    )
    parser.add_argument(
        "-n", "--dry-run",
        action="store_true",
        help="Only print discovered repos, don't modify config.toml",
    )
    args = parser.parse_args()

    known, known_names = load_known_repos(args.config)

    repos: list[str] = []
    for url in args.urls:
        normalized = normalize(url)
        if is_repo_url(normalized):
            # A GitHub repo URL itself — use it directly.
            repos.append(normalized)
            continue
        # Otherwise treat the input as a web page to scan for GitHub URLs.
        resp = requests.get(
            url,
            headers={
                "User-Agent": "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0",
                "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            },
            timeout=30,
        )
        resp.raise_for_status()

        soup = BeautifulSoup(resp.text, "html.parser")

        # Extract links from <a href="...">
        href_urls = {a["href"] for a in soup.find_all("a", href=True)}

        # Also scan raw HTML for GitHub URLs not in href attributes
        raw_urls = set(re.findall(r"https?://github\.com/[A-Za-z0-9_./-]+", resp.text))

        candidates = href_urls | raw_urls
        repos.extend(sorted({normalize(u) for u in candidates if is_repo_url(u)}))

    # Filter out repos already in config.toml
    repos = [r for r in repos if repo_slug(r).lower() not in known]

    if not repos:
        print("No new GitHub repository URLs found.", file=sys.stderr)
        return

    # Deduplicate discovered URLs (case-insensitive, preserving order). This
    # matters for scraped pages where the same repo appears via several link
    # forms (http/https, .git suffix, trailing slash).
    seen_urls: set[str] = set()
    unique_repos: list[str] = []
    for r in repos:
        key = normalize(r).lower()
        if key not in seen_urls:
            seen_urls.add(key)
            unique_repos.append(normalize(r))

    # Deduplicate candidate slugs (preserving order).
    seen: set[str] = set()
    candidate_slugs: list[str] = []
    for r in unique_repos:
        slug = repo_slug(r)
        if slug.lower() not in seen:
            seen.add(slug.lower())
            candidate_slugs.append(slug)

    ensure_github_token()
    conda_channel = conda_channel_from_config(args.config)

    print(
        f"Checking {len(candidate_slugs)} repo(s) by running octoconda...",
        file=sys.stderr,
    )

    passing, skip_reasons = check_with_octoconda(candidate_slugs, conda_channel)

    for slug in candidate_slugs:
        if slug in passing:
            continue
        reason = skip_reasons.get(slug, "no recipe generated by octoconda")
        print(f"  skipped ({reason}): https://github.com/{slug}", file=sys.stderr)

    new_slugs = [s for s in candidate_slugs if s in passing]

    if not new_slugs:
        print("No new repos with packageable releases found.", file=sys.stderr)
        return

    for slug in sorted(new_slugs, key=str.lower):
        print(slug)

    if not args.dry_run:
        add_repos_to_config(args.config, new_slugs, known_names)
    else:
        print("(dry run, config.toml not modified)", file=sys.stderr)


def _run_self_tests():
    """Run the embedded unit tests (invoked via `pixi run test-add-repo`)."""
    import traceback

    failures = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("_test_") and callable(fn):
            try:
                fn()
                print(f"ok {name}")
            except Exception:
                failures += 1
                print(f"FAIL {name}")
                traceback.print_exc()
    if failures:
        sys.exit(1)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--self-test":
        _run_self_tests()
    else:
        main()
