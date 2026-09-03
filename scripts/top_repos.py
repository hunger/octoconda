#!/usr/bin/env python3
"""Find the most-starred GitHub repositories whose releases octoconda can package.

Usage:
    pixi run top-repos
    # -> prints the 500 most-starred repos (one https://github.com/owner/repo
    #    URL per line) that are not yet in config.toml and for which
    #    octoconda generates at least one recipe.

    pixi run top-repos --count 50 --language rust
    pixi run top-repos --language go --language zig --query "topic:cli"
    pixi run top-repos --count 20 --add
    # -> also builds every platform of each candidate with rattler-build and
    #    validates the packages (add_repo.py --full-test); only repositories
    #    that pass on all platforms count and are written to config.toml.

Repositories are read from GitHub's search API in descending star order and
prefiltered cheaply (not archived, not a fork, has a stable release with at
least one asset). Whether octoconda can actually handle a repository is
decided by octoconda itself: candidates are checked in batches via
scripts/add_repo.py, which runs the octoconda binary on a temporary config.

A GitHub token is required (GraphQL has no anonymous access); see
`ensure_github_token` in add_repo.py for where it is taken from.
"""

import argparse
import datetime
import os
import sys
import time
from collections.abc import Iterator

import requests

# add_repo.py lives next to this file; make it importable regardless of cwd.
_SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

from add_repo import (
    add_repos_to_config,
    check_with_octoconda,
    conda_channel_from_config,
    ensure_github_token,
    load_known_repos,
)

GRAPHQL_URL = "https://api.github.com/graphql"

# GitHub search never returns more than this many results for one query, so
# the scan is split into star windows once a window is exhausted.
SEARCH_RESULT_CAP = 1000
PAGE_SIZE = 100

# How many of the newest releases to inspect for the "has binaries" prefilter.
# Projects often publish a prerelease with no assets on top of a stable
# release that has them.
RELEASES_TO_INSPECT = 5

SEARCH_QUERY = """
query($q: String!, $after: String, $pageSize: Int!, $releases: Int!) {
  rateLimit { remaining resetAt }
  search(query: $q, type: REPOSITORY, first: $pageSize, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on Repository {
        nameWithOwner
        stargazerCount
        isArchived
        isFork
        primaryLanguage { name }
        releases(first: $releases, orderBy: {field: CREATED_AT, direction: DESC}) {
          nodes {
            isPrerelease
            isDraft
            releaseAssets(first: 1) { totalCount }
          }
        }
      }
    }
  }
}
"""

REQUEST_TIMEOUT = 60
MAX_RETRIES = 5


class GitHubSearchError(RuntimeError):
    """Raised when the GraphQL search cannot be completed."""


def build_search_query(min_stars: int, max_stars: int | None, languages: list[str], extra: str) -> str:
    """Compose the GitHub search string for one star window.

    Several `language:` qualifiers are OR-ed by GitHub, so `--language rust
    --language go` finds repositories whose primary language is either.
    """
    stars = f"stars:>={min_stars}" if max_stars is None else f"stars:{min_stars}..{max_stars}"
    parts = [stars]
    parts += [f"language:{lang}" for lang in languages]
    if extra:
        parts.append(extra)
    parts.append("sort:stars-desc")
    return " ".join(parts)


def _post_graphql(session: requests.Session, variables: dict) -> dict:
    """Run SEARCH_QUERY once, retrying on transient errors and rate limits."""
    for attempt in range(1, MAX_RETRIES + 1):
        try:
            response = session.post(
                GRAPHQL_URL, json={"query": SEARCH_QUERY, "variables": variables},
                timeout=REQUEST_TIMEOUT,
            )
        except requests.RequestException as e:
            if attempt == MAX_RETRIES:
                raise GitHubSearchError(f"GitHub GraphQL request failed: {e}") from e
            time.sleep(2**attempt)
            continue

        if response.status_code in (403, 429, 502, 503):
            wait = int(response.headers.get("Retry-After", 2**attempt))
            print(f"  GitHub answered {response.status_code}; retrying in {wait}s", file=sys.stderr)
            time.sleep(wait)
            continue
        if response.status_code != 200:
            raise GitHubSearchError(
                f"GitHub GraphQL request failed with status {response.status_code}: "
                f"{response.text[:500]}"
            )
        payload = response.json()
        if payload.get("errors"):
            messages = "; ".join(str(e.get("message", e)) for e in payload["errors"])
            if "rate limit" in messages.lower() and attempt < MAX_RETRIES:
                time.sleep(60)
                continue
            raise GitHubSearchError(f"GitHub GraphQL query failed: {messages}")
        data = payload.get("data")
        if not data:
            raise GitHubSearchError(f"GitHub GraphQL returned no data: {payload!r}")
        return data
    raise GitHubSearchError("GitHub GraphQL request failed after retries")


