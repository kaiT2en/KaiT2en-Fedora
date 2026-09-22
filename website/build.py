#!/usr/bin/env python3
"""Build the kait2en.org site from Markdown into an output directory.

Everything the site is made of lives next to this file:
  site.yml     site configuration, section order and sidebar grouping
  docs/        documentation, rendered into one long documentation.html
  blog/        one Markdown file per post, YAML front matter on top
  pages/       prose blocks embedded into the landing page
  templates/   Jinja templates
  static/      stylesheet, script and images, copied verbatim

Images are referenced relative to the output page: `img/x.png` from a page in
the output root, `../img/x.png` from a blog post. The link check catches the
mistake either way.

Usage: python3 website/build.py [--output website/build] [--serve [PORT]]
"""

from __future__ import annotations

import argparse
import datetime
import email.utils
import re
import shutil
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any
from xml.sax.saxutils import escape as xml_escape

import markdown
import yaml
from jinja2 import Environment, FileSystemLoader, StrictUndefined
from markdown.extensions.toc import slugify
from markupsafe import Markup

WEBSITE = Path(__file__).resolve().parent
ROOT = WEBSITE.parent
CONFIG_FILE = WEBSITE / "site.yml"
DOCS_DIR = WEBSITE / "docs"
BLOG_DIR = WEBSITE / "blog"
DEFAULT_OUTPUT = WEBSITE / "build"

# The feature board renderer lives with the other CI scripts.
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

MD_EXTENSIONS = [
    "extra",
    "admonition",
    "sane_lists",
    "codehilite",
    "toc",
]
MD_EXTENSION_CONFIGS = {
    "codehilite": {"guess_lang": False, "css_class": "highlight"},
    "toc": {"permalink": "#", "permalink_class": "headerlink", "permalink_title": ""},
}

FRONT_MATTER_RE = re.compile(r"\A---\r?\n(.*?)\r?\n---\r?\n?", re.DOTALL)
H1_RE = re.compile(r"^#\s+(?P<title>.+?)\s*$", re.MULTILINE)
# An H2 that starts a line, i.e. not a `##` inside a fenced code block.
H2_SPLIT_RE = re.compile(r"^##\s+(?P<title>.+?)\s*$", re.MULTILINE)
FENCE_RE = re.compile(r"^(?P<fence>```+|~~~+)", re.MULTILINE)


class BuildError(Exception):
    pass


@dataclass
class Section:
    """One `<section id>` of the documentation page."""

    id: str
    title: str
    html: Markup


@dataclass
class DocGroup:
    title: str
    sections: list[Section] = field(default_factory=list)


@dataclass
class Post:
    slug: str
    title: str
    date: datetime.date
    author: str
    summary: str
    tags: list[str]
    html: Markup
    url: str


def make_id(title: str, seen: set[str]) -> str:
    base = slugify(title, "-") or "section"
    candidate, counter = base, 2
    while candidate in seen:
        candidate, counter = f"{base}-{counter}", counter + 1
    seen.add(candidate)
    return candidate


def render_markdown(text: str, seen: set[str] | None = None) -> Markup:
    """Render Markdown. `seen` keeps heading ids unique across a whole page.

    Every section of a page is a separate Markdown run, so the toc extension
    cannot deduplicate across them on its own.
    """
    configs = dict(MD_EXTENSION_CONFIGS)
    if seen is not None:
        configs["toc"] = dict(configs["toc"], slugify=lambda text, sep: make_id(text, seen))
    md = markdown.Markdown(extensions=MD_EXTENSIONS, extension_configs=configs)
    return Markup(md.convert(text))


