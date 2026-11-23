# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>
"""Render a feature's upstream submission as a Discord message.

Plain content, not an embed: only plain content makes Discord unfurl the link.
"""

from __future__ import annotations

from schema import UPSTREAM_LABELS

UPSTREAM_EMOJI = {
    "downstream": "🏠",
    "preparing": "🛠",
    "submitted": "🔵",
    "merged": "🟢",
    "superseded": "⏭",
    "revoked": "↻",
    "rejected": "🔴",
    "stale": "⚪",
}

MAX_CONTENT = 2000


def render(item: dict) -> str:
    upstream = item["upstream"]
    header = [f"{UPSTREAM_EMOJI[upstream]} **{UPSTREAM_LABELS[upstream].upper()}**"]
    for field in ("project", "subsystem"):
        if item.get(field):
            header.append(item[field])

    byline = [f"@{author}" for author in item.get("authors", [])]
    if item.get("version"):
        byline.append(item["version"])
    byline.append(item["updated"])

    lines = [" · ".join(header), f'**{item["title"]}**']
    if item.get("notes"):
        lines.append(item["notes"])
    lines.append(" · ".join(byline))
    if item.get("help"):
        lines.append(f'🙋 **Help wanted:** {item["help"]}')
    if item.get("link", "").startswith("https://"):
        lines.append(item["link"])

    content = "\n".join(lines)
    if len(content) > MAX_CONTENT:
        raise ValueError(
            f'{item["id"]}: rendered Discord message is {len(content)} characters, '
            f"the limit is {MAX_CONTENT} — shorten 'notes' or 'title'"
        )
    return content
