#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>
"""Announce the blog posts from website/blog/ in Discord.

One webhook message per published post; the message id is remembered in
scripts/ci/state/blog-discord-state.json so an edited post updates the existing
message instead of posting a new one.

Usage:
    DISCORD_BLOG_WEBHOOK=https://discord.com/api/webhooks/... \\
        python3 scripts/ci/sync_blog_discord.py [--dry-run]
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "website"))

import discord_webhook as discord  # noqa: E402
from build import BuildError, load_posts, read_config  # noqa: E402
from render_blog_discord import render  # noqa: E402

STATE_FILE = HERE / "state" / "blog-discord-state.json"
STATE_VERSION = 1
WEBHOOK_ENV = "DISCORD_BLOG_WEBHOOK"


def sync(webhook: str, dry_run: bool) -> int:
    config = read_config()
    base_url = config["site"]["url"]
    state = discord.load_state(STATE_FILE, STATE_VERSION)

    # load_posts() sorts newest first; announce oldest first so the channel
    # reads in the order the posts were written.
    entries = [
        discord.Entry(post.slug, render(post, base_url), post.date.isoformat())
        for post in reversed(load_posts(config))
    ]
    return discord.sync(webhook, entries, state, STATE_FILE, dry_run, suppress_embeds=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="report what would be posted or edited without calling Discord",
    )
    args = parser.parse_args()

    try:
        webhook = discord.resolve_webhook(WEBHOOK_ENV, args.dry_run)
        return sync(webhook, args.dry_run)
    except (discord.SyncError, BuildError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