def read_config() -> dict[str, Any]:
    try:
        data = yaml.safe_load(CONFIG_FILE.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise BuildError(f"{CONFIG_FILE} not found") from None
    except yaml.YAMLError as exc:
        raise BuildError(f"{CONFIG_FILE}: invalid YAML: {exc}") from None
    if not isinstance(data, dict):
        raise BuildError(f"{CONFIG_FILE}: expected a mapping at the top level")
    for key in ("site", "links", "nav", "hero", "index", "documentation", "blog"):
        if key not in data:
            raise BuildError(f"{CONFIG_FILE}: missing required key '{key}'")
    return data


def split_front_matter(path: Path) -> tuple[dict[str, Any], str]:
    text = path.read_text(encoding="utf-8")
    match = FRONT_MATTER_RE.match(text)
    if not match:
        return {}, text
    try:
        meta = yaml.safe_load(match.group(1)) or {}
    except yaml.YAMLError as exc:
        raise BuildError(f"{path}: invalid front matter: {exc}") from None
    if not isinstance(meta, dict):
        raise BuildError(f"{path}: front matter must be a mapping")
    return meta, text[match.end():]


def code_fence_spans(text: str) -> list[tuple[int, int]]:
    """Character ranges covered by fenced code blocks."""
    spans: list[tuple[int, int]] = []
    open_at: int | None = None
    marker = ""
    for match in FENCE_RE.finditer(text):
        fence = match.group("fence")
        if open_at is None:
            open_at, marker = match.start(), fence[0] * 3
        elif fence.startswith(marker):
            spans.append((open_at, match.end()))
            open_at = None
    if open_at is not None:
        spans.append((open_at, len(text)))
    return spans


def strip_h1(text: str, path: Path) -> tuple[str, str]:
    """Return the document title and the body without its H1 line."""
    match = H1_RE.search(text)
    if not match:
        raise BuildError(f"{path}: no level-1 heading found")
    body = text[: match.start()] + text[match.end():]
    return match.group("title"), body.lstrip("\n")


def demote_headings(text: str) -> str:
    """Shift H2..H5 down one level; sections already carry an H2 heading."""
    spans = code_fence_spans(text)

    def in_code(pos: int) -> bool:
        return any(start <= pos < end for start, end in spans)

    out, last = [], 0
    for match in re.finditer(r"^(#{2,5})(\s+)", text, re.MULTILINE):
        if in_code(match.start()):
            continue
        out.append(text[last: match.start()])
        out.append("#" + match.group(1) + match.group(2))
        last = match.end()
    out.append(text[last:])
    return "".join(out)


def split_on_h2(text: str, path: Path) -> list[tuple[str | None, str]]:
    """Split a document into (h2 title, body) chunks, keeping the intro first."""
    spans = code_fence_spans(text)
    cuts = [
        m for m in H2_SPLIT_RE.finditer(text)
        if not any(start <= m.start() < end for start, end in spans)
    ]
    if not cuts:
        return [(None, text)]

    chunks: list[tuple[str | None, str]] = []
    intro = text[: cuts[0].start()].strip()
    if intro:
        chunks.append((None, intro))
    for index, cut in enumerate(cuts):
        end = cuts[index + 1].start() if index + 1 < len(cuts) else len(text)
        chunks.append((cut.group("title"), text[cut.end(): end].strip()))
    if not chunks:
        raise BuildError(f"{path}: no content found")
    return chunks


def rewrite_doc_links(html: str, anchors: dict[str, str], source: Path) -> Markup:
    """Point links between documentation files at anchors on the single page."""
    def replace(match: re.Match[str]) -> str:
        target = match.group("href")
        path, _, fragment = target.partition("#")
        if not path or path.startswith(("http://", "https://", "mailto:", "#", "/")):
            return match.group(0)
        resolved = (source.parent / path).resolve().relative_to(DOCS_DIR).as_posix()
        if resolved not in anchors:
            raise BuildError(
                f"{source.relative_to(ROOT)}: link '{target}' points at "
                f"docs/{resolved}, which is not part of the documentation page"
            )
        anchor = f"#{fragment}" if fragment else f"#{anchors[resolved]}"
        return f'href="{anchor}"'

    return Markup(re.sub(r'href="(?P<href>[^"]+)"', replace, html))


def wrap_code_blocks(html: str) -> Markup:
    """Wrap code blocks so the template can hang a copy button on them."""
    button = (
        '<button class="copy-btn" type="button" onclick="copyBlock(this)">'
        '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"'
        ' width="14" height="14"><rect x="9" y="9" width="13" height="13" rx="2"'
        ' ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>'
        "</svg>Copy</button>"
    )
    return Markup(re.sub(
        r'(<div class="highlight">.*?</div>)',
        lambda m: f'<div class="code-wrap">{m.group(1)}{button}</div>',
        html,
        flags=re.DOTALL,
    ))


def build_documentation(config: dict[str, Any]) -> tuple[list[DocGroup], dict[str, str]]:
    """Render docs/ into sections, and map each source file to its anchor."""
    groups: list[DocGroup] = []
    anchors: dict[str, str] = {}
    seen: set[str] = set()

    # Two passes: anchors have to exist before links between files are rewritten.
    staged: list[tuple[DocGroup, Path, list[tuple[str, str, str]]]] = []
    for raw_group in config["documentation"]["groups"]:
        group = DocGroup(title=raw_group["title"])
        groups.append(group)
        for entry in raw_group["pages"]:
            source = DOCS_DIR / entry["file"]
            if not source.is_file():
                raise BuildError(f"site.yml: docs/{entry['file']} does not exist")
            meta, text = split_front_matter(source)
            title, body = strip_h1(text, source)
            title = entry.get("title") or meta.get("title") or title

            pieces: list[tuple[str, str, str]] = []
            if entry.get("split") == "h2":
                for chunk_title, chunk in split_on_h2(body, source):
                    name = title if chunk_title is None else chunk_title
                    pieces.append((make_id(name, seen), name, chunk))
            else:
                pieces.append((make_id(title, seen), title, body))
            staged.append((group, source, pieces))
            anchors[entry["file"]] = pieces[0][0]

    for group, source, pieces in staged:
        for section_id, title, body in pieces:
            html = render_markdown(demote_headings(body), seen)
            html = rewrite_doc_links(html, anchors, source)
            group.sections.append(
                Section(id=section_id, title=title, html=wrap_code_blocks(html))
            )
    return groups, anchors


def load_posts(config: dict[str, Any]) -> list[Post]:
    if not BLOG_DIR.is_dir():
        return []
    posts: list[Post] = []
    for source in sorted(BLOG_DIR.glob("*.md")):
        meta, text = split_front_matter(source)
        if not meta:
            raise BuildError(f"{source}: blog posts need YAML front matter")
        for field_name in ("title", "date"):
            if field_name not in meta:
                raise BuildError(f"{source}: front matter is missing '{field_name}'")
        date = meta["date"]
        if isinstance(date, datetime.datetime):
            date = date.date()
        if not isinstance(date, datetime.date):
            raise BuildError(f"{source}: 'date' must be a YYYY-MM-DD date")
        if meta.get("draft"):
            continue
        slug = meta.get("slug") or re.sub(r"^\d{4}-\d{2}-\d{2}-", "", source.stem)
        html = wrap_code_blocks(render_markdown(text, set()))
        posts.append(
            Post(
                slug=slug,
                title=str(meta["title"]),
                date=date,
                author=str(meta.get("author", config["site"]["name"])),
                summary=" ".join(str(meta.get("summary", "")).split()),
                tags=[str(tag) for tag in meta.get("tags", [])],
                html=html,
                url=f"blog/{slug}.html",
            )
        )
    posts.sort(key=lambda post: (post.date, post.title), reverse=True)
    slugs = [post.slug for post in posts]
    duplicate = next((s for s in slugs if slugs.count(s) > 1), None)
    if duplicate:
        raise BuildError(f"blog/: duplicate post slug '{duplicate}'")
    return posts


def build_index_sections(config: dict[str, Any], anchors: dict[str, str]) -> list[dict]:
    from render_html import render as render_board
    from schema import load_items

    sections = []
    seen = {raw["id"] for raw in config["index"]["sections"]}
    for raw in config["index"]["sections"]:
        section = dict(raw)
        kind = section.get("type", "markdown")
        if kind == "markdown":
            source = WEBSITE / section["file"]
            if not source.is_file():
                raise BuildError(f"site.yml: {section['file']} does not exist")
            _, text = split_front_matter(source)
            section["html"] = render_markdown(text, seen)
        elif kind == "feature-board":
            def resolve(link: str) -> str:
                if link not in anchors:
                    raise BuildError(
                        f"website/data/features.yml: link 'docs/{link}' is not part of "
                        "the documentation page listed in site.yml"
                    )
                return f"documentation.html#{anchors[link]}"

            section["html"] = Markup(render_board(load_items(), resolve=resolve))
        elif kind not in ("install", "community", "donations"):
            raise BuildError(f"site.yml: unknown index section type '{kind}'")
        sections.append(section)
    return sections


def group_sidebar(items: list[dict], key: str = "group") -> list[dict]:
    """Turn a flat section list into the sidebar's grouped shape."""
    groups: list[dict] = []
    for item in items:
        if item.get("sidebar") is False:
            continue
        title = item.get(key) or ""
        if not groups or groups[-1]["title"] != title:
            groups.append({"title": title, "entries": []})
        groups[-1]["entries"].append({"id": item["id"], "title": item["title"]})
    return groups


def rfc822(date: datetime.date) -> str:
    stamp = datetime.datetime.combine(date, datetime.time(12, 0), datetime.timezone.utc)
    return email.utils.format_datetime(stamp)


def write_feed(config: dict[str, Any], posts: list[Post], out: Path) -> None:
    base = config["site"]["url"].rstrip("/")
    items = []
    for post in posts[:20]:
        items.append(
            "<item>"
            f"<title>{xml_escape(post.title)}</title>"
            f"<link>{base}/{post.url}</link>"
            f"<guid isPermaLink='true'>{base}/{post.url}</guid>"
            f"<pubDate>{rfc822(post.date)}</pubDate>"
            f"<description>{xml_escape(post.summary or post.title)}</description>"
            "</item>"
        )
    feed = (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<rss version="2.0"><channel>'
        f"<title>{xml_escape(config['site']['name'])} - {xml_escape(config['blog']['title'])}</title>"
        f"<link>{base}/blog.html</link>"
        f"<description>{xml_escape(config['blog']['description'])}</description>"
        f"<language>{config['site']['language']}</language>"
        + "".join(items)
        + "</channel></rss>\n"
    )
    (out / "feed.xml").write_text(feed, encoding="utf-8")


def write_sitemap(
    config: dict[str, Any], posts: list[Post], out: Path, noindex: bool
) -> None:
    base = config["site"]["url"].rstrip("/")
    if noindex:
        (out / "robots.txt").write_text("User-agent: *\nDisallow: /\n", encoding="utf-8")
        return

    # No lastmod on the static pages: a build-date stamp would claim every page
    # changed on every deploy. Posts carry a real date, so they get one.
    urls = [f"<url><loc>{base}/</loc></url>"]
    urls += [f"<url><loc>{base}/{page}</loc></url>"
             for page in ("documentation.html", "blog.html")]
    urls += [f"<url><loc>{base}/{post.url}</loc>"
             f"<lastmod>{post.date.isoformat()}</lastmod></url>" for post in posts]

    (out / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
        + "".join(urls)
        + "</urlset>\n",
        encoding="utf-8",
    )
    (out / "robots.txt").write_text(
        f"User-agent: *\nAllow: /\nSitemap: {base}/sitemap.xml\n", encoding="utf-8"
    )


def write_redirects(config: dict[str, Any], out: Path) -> None:
    for source, target in (config.get("redirects") or {}).items():
        page = out / source.strip("/") / "index.html"
        page.parent.mkdir(parents=True, exist_ok=True)
        depth = len(source.strip("/").split("/"))
        href = "../" * depth + target
        page.write_text(
            "<!doctype html>\n"
            f'<html lang="en"><head><meta charset="utf-8">'
            f'<meta http-equiv="refresh" content="0; url={href}">'
            f'<link rel="canonical" href="{href}">'
            "<title>Moved</title></head>"
            f'<body>This page moved to <a href="{href}">{target}</a>.</body></html>\n',
            encoding="utf-8",
        )


LINK_RE = re.compile(r'(?:href|src)="([^"]+)"')
ID_RE = re.compile(r'\sid="([^"]+)"')


def check_links(out: Path) -> None:
    """Fail the build on a dead internal link, the way --strict used to."""
    pages = sorted(out.rglob("*.html"))
    ids: dict[Path, set[str]] = {}
    problems: list[str] = []

    for page in pages:
        found = ID_RE.findall(page.read_text(encoding="utf-8"))
        ids[page] = set(found)
        for duplicate in sorted({i for i in found if found.count(i) > 1}):
            problems.append(f"{page.relative_to(out)}: id '{duplicate}' is not unique")

    for page in pages:
        for target in LINK_RE.findall(page.read_text(encoding="utf-8")):
            if target.startswith(("http://", "https://", "mailto:", "bitcoin:", "data:", "//")):
                continue
            path, _, fragment = target.partition("#")
            where = page.relative_to(out)
            if not path:
                if fragment and fragment not in ids[page]:
                    problems.append(f"{where}: no element with id '{fragment}'")
                continue
            resolved = (page.parent / path).resolve()
            if resolved.is_dir():
                resolved = resolved / "index.html"
            if not resolved.exists():
                problems.append(f"{where}: '{target}' does not exist")
            elif fragment and resolved.suffix == ".html":
                if fragment not in ids.get(resolved, set()):
                    problems.append(
                        f"{where}: '{target}' has no element with id '{fragment}'"
                    )

    if problems:
        raise BuildError(
            f"{len(problems)} broken link(s):\n  " + "\n  ".join(problems)
        )


def copy_static(out: Path) -> None:
    shutil.copytree(WEBSITE / "static", out, dirs_exist_ok=True)


def build(output: Path, site_url: str | None = None, noindex: bool = False) -> None:
    config = read_config()
    if site_url:
        config["site"]["url"] = site_url.rstrip("/")
    env = Environment(
        loader=FileSystemLoader(WEBSITE / "templates"),
        undefined=StrictUndefined,
        autoescape=True,
        trim_blocks=True,
        lstrip_blocks=True,
    )
    env.filters["long_date"] = lambda d: d.strftime("%d %b %Y")
    env.filters["iso_date"] = lambda d: d.isoformat()

    doc_groups, anchors = build_documentation(config)
    index_sections = build_index_sections(config, anchors)
    posts = load_posts(config)

    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    (output / "blog").mkdir()

    shared = {
        "site": config["site"],
        "links": config["links"],
        "nav": config["nav"],
        "posts": posts,
        "noindex": noindex,
    }

    base_url = config["site"]["url"].rstrip("/")

    def write(name: str, template: str, **context: Any) -> None:
        depth = name.count("/")
        # The homepage is canonically the bare root, which is what the sitemap
        # lists and what inbound links point at.
        canonical = f"{base_url}/{'' if name == 'index.html' else name}"
        page = env.get_template(template).render(
            base="../" * depth, page_url=name, canonical=canonical, **shared, **context
        )
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(page, encoding="utf-8")

    write(
        "index.html",
        "index.html",
        hero=config["hero"],
        sections=index_sections,
        sidebar_title=config["index"]["sidebar_title"],
        sidebar=group_sidebar(index_sections),
    )
    write(
        "documentation.html",
        "documentation.html",
        doc=config["documentation"],
        groups=doc_groups,
    )
    write("blog.html", "blog_index.html", blog=config["blog"])
    for index, post in enumerate(posts):
        write(
            post.url,
            "blog_post.html",
            blog=config["blog"],
            post=post,
            newer=posts[index - 1] if index else None,
            older=posts[index + 1] if index + 1 < len(posts) else None,
        )
    write("404.html", "404.html")

    write_feed(config, posts, output)
    write_sitemap(config, posts, output, noindex)
    write_redirects(config, output)
    copy_static(output)
    (output / ".nojekyll").write_text("", encoding="utf-8")
    check_links(output)

    print(
        f"built {output.relative_to(ROOT) if output.is_relative_to(ROOT) else output}: "
        f"{sum(len(g.sections) for g in doc_groups)} doc sections, "
        f"{len(posts)} blog posts, {len(index_sections)} index sections"
    )


def serve(output: Path, port: int) -> None:
    import functools
    import http.server

    handler = functools.partial(http.server.SimpleHTTPRequestHandler,
                                directory=str(output))
    with http.server.ThreadingHTTPServer(("127.0.0.1", port), handler) as httpd:
        print(f"serving {output} on http://127.0.0.1:{port}/ (Ctrl-C to stop)")
        httpd.serve_forever()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default=DEFAULT_OUTPUT, type=Path,
                        help="output directory (default: website/build)")
    parser.add_argument("--site-url",
                        help="override site.url, for a preview deployment")
    parser.add_argument("--noindex", action="store_true",
                        help="ask search engines to skip the pages")
    parser.add_argument("--serve", nargs="?", const=8000, type=int, metavar="PORT",
                        help="serve the result after building")
    args = parser.parse_args()

    output = args.output if args.output.is_absolute() else Path.cwd() / args.output
    try:
        build(output, args.site_url, args.noindex)
    except BuildError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.serve:
        serve(output, args.serve)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
