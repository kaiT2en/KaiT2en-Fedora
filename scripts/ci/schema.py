# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
"""Load and validate website/data/features.yml."""

from __future__ import annotations

import datetime
import re
from pathlib import Path

import yaml

# What a user gets. Sort order of the filter chips and of the table.
STATES = ("in-progress", "planned", "available")

STATE_LABELS = {
    "in-progress": "In progress",
    "planned": "Planned",
    "available": "Available",
}

# What happens to it upstream.
UPSTREAM = (
    "downstream",
    "preparing",
    "submitted",
    "merged",
    "superseded",
    "revoked",
    "rejected",
    "stale",
)

UPSTREAM_LABELS = {
    "downstream": "Downstream only",
    "preparing": "Preparing",
    "submitted": "Submitted",
    "merged": "Merged",
    "superseded": "Superseded",
    "revoked": "Withdrawn",
    "rejected": "Rejected",
    "stale": "Stale",
}

# Something was submitted, so it needs a link and an author, and it is what the
# Discord channel announces.
SUBMITTED_UPSTREAM = ("submitted", "merged", "superseded", "revoked", "rejected", "stale")

REQUIRED_FIELDS = ("id", "title", "state", "upstream", "updated")
OPTIONAL_FIELDS = ("authors", "link", "help", "notes", "project", "subsystem", "version")

ID_RE = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")

ROOT = Path(__file__).resolve().parents[2]
DATA_FILE = ROOT / "website" / "data" / "features.yml"
DOCS_DIR = ROOT / "website" / "docs"


class UpstreamDataError(Exception):
    pass


def _fail(where: str, message: str) -> None:
    raise UpstreamDataError(f"{DATA_FILE.name}: {where}: {message}")


def _validate_item(item: object, index: int, seen_ids: set[str]) -> None:
    where = f"items[{index}]"
    if not isinstance(item, dict):
        _fail(where, "must be a mapping")

    item_id = item.get("id")
    if isinstance(item_id, str) and item_id:
        where = f"items[{index}] (id: {item_id})"

    for field in REQUIRED_FIELDS:
        if field not in item:
            _fail(where, f"missing required field '{field}'")

    unknown = set(item) - set(REQUIRED_FIELDS) - set(OPTIONAL_FIELDS)
    if unknown:
        _fail(where, f"unknown field(s): {', '.join(sorted(unknown))}")

    if not isinstance(item_id, str) or not ID_RE.match(item_id):
        _fail(where, "'id' must be a lowercase slug matching [a-z0-9]+(-[a-z0-9]+)*")
    if item_id in seen_ids:
        _fail(where, f"duplicate id '{item_id}' — ids must be unique and stable")
    seen_ids.add(item_id)

    if item["state"] not in STATES:
        _fail(where, f"'state' must be one of: {', '.join(STATES)}")
    if item["upstream"] not in UPSTREAM:
        _fail(where, f"'upstream' must be one of: {', '.join(UPSTREAM)}")

    submitted = item["upstream"] in SUBMITTED_UPSTREAM

    for field in ("title", "project", "subsystem", "version", "notes", "help"):
        if field in item:
            value = item[field]
            if not isinstance(value, str) or not value.strip():
                _fail(where, f"'{field}' must be a non-empty string when present")

    if "authors" in item:
        authors = item["authors"]
        if not isinstance(authors, list) or not authors:
            _fail(where, "'authors' must be a non-empty list")
        for author in authors:
            if not isinstance(author, str) or not author.strip():
                _fail(where, "each author must be a non-empty string")
            elif author.startswith("@"):
                _fail(where, f"author '{author}' must not include a leading '@'")
    elif submitted:
        _fail(where, f"'authors' is required when upstream is '{item['upstream']}'")

    if "link" in item:
        link = item["link"]
        if not isinstance(link, str) or not link.strip():
            _fail(where, "'link' must be a non-empty string")
        if link.startswith("https://"):
            pass
        elif submitted:
            _fail(where, f"'link' must be the upstream URL when upstream is '{item['upstream']}'")
        elif not link.endswith(".md"):
            _fail(where, "'link' must be an https:// URL or a .md page below website/docs/")
        # The site build catches this too, but a data error should name the data.
        elif not (DOCS_DIR / link).is_file():
            _fail(where, f"'link' points at website/docs/{link}, which does not exist")
    elif submitted:
        _fail(where, f"'link' is required when upstream is '{item['upstream']}'")

    # Unquoted YYYY-MM-DD parses as datetime.date, quoted as str; accept both.
    updated = item["updated"]
    if isinstance(updated, datetime.date):
        item["updated"] = updated.isoformat()
    elif isinstance(updated, str):
        try:
            datetime.date.fromisoformat(updated)
        except ValueError:
            _fail(where, f"'updated' must be a YYYY-MM-DD date, got '{updated}'")
    else:
        _fail(where, "'updated' must be a YYYY-MM-DD date")

    for field in ("notes", "help"):
        if field in item:
            item[field] = " ".join(item[field].split())


def load_items(path: Path | None = None) -> list[dict]:
    data_file = Path(path) if path else DATA_FILE
    try:
        raw = yaml.safe_load(data_file.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise UpstreamDataError(f"{data_file}: file not found") from None
    except yaml.YAMLError as exc:
        raise UpstreamDataError(f"{data_file}: invalid YAML: {exc}") from None

    if not isinstance(raw, dict) or "items" not in raw:
        _fail("<root>", "expected a top-level 'items' key")
    items = raw["items"]
    if not isinstance(items, list) or not items:
        _fail("items", "must be a non-empty list")

    seen_ids: set[str] = set()
    for index, item in enumerate(items):
        _validate_item(item, index, seen_ids)
    return items


def sorted_for_docs(items: list[dict]) -> list[dict]:
    """Things in motion first, newest movement first within a state."""
    def key(item: dict) -> tuple[int, int, str]:
        updated = datetime.date.fromisoformat(item["updated"])
        return (STATES.index(item["state"]), -updated.toordinal(), item["title"])

    return sorted(items, key=key)


def main() -> int:
    import sys

    try:
        items = load_items()
    except UpstreamDataError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    states = ", ".join(
        f"{sum(1 for i in items if i['state'] == s)} {s}" for s in STATES
    )
    announced = sum(1 for i in items if i["upstream"] in SUBMITTED_UPSTREAM)
    helping = sum(1 for i in items if i.get("help"))
    print(f"ok: {len(items)} items ({states}); {announced} upstream, {helping} help wanted")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
