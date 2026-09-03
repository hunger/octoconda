#!/usr/bin/env python3
"""Fetch all posts from a Mastodon account and extract the GitHub repos they link to.

Usage:
    pixi run mastodon-repos @orhun@fosstodon.org
    # -> prints one normalized https://github.com/owner/repo URL per line,
    #    suitable for feeding into `pixi run add-repo <url>`.

    pixi run mastodon-repos @orhun@fosstodon.org --add
    # -> additionally checks candidates with octoconda and writes the new
    #    repos into config.toml (reuses scripts/add_repo.py logic).

Reblogged statuses are scanned too, since boosts often carry the project link.
"""

import argparse
import os
import re
import sys
import time

import requests
from bs4 import BeautifulSoup

# add_repo.py lives next to this file; make it importable regardless of cwd.
_SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

from add_repo import (  # noqa: E402
    is_repo_url,
    normalize,
    repo_slug,
    load_known_repos,
    ensure_github_token,
    conda_channel_from_config,
    check_with_octoconda,
    add_repos_to_config,
)

# Bare github.com URLs that may appear in raw (non-href) text.
GITHUB_RAW_RE = re.compile(r"https?://github\.com/[A-Za-z0-9_./-]+")


def parse_handle(handle: str) -> tuple[str, str]:
    """Parse '@user@domain' / 'user@domain' into (qualified_acct, domain)."""
    h = handle.strip()
    if h.startswith("@"):
        h = h[1:]
    local, sep, domain = h.partition("@")
    if not sep or not local or not domain:
        raise ValueError(
            f"Invalid Mastodon handle '{handle}'. Expected '@user@domain' "
            f"(e.g. @orhun@fosstodon.org)."
        )
    return f"{local}@{domain}", domain


def make_session(token: str | None = None) -> requests.Session:
    s = requests.Session()
    s.headers["User-Agent"] = "octoconda-mastodon-repos/1.0"
    if token:
        s.headers["Authorization"] = f"Bearer {token}"
    return s


def get_with_retry(
    session: requests.Session, url: str, timeout: int = 30, retries: int = 5,
    params: dict | None = None,
) -> requests.Response:
    """GET with backoff on rate-limit (429) and transient 5xx responses."""
    resp = None
    for attempt in range(retries + 1):
        resp = session.get(url, timeout=timeout, params=params)
        if resp.status_code == 429:
            wait = int(resp.headers.get("Retry-After", "5") or 5)
            time.sleep(min(wait, 60) + 1)
            continue
        if 500 <= resp.status_code < 600 and attempt < retries:
            time.sleep(2 ** attempt)
            continue
        return resp
    return resp


def lookup_account(session: requests.Session, acct: str, domain: str) -> str:
    """Resolve a Mastodon account to its numeric id on the given instance."""
    # Pass acct via params so the '@' is percent-encoded correctly.
    r = get_with_retry(
        session, f"https://{domain}/api/v1/accounts/lookup", timeout=30,
        params={"acct": acct},
    )
    if r.status_code == 200:
        return r.json()["id"]
    # Fallback for older instances without /accounts/lookup.
    local = acct.split("@", 1)[0]
    r = get_with_retry(session, f"https://{domain}/api/v1/accounts/{local}", timeout=30)
    if r.status_code == 200:
        return r.json()["id"]
    raise RuntimeError(
        f"Could not find account '{acct}' on {domain} "
        f"(lookup -> HTTP {r.status_code}). Check the handle/domain."
    )


def parse_link_next(link_header: str | None) -> str | None:
    """Return the rel='next' URL from an HTTP Link header, or None."""
    if not link_header:
        return None
    for url, rel in re.findall(r'<([^>]+)>;\s*rel="(\w+)"', link_header):
        if rel == "next":
            return url
    return None


def iter_statuses(
    session: requests.Session, domain: str, account_id: str,
    limit: int = 40, max_posts: int | None = None,
):
    """Yield status dicts for every post of the account (newest first).

    Paginates using the HTTP Link header's rel='next' (Mastodon bakes in a
    pre-computed max_id). Falls back to manual max_id pagination if a full
    page arrives without a Link header. Stops at an empty/short final page.
    """
    base = f"https://{domain}/api/v1/accounts/{account_id}/statuses"
    url: str | None = f"{base}?limit={limit}"
    seen = 0
    while url is not None:
        resp = get_with_retry(session, url)
        if resp.status_code != 200:
            raise RuntimeError(
                f"Fetching statuses failed (HTTP {resp.status_code}): "
                f"{resp.text[:500]}"
            )
        page = resp.json()
        if not page:
            break
        for st in page:
            yield st
            seen += 1
            if max_posts is not None and seen >= max_posts:
                return

        next_url = parse_link_next(resp.headers.get("Link"))
        if next_url:
            url = next_url
        elif len(page) < limit:
            break  # short final page -> end of timeline
        else:
            # Full page but no Link header (non-standard server): paginate manually.
            last_id = page[-1]["id"]
            url = f"{base}?limit={limit}&max_id={last_id}"