def _wait_for_rate_limit(rate_limit: dict | None) -> None:
    if not rate_limit or rate_limit.get("remaining", 1) > 2:
        return
    reset_at = datetime.datetime.fromisoformat(rate_limit["resetAt"].replace("Z", "+00:00"))
    wait = max(1, int((reset_at - datetime.datetime.now(datetime.UTC)).total_seconds()) + 1)
    print(f"  GraphQL rate limit exhausted; sleeping {wait}s", file=sys.stderr)
    time.sleep(wait)


def iter_repositories(
    session: requests.Session, *, min_stars: int, languages: list[str], extra_query: str,
) -> Iterator[dict]:
    """Yield repository nodes in descending star order, down to `min_stars`.

    Works around the 1000-result search cap by restarting the query with
    `stars:min..<last seen count>` whenever a window is exhausted; repositories
    that reappear at the window boundary are skipped.
    """
    seen: set[str] = set()
    max_stars: int | None = None
    while True:
        query = build_search_query(min_stars, max_stars, languages, extra_query)
        after = None
        fetched_in_window = 0
        new_in_window = 0
        last_stars: int | None = None
        while True:
            data = _post_graphql(session, {
                "q": query, "after": after, "pageSize": PAGE_SIZE, "releases": RELEASES_TO_INSPECT,
            })
            search = data["search"]
            for node in search["nodes"]:
                if not node:
                    continue
                fetched_in_window += 1
                last_stars = node["stargazerCount"]
                slug = node["nameWithOwner"]
                if slug.lower() in seen:
                    continue
                seen.add(slug.lower())
                new_in_window += 1
                yield node
            _wait_for_rate_limit(data.get("rateLimit"))
            if not search["pageInfo"]["hasNextPage"] or fetched_in_window >= SEARCH_RESULT_CAP:
                break
            after = search["pageInfo"]["endCursor"]

        if fetched_in_window < SEARCH_RESULT_CAP or last_stars is None:
            return  # the window was not full, so the query is exhausted
        if new_in_window == 0:
            print(
                f"Warning: more than {SEARCH_RESULT_CAP} repositories have exactly "
                f"{last_stars} stars; GitHub search cannot enumerate them all.",
                file=sys.stderr,
            )
            return
        if last_stars < min_stars:
            return
        max_stars = last_stars


def has_stable_release_with_assets(node: dict) -> bool:
    """Cheap prefilter mirroring what octoconda needs: a non-prerelease,
    non-draft release with at least one asset among the newest releases."""
    if node.get("isArchived") or node.get("isFork"):
        return False
    releases = (node.get("releases") or {}).get("nodes") or []
    return any(
        not r.get("isPrerelease") and not r.get("isDraft")
        and (r.get("releaseAssets") or {}).get("totalCount", 0) > 0
        for r in releases
    )


def take_batch(candidates: list[dict], size: int) -> tuple[list[dict], list[dict]]:
    """Split `candidates` into a batch to check now and the remainder.

    The temporary config written by check_with_octoconda derives package
    names from the repository basename, and octoconda rejects duplicate
    names, so at most one repository per basename goes into a batch; the
    others wait for a later batch.
    """
    batch: list[dict] = []
    rest: list[dict] = []
    basenames: set[str] = set()
    for node in candidates:
        basename = node["nameWithOwner"].split("/", 1)[1].lower()
        if len(batch) < size and basename not in basenames:
            basenames.add(basename)
            batch.append(node)
        else:
            rest.append(node)
    return batch, rest


