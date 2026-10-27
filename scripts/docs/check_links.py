#!/usr/bin/env python3
"""Checks every link in the built documentation website, without network access.

    python3 scripts/docs/check_links.py [book/book]

For each HTML page mdBook wrote, it checks:

- relative links and absolute links under the site's base path (/ticoraw/): the target
  file exists, and a #fragment names an element id on the target page;
- links into this repository on GitHub (https://github.com/lukaj99/ticoraw/blob|tree|edit/
  main/<path>): <path> exists in the working tree, and for a Markdown file a #fragment
  matches one of its headings (GitHub's heading ids);
- other external links are counted but not fetched.

print.html is skipped: it repeats every page with rewritten links, so it adds nothing but
duplicates. Exit status 1 lists every broken link; 0 means none.
"""

from __future__ import annotations

import posixpath
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

REPO_URL = "https://github.com/lukaj99/ticoraw/"
SITE_BASE = "/ticoraw/"
SKIP_PAGES = {"print.html"}


class Page(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.ids: set[str] = set()
        self.links: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        a = dict(attrs)
        for key in ("id", "name"):
            if a.get(key):
                self.ids.add(a[key])  # type: ignore[arg-type]
        for key in ("href", "src"):
            v = a.get(key)
            if v and not (tag == "link" and a.get("rel") in ("preconnect", "dns-prefetch")):
                self.links.append(v)


def github_slug(heading: str) -> str:
    s = re.sub(r"`|\*\*|\*|\\", "", heading.strip().lower())
    s = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", s)
    s = re.sub(r"[^\w\- ]", "", s)
    return s.replace(" ", "-")


def markdown_ids(path: Path) -> set[str]:
    ids: set[str] = set()
    counts: dict[str, int] = {}
    fence = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if re.match(r"^ {0,3}(```|~~~)", line):
            fence = not fence
            continue
        m = None if fence else re.match(r"^#{1,6}\s+(.*?)\s*#*\s*$", line)
        if m:
            slug = github_slug(m.group(1))
            n = counts.get(slug, 0)
            counts[slug] = n + 1
            ids.add(slug if n == 0 else f"{slug}-{n}")
    return ids


def main() -> int:
    repo = Path(__file__).resolve().parents[2]
    out = Path(sys.argv[1] if len(sys.argv) > 1 else repo / "book" / "book").resolve()
    if not (out / "index.html").is_file():
        print(f"check_links: no book at {out}; run `mdbook build book` first", file=sys.stderr)
        return 2

    pages: dict[str, Page] = {}
    for f in sorted(out.rglob("*.html")):
        p = Page()
        p.feed(f.read_text(encoding="utf-8"))
        pages[f.relative_to(out).as_posix()] = p

    md_ids: dict[str, set[str]] = {}
    broken: list[str] = []
    internal = repo_links = external = 0

    for name, page in pages.items():
        if name in SKIP_PAGES or name.startswith("convert/"):
            continue
        for link in page.links:
            u = urlsplit(link)
            if u.scheme in ("mailto", "data", "javascript"):
                continue
            if u.scheme in ("http", "https"):
                if link.startswith(REPO_URL):
                    repo_links += 1
                    parts = u.path.split("/")
                    # /lukaj99/ticoraw/<kind>/main/<path...>
                    if len(parts) >= 5 and parts[3] in ("blob", "tree", "edit") and parts[4] == "main":
                        rel = posixpath.normpath(unquote("/".join(parts[5:])))
                        target = repo / rel
                        if rel.startswith("..") or not target.exists():
                            broken.append(f"{name}: {link} (no {rel} in the repository)")
                        elif u.fragment and target.suffix.lower() == ".md":
                            ids = md_ids.setdefault(rel, markdown_ids(target))
                            if u.fragment not in ids:
                                broken.append(f"{name}: {link} (no heading #{u.fragment} in {rel})")
                else:
                    external += 1
                continue
            if u.scheme or u.netloc:
                external += 1
                continue
            internal += 1
            if u.path.startswith("/"):
                if not u.path.startswith(SITE_BASE):
                    broken.append(f"{name}: {link} (outside {SITE_BASE})")
                    continue
                target = u.path[len(SITE_BASE):]
            elif u.path:
                target = posixpath.normpath(posixpath.join(posixpath.dirname(name), unquote(u.path)))
            else:
                target = name
            if target.startswith(".."):
                broken.append(f"{name}: {link} (leaves the site)")
                continue
            if target.endswith("/") or target in ("", "."):
                target = posixpath.join(target, "index.html")
            path = out / target
            if path.is_dir():
                target = posixpath.join(target, "index.html")
                path = out / target
            if not path.is_file():
                broken.append(f"{name}: {link} (no file {target})")
                continue
            if u.fragment and target.endswith(".html"):
                ids = pages[target].ids if target in pages else set()
                if unquote(u.fragment) not in ids:
                    broken.append(f"{name}: {link} (no #{u.fragment} on {target})")

    for b in broken:
        print(f"broken: {b}")
    print(
        f"check_links: {len(pages)} pages, {internal} internal and {repo_links} repository "
        f"links checked, {external} other external links not fetched, {len(broken)} broken"
    )
    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main())