def status_contents(status: dict):
    """Yield HTML content blobs for a status (own body + reblogged body)."""
    c = status.get("content")
    if c:
        yield c
    rb = status.get("reblog") or {}
    rc = rb.get("content")
    if rc:
        yield rc


def extract_github_urls(html: str) -> list[str]:
    """Return sorted, normalized GitHub repo URLs found in an HTML blob."""
    soup = BeautifulSoup(html, "html.parser")
    hrefs = {a["href"] for a in soup.find_all("a", href=True)}
    raw = set(GITHUB_RAW_RE.findall(html))
    return sorted({normalize(u) for u in (hrefs | raw) if is_repo_url(u)})


def collect_repos(
    handle: str, token: str | None = None, max_posts: int | None = None, limit: int = 40,
) -> list[str]:
    """Return an ordered, de-duplicated list of GitHub repo URLs across all posts."""
    acct, domain = parse_handle(handle)
    session = make_session(token)
    account_id = lookup_account(session, acct, domain)
    print(f"Account: {acct} (id={account_id}) on {domain}", file=sys.stderr)

    ordered: dict[str, str] = {}  # slug(lower) -> normalized url, first-seen order
    posts_scanned = 0
    for st in iter_statuses(session, domain, account_id, limit=limit, max_posts=max_posts):
        posts_scanned += 1
        if posts_scanned % 500 == 0:
            print(
                f"  ... {posts_scanned} posts scanned, "
                f"{len(ordered)} repo(s) so far", file=sys.stderr,
            )
        for html in status_contents(st):
            for url in extract_github_urls(html):
                slug = repo_slug(url).lower()
                if slug and slug not in ordered:
                    ordered[slug] = url

    print(
        f"Scanned {posts_scanned} post(s); found {len(ordered)} unique GitHub repo(s).",
        file=sys.stderr,
    )
    return list(ordered.values())


def main():
    parser = argparse.ArgumentParser(
        description="Fetch all posts from a Mastodon account and extract the "
                    "GitHub repository URLs they link to."
    )
    parser.add_argument("handle", help="Mastodon handle, e.g. @orhun@fosstodon.org")
    parser.add_argument(
        "--token", default=os.environ.get("MASTODON_TOKEN"),
        help="Optional Mastodon access token (env: MASTODON_TOKEN)",
    )
    parser.add_argument(
        "--max-posts", type=int, default=None,
        help="Stop after this many posts (default: fetch all)",
    )
    parser.add_argument(
        "--limit", type=int, default=40,
        help="Page size for the API (default: 40)",
    )
    parser.add_argument(
        "--add", action="store_true",
        help="Check candidates with octoconda and add new ones to config.toml "
             "(reuses scripts/add_repo.py logic) instead of just printing URLs.",
    )
    parser.add_argument(
        "-c", "--config", default="./config.toml",
        help="Path to config.toml for --add (default: ./config.toml)",
    )
    args = parser.parse_args()

    try:
        urls = collect_repos(
            args.handle, token=args.token, max_posts=args.max_posts, limit=args.limit,
        )
    except (ValueError, RuntimeError) as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

    if not urls:
        print("No GitHub repository URLs found.", file=sys.stderr)
        return

    if not args.add:
        for url in urls:
            print(url)
        return

    # --add: batch-check candidates with octoconda, then write new ones to config.
    known, known_names = load_known_repos(args.config)
    candidate_slugs: list[str] = []
    seen: set[str] = set()
    for u in urls:
        slug = repo_slug(u)
        key = slug.lower()
        if key not in seen and key not in known:
            seen.add(key)
            candidate_slugs.append(slug)

    ensure_github_token()
    conda_channel = conda_channel_from_config(args.config)
    print(
        f"Checking {len(candidate_slugs)} repo(s) by running octoconda...",
        file=sys.stderr,
    )
    passing, skip_reasons = check_with_octoconda(candidate_slugs, conda_channel)
    for slug in candidate_slugs:
        if slug not in passing:
            reason = skip_reasons.get(slug, "no recipe generated by octoconda")
            print(f"  skipped ({reason}): https://github.com/{slug}", file=sys.stderr)

    new_slugs = [s for s in candidate_slugs if s in passing]
    if not new_slugs:
        print("No new repos with packageable releases found.", file=sys.stderr)
        return
    add_repos_to_config(args.config, new_slugs, known_names)


if __name__ == "__main__":
    main()