def find_top_repos(
    session: requests.Session,
    *,
    count: int,
    min_stars: int,
    languages: list[str],
    extra_query: str,
    batch_size: int,
    known_slugs: set[str],
    conda_channel: str,
    full_test: bool = False,
) -> tuple[list[dict], int]:
    """Return the `count` most-starred repositories octoconda can handle.

    With `full_test`, a repository only counts if every recipe octoconda
    generates for it also builds and validates (see add_repo.full_test_slugs).

    Returns (repositories, skipped_known), the repositories being the GraphQL
    nodes of the passing candidates in star order. Repositories already in
    the config are skipped and only counted.
    """
    found: list[dict] = []
    pending: list[dict] = []
    scanned = 0
    skipped_known = 0
    checked = 0
    exhausted = False
    repos = iter_repositories(
        session, min_stars=min_stars, languages=languages, extra_query=extra_query,
    )

    def run_batch(batch: list[dict]) -> None:
        nonlocal checked
        slugs = [n["nameWithOwner"] for n in batch]
        passing, reasons = check_with_octoconda(slugs, conda_channel, full_test=full_test)
        checked += len(batch)
        for slug, reason in reasons.items():
            if reason.startswith("full test failed"):
                print(f"  skipped ({reason}): https://github.com/{slug}", file=sys.stderr)
        # Preserve star order: `batch` is already sorted by stars descending.
        found.extend(n for n in batch if n["nameWithOwner"] in passing)
        print(
            f"  scanned {scanned} repos (down to {batch[-1]['stargazerCount']} stars), "
            f"checked {checked}, found {len(found)}/{count}",
            file=sys.stderr,
        )

    while len(found) < count:
        # Near the end only a few more hits are needed; keep the last batches
        # small so octoconda does not check far more repositories than asked
        # for (roughly every second candidate passes in practice).
        want = min(batch_size, max(10, 2 * (count - len(found))))
        while len(pending) < want and not exhausted:
            node = next(repos, None)
            if node is None:
                exhausted = True
                break
            scanned += 1
            if node["nameWithOwner"].lower() in known_slugs:
                skipped_known += 1
                continue
            if has_stable_release_with_assets(node):
                pending.append(node)
        if not pending:
            break
        batch, pending = take_batch(pending, want)
        run_batch(batch)
        if exhausted and not pending:
            break

    return found[:count], skipped_known


def _test_build_search_query():
    assert build_search_query(100, None, [], "") == "stars:>=100 sort:stars-desc"
    assert build_search_query(100, 5000, ["rust", "go"], "topic:cli") == (
        "stars:100..5000 language:rust language:go topic:cli sort:stars-desc"
    )


def _test_has_stable_release_with_assets():
    def node(**overrides):
        base = {
            "isArchived": False, "isFork": False,
            "releases": {"nodes": [
                {"isPrerelease": True, "isDraft": False, "releaseAssets": {"totalCount": 3}},
                {"isPrerelease": False, "isDraft": False, "releaseAssets": {"totalCount": 2}},
            ]},
        }
        base.update(overrides)
        return base

    assert has_stable_release_with_assets(node())
    assert not has_stable_release_with_assets(node(isArchived=True))
    assert not has_stable_release_with_assets(node(isFork=True))
    assert not has_stable_release_with_assets(node(releases={"nodes": []}))
    assert not has_stable_release_with_assets(node(releases=None))
    only_prerelease = {"nodes": [
        {"isPrerelease": True, "isDraft": False, "releaseAssets": {"totalCount": 3}},
    ]}
    assert not has_stable_release_with_assets(node(releases=only_prerelease))
    no_assets = {"nodes": [
        {"isPrerelease": False, "isDraft": False, "releaseAssets": {"totalCount": 0}},
    ]}
    assert not has_stable_release_with_assets(node(releases=no_assets))


def _test_take_batch_defers_duplicate_basenames():
    nodes = [{"nameWithOwner": s} for s in ["a/cli", "b/tool", "c/CLI", "d/other", "e/more"]]
    batch, rest = take_batch(nodes, 3)
    assert [n["nameWithOwner"] for n in batch] == ["a/cli", "b/tool", "d/other"], batch
    assert [n["nameWithOwner"] for n in rest] == ["c/CLI", "e/more"], rest
    batch, rest = take_batch(rest, 3)
    assert [n["nameWithOwner"] for n in batch] == ["c/CLI", "e/more"]
    assert rest == []


