#!/usr/bin/env python3
"""mdBook preprocessor: include repository files and keep their links working.

The book includes files that also live elsewhere in the repository (docs/FORMAT.md,
CONTRIBUTING.md, crate READMEs, ...), so that each has one source of truth. mdBook's own
`{{#include}}` pastes the text unchanged, which breaks every relative link in it: a link
written relative to `docs/` no longer resolves from `book/src/`. This preprocessor runs
before mdBook's `links` preprocessor and expands the same directive itself:

    {{#include ../../docs/FORMAT.md}}        the whole file
    {{#include ../../README.md:12:40}}       lines 12 to 40 (1-based, inclusive)
    {{#include ../../README.md:12:}}         line 12 to the end ( ::40 and :12 also work)

For Markdown files, every relative link is resolved against the included file's own
directory, then rewritten:

- to the book page that renders that file, when there is one (see PAGES below), keeping
  any #fragment;
- otherwise to the file or directory on GitHub (blob/ or tree/ on main).

Other files (source code in a fenced block, for example) are inserted verbatim. Includes
with a named anchor (`file.rs:anchor`) are left for mdBook's `links` preprocessor.

A chapter whose whole text is one include of a Markdown file gets that file as its
source path, so the page's "edit" button opens the real file on GitHub.

Configured in book/book.toml as:

    [preprocessor.repo-include]
    command = "python3 ../scripts/docs/repo_include.py"
    before = ["links"]

Only the Python standard library is used.
"""

from __future__ import annotations

import json
import posixpath
import re
import sys
from pathlib import Path

REPO_URL = "https://github.com/lukaj99/ticoraw"
BRANCH = "main"

# Repository files that the book renders, mapped to their page (relative to book/src).
PAGES = {
    "docs/FORMAT.md": "format.md",
    "docs/compat.md": "compat.md",
    "docs/SAMPLES.md": "contributing/samples.md",
    "conformance/README.md": "conformance.md",
    "CONTRIBUTING.md": "contributing/index.md",
    "CHANGELOG.md": "changelog.md",
    "SUPPORT.md": "help.md",
}

INCLUDE = re.compile(r"\{\{#include\s+([^}\s]+)\s*\}\}")
# Inline links and images: ](target) or ](target "title"). Targets with spaces must use <>.
INLINE = re.compile(r"(\]\()(<[^>]*>|[^)\s]+)((?:\s+\"[^\"]*\")?\))")
# Reference definitions: [label]: target
REFDEF = re.compile(r"^( {0,3}\[[^\]]+\]:\s*)(\S+)(.*)$")
# HTML attributes in raw HTML blocks.
HTMLATTR = re.compile(r"((?:href|src)=\")([^\"]+)(\")")
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
RANGE = re.compile(r"^(.*?):(\d*)(?::(\d*))?$")


class IncludeError(Exception):
    pass


def split_spec(spec: str) -> tuple[str, int | None, int | None]:
    """Splits `path[:first[:last]]` into the path and a 1-based, inclusive line range."""
    m = RANGE.match(spec)
    if not m or not m.group(1) or (m.group(2) == "" and m.group(3) is None):
        return spec, None, None
    path, a, b = m.group(1), m.group(2), m.group(3)
    if b is None:
        # file:N is line N only.
        return path, int(a), int(a)
    return path, int(a) if a else None, int(b) if b else None


def is_anchor_include(spec: str) -> bool:
    # file.rs:name (a named anchor), which mdBook handles itself.
    _, sep, tail = spec.partition(":")
    return bool(sep) and not re.fullmatch(r"\d*(?::\d*)?", tail)


