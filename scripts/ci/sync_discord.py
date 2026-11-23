#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>
"""Announce the upstream submissions from website/data/features.yml in Discord.

One webhook message per submission; the message id is remembered in
scripts/ci/state/discord-state.json so a status change edits the existing
message instead of posting a new one.

Usage:
    DISCORD_UPSTREAM_WEBHOOK=https://discord.com/api/webhooks/... \\
        python3 scripts/ci/sync_discord.py [--dry-run]
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import discord_webhook as discord  # noqa: E402
from render_discord import render  # noqa: E402
from schema import SUBMITTED_UPSTREAM, UpstreamDataError, load_items  # noqa: E402

STATE_FILE = HERE / "state" / "discord-state.json"
STATE_VERSION = 1
WEBHOOK_ENV = "DISCORD_UPSTREAM_WEBHOOK"


def sync(webhook: str, dry_run: bool) -> int:
    items = load_items()
    state = discord.load_state(STATE_FILE, STATE_VERSION)
    known = set(state["messages"])

    # The channel announces submissions, not the board. Everything else is
    # documentation-site only, unless it has already been announced once.
    entries = [
        discord.Entry(item["id"], render(item), item["upstream"])
        for item in items
        if item["upstream"] in SUBMITTED_UPSTREAM or item["id"] in known
    ]
    return discord.sync(webhook, entries, state, STATE_FILE, dry_run)


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
    except (discord.SyncError, UpstreamDataError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
