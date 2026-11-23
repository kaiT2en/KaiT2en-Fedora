# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>
"""Render a blog post as a Discord message.

Plain content with the embed suppressed by the sync: the channel is a readable
list of teasers, not a wall of identical preview cards.
"""

from __future__ import annotations

import html
import re

MAX_CONTENT = 2000
MAX_TEASER = 400

TAG_RE = re.compile(r"<[^>]+>")
PARAGRAPH_RE = re.compile(r"<p>(.*?)</p>", re.DOTALL)
SENTENCE_END_RE = re.compile(r"(?<=[.!?])\s+")


def _first_paragraph(post_html: str) -> str:
    """Fallback teaser for a post without a summary."""
    match = PARAGRAPH_RE.search(post_html)
    if not match:
        return ""
    text = " ".join(html.unescape(TAG_RE.sub("", match.group(1))).split())

    teaser = ""
    for sentence in SENTENCE_END_RE.split(text)[:3]:
        candidate = f"{teaser} {sentence}".strip()
        if teaser and len(candidate) > MAX_TEASER:
            break
        teaser = candidate
    return teaser[:MAX_TEASER].rstrip()


def render(post, base_url: str) -> str:
    teaser = post.summary or _first_paragraph(post.html)

    byline = [post.author, *post.tags]
    lines = [
        f"📝 **NEW POST** · {post.date.strftime('%d %b %Y')}",
        f"**{post.title}**",
    ]
    if teaser:
        lines.append(teaser)
    lines.append(" · ".join(byline))
    lines.append(f"{base_url.rstrip('/')}/{post.url}")

    content = "\n".join(lines)
    if len(content) > MAX_CONTENT:
        raise ValueError(
            f"{post.slug}: rendered Discord message is {len(content)} characters, "
            f"the limit is {MAX_CONTENT} — shorten 'summary' or 'title'"
        )
    return content