def rewrite_target(target: str, src_dir: str, chapter_dir: str, root: Path) -> str:
    """Rewrites one link target written relative to repo directory `src_dir`."""
    bracketed = target.startswith("<") and target.endswith(">")
    raw = target[1:-1] if bracketed else target
    if not raw or raw.startswith(("#", "/")) or SCHEME.match(raw):
        return target
    path, hash_, frag = raw.partition("#")
    repo_path = posixpath.normpath(posixpath.join(src_dir, path)) if path else src_dir
    if repo_path.startswith(".."):
        raise IncludeError(f"link {raw!r} leaves the repository")
    if repo_path in PAGES:
        page = PAGES[repo_path]
        new = posixpath.relpath(page, chapter_dir or ".")
    else:
        kind = "tree" if (root / repo_path).is_dir() else "blob"
        new = f"{REPO_URL}/{kind}/{BRANCH}/{repo_path}"
    if hash_:
        new += "#" + frag
    return f"<{new}>" if bracketed else new


def rewrite_links(text: str, src_dir: str, chapter_dir: str, root: Path) -> str:
    out = []
    fence = None
    for line in text.split("\n"):
        m = FENCE.match(line)
        if fence:
            if m and m.group(1)[0] == fence[0] and len(m.group(1)) >= len(fence):
                fence = None
            out.append(line)
            continue
        if m:
            fence = m.group(1)
            out.append(line)
            continue

        def fix(mm: re.Match[str]) -> str:
            return mm.group(1) + rewrite_target(mm.group(2), src_dir, chapter_dir, root) + mm.group(3)

        r = REFDEF.match(line)
        if r:
            line = fix(r)
        else:
            line = INLINE.sub(fix, line)
        line = HTMLATTR.sub(fix, line)
        out.append(line)
    return "\n".join(out)


def expand(content: str, chapter_src: str, src_root: Path, repo_root: Path) -> tuple[str, str | None]:
    """Expands includes in one chapter. Returns (content, new source path or None)."""
    chapter_dir = posixpath.dirname(chapter_src)
    only = None
    stripped = content.strip()
    whole = INCLUDE.fullmatch(stripped)

    def repl(m: re.Match[str]) -> str:
        nonlocal only
        spec = m.group(1)
        if is_anchor_include(spec):
            return m.group(0)
        path, first, last = split_spec(spec)
        file = (src_root / chapter_dir / path).resolve()
        try:
            rel = file.relative_to(repo_root).as_posix()
        except ValueError as e:
            raise IncludeError(f"{chapter_src}: {spec} is outside the repository") from e
        if not file.is_file():
            raise IncludeError(f"{chapter_src}: {spec}: no such file")
        text = file.read_text(encoding="utf-8")
        if first is not None or last is not None:
            lines = text.split("\n")
            lo = (first or 1) - 1
            hi = last if last is not None else len(lines)
            text = "\n".join(lines[lo:hi])
        if file.suffix.lower() == ".md":
            text = rewrite_links(text, posixpath.dirname(rel), chapter_dir, repo_root)
            if whole and first is None and last is None:
                only = posixpath.relpath(file.as_posix(), (src_root).as_posix())
        return text.rstrip("\n")

    return INCLUDE.sub(repl, content), only


def walk(items: list, fn) -> None:
    for item in items:
        if isinstance(item, dict) and "Chapter" in item:
            ch = item["Chapter"]
            fn(ch)
            walk(ch.get("sub_items", []), fn)


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == "supports":
        return 0
    context, book = json.load(sys.stdin)
    book_root = Path(context["root"]).resolve()
    src_root = (book_root / context["config"]["book"].get("src", "src")).resolve()
    repo_root = book_root.parent

    errors = []

    def fn(ch: dict) -> None:
        src = ch.get("source_path") or ch.get("path")
        if not src or not ch.get("content"):
            return
        try:
            ch["content"], only = expand(ch["content"], src, src_root, repo_root)
        except IncludeError as e:
            errors.append(str(e))
            return
        if only:
            ch["source_path"] = only

    walk(book.get("items", book.get("sections", [])), fn)
    if errors:
        for e in errors:
            print(f"repo-include: {e}", file=sys.stderr)
        return 1
    json.dump(book, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