def _test_find_top_repos_stops_at_count():
    import unittest.mock

    pages = [
        {"nameWithOwner": f"o/r{i}", "stargazerCount": 1000 - i, "isArchived": False,
         "isFork": False, "primaryLanguage": {"name": "Rust"},
         "releases": {"nodes": [
             {"isPrerelease": False, "isDraft": False, "releaseAssets": {"totalCount": 1}},
         ]}}
        for i in range(10)
    ]
    # o/r3 is already configured, o/r5 has no assets, and octoconda rejects o/r1.
    pages[5]["releases"]["nodes"][0]["releaseAssets"]["totalCount"] = 0

    def fake_check(slugs, _channel, full_test=False):
        assert full_test is True
        return {s for s in slugs if s != "o/r1"}, {"o/r1": "full test failed: linux-64: boom"}

    with unittest.mock.patch(f"{__name__}.iter_repositories", return_value=iter(pages)), \
            unittest.mock.patch(f"{__name__}.check_with_octoconda", side_effect=fake_check):
        found, skipped_known = find_top_repos(
            None, count=4, min_stars=1, languages=[], extra_query="", batch_size=3,
            known_slugs={"o/r3"}, conda_channel="https://example.invalid/c", full_test=True,
        )
    assert [n["nameWithOwner"] for n in found] == ["o/r0", "o/r2", "o/r4", "o/r6"], found
    assert skipped_known == 1


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Find the most-starred GitHub repositories whose releases "
                    "octoconda can package.",
    )
    parser.add_argument(
        "-n", "--count", type=int, default=500,
        help="How many repositories to find (default: 500)",
    )
    parser.add_argument(
        "-c", "--config", default="./config.toml",
        help="config.toml used to skip known repos and to read the conda channel "
             "(default: ./config.toml)",
    )
    parser.add_argument(
        "--language", action="append", default=[], metavar="LANG",
        help="Only repositories whose primary language is LANG (GitHub's "
             "`language:` qualifier, e.g. rust, go, zig). Repeat to allow several.",
    )
    parser.add_argument(
        "--query", default="", metavar="QUALIFIERS",
        help="Extra GitHub search qualifiers appended verbatim, e.g. "
             "'topic:cli' or 'pushed:>2025-01-01'",
    )
    parser.add_argument(
        "--min-stars", type=int, default=50,
        help="Stop scanning below this many stars (default: 50)",
    )
    parser.add_argument(
        "--batch-size", type=int, default=100,
        help="Candidates handed to one octoconda run (default: 100)",
    )
    parser.add_argument(
        "--tsv", action="store_true",
        help="Print 'stars<TAB>language<TAB>url' instead of bare URLs",
    )
    parser.add_argument(
        "--full-test", action="store_true",
        help="Only count repositories whose latest release builds with rattler-build "
             "and validates on every platform it has a recipe for (slow: downloads "
             "and builds every candidate)",
    )
    parser.add_argument(
        "--add", action="store_true",
        help="Append the repositories found to config.toml instead of just printing "
             "them; implies --full-test",
    )
    args = parser.parse_args()
    if args.add:
        args.full_test = True
    if args.count < 1:
        parser.error("--count must be at least 1")
    if args.batch_size < 1:
        parser.error("--batch-size must be at least 1")

    ensure_github_token()
    if not os.environ.get("GITHUB_TOKEN"):
        print("Error: a GitHub token is required for the GraphQL search API.", file=sys.stderr)
        sys.exit(1)
    known_slugs, known_names = load_known_repos(args.config)
    conda_channel = conda_channel_from_config(args.config)

    session = requests.Session()
    session.headers.update({
        "Authorization": f"bearer {os.environ['GITHUB_TOKEN']}",
        "User-Agent": "octoconda-top-repos",
    })

    print(
        f"Looking for the {args.count} most-starred repositories octoconda can handle"
        + (f" (languages: {', '.join(args.language)})" if args.language else "")
        + (f" ({args.query})" if args.query else "")
        + (" with a full build and package validation" if args.full_test else "")
        + " ...",
        file=sys.stderr,
    )
    try:
        found, skipped_known = find_top_repos(
            session, count=args.count, min_stars=args.min_stars, languages=args.language,
            extra_query=args.query, batch_size=args.batch_size, known_slugs=known_slugs,
            conda_channel=conda_channel, full_test=args.full_test,
        )
    except GitHubSearchError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

    if skipped_known:
        print(f"Skipped {skipped_known} repositories already in {args.config}.", file=sys.stderr)
    if len(found) < args.count:
        print(
            f"Warning: only {len(found)} repositories found before reaching "
            f"{args.min_stars} stars; lower --min-stars to scan further.",
            file=sys.stderr,
        )

    for node in found:
        url = f"https://github.com/{node['nameWithOwner']}"
        if args.tsv:
            language = (node.get("primaryLanguage") or {}).get("name") or "-"
            print(f"{node['stargazerCount']}\t{language}\t{url}")
        else:
            print(url)

    if args.add and found:
        add_repos_to_config(args.config, [n["nameWithOwner"] for n in found], known_names)


def _run_self_tests() -> None:
    import traceback

    failures = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("_test_") and callable(fn):
            try:
                fn()
                print(f"ok {name}")
            except Exception:  # noqa: BLE001 - report every failing test, whatever it raised
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
