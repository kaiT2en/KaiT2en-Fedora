# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>
"""Post and edit Discord messages through an incoming webhook.

Shared by the sync scripts in this directory. Each of them renders its own
messages and keeps its own state file; everything below is the transport and
the create-or-edit bookkeeping they have in common.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

WEBHOOK_PREFIXES = ("https://discord.com/api/webhooks/", "https://discordapp.com/api/webhooks/")
DRY_RUN_WEBHOOK = "https://discord.com/api/webhooks/0/dry-run"

# Message flag 1 << 2: the link is still clickable, Discord just does not
# expand it into a preview card.
SUPPRESS_EMBEDS = 4

REQUEST_PAUSE = 0.6
MAX_ATTEMPTS = 5


class SyncError(Exception):
    pass


@dataclass
class Entry:
    """One message to keep in sync. `note` only shows up in the run log."""

    key: str
    content: str
    note: str = ""


def _redact(url: str) -> str:
    head, _, _ = url.rpartition("/")
    return f"{head}/<token>"


def _request(url: str, payload: dict, method: str) -> dict:
    body = json.dumps(payload).encode("utf-8")
    for attempt in range(1, MAX_ATTEMPTS + 1):
        request = urllib.request.Request(
            url,
            data=body,
            method=method,
            headers={
                "Content-Type": "application/json",
                "User-Agent": "KaiT2en-sync/1.0 (+https://github.com/kaiT2en/KaiT2en-Fedora)",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                raw = response.read().decode("utf-8")
                return json.loads(raw) if raw else {}
        except urllib.error.HTTPError as exc:
            detail = exc.read().decode("utf-8", "replace")[:400]
            if exc.code == 429 and attempt < MAX_ATTEMPTS:
                wait = 5.0
                try:
                    wait = float(json.loads(detail).get("retry_after", wait))
                except (ValueError, AttributeError):
                    pass
                print(f"  rate limited, retrying in {wait:.1f}s", flush=True)
                time.sleep(min(wait, 60.0) + 0.25)
                continue
            if exc.code >= 500 and attempt < MAX_ATTEMPTS:
                time.sleep(2.0 * attempt)
                continue
            raise SyncError(
                f"{method} {_redact(url)} failed with HTTP {exc.code}: {detail}"
            ) from None
        except urllib.error.URLError as exc:
            if attempt < MAX_ATTEMPTS:
                time.sleep(2.0 * attempt)
                continue
            raise SyncError(f"{method} {_redact(url)} failed: {exc.reason}") from None
    raise SyncError(f"{method} {_redact(url)} failed after {MAX_ATTEMPTS} attempts")


def _payload(content: str) -> dict:
    # The @handles are plain-text GitHub names and must never become real pings.
    return {"content": content, "allowed_mentions": {"parse": []}}


def resolve_webhook(env_name: str, dry_run: bool) -> str:
    """Read and sanity-check the webhook URL, or return a placeholder."""
    webhook = os.environ.get(env_name, "").strip()
    if not webhook:
        if dry_run:
            return DRY_RUN_WEBHOOK
        raise SyncError(f"{env_name} is not set")
    if not webhook.startswith(WEBHOOK_PREFIXES):
        raise SyncError(f"{env_name} does not look like a Discord webhook URL")
    return webhook.rstrip("/")


def post(webhook: str, content: str, suppress_embeds: bool = False) -> str:
    payload = _payload(content)
    if suppress_embeds:
        payload["flags"] = SUPPRESS_EMBEDS
    separator = "&" if "?" in webhook else "?"
    result = _request(f"{webhook}{separator}wait=true", payload, "POST")
    message_id = result.get("id")
    if not message_id:
        raise SyncError("Discord accepted the message but returned no message id")
    return str(message_id)


def patch(webhook: str, message_id: str, content: str) -> bool:
    """Return False if the message is gone and has to be posted again."""
    # No flags here: editing a webhook message cannot change them, and the ones
    # set when it was posted stay in place.
    try:
        _request(f"{webhook}/messages/{message_id}", _payload(content), "PATCH")
        return True
    except SyncError as exc:
        if "HTTP 404" in str(exc):
            return False
        raise


def load_state(path: Path, version: int) -> dict:
    if not path.exists():
        return {"version": version, "messages": {}}
    try:
        state = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise SyncError(f"{path.name} is not valid JSON: {exc}") from None
    if state.get("version") != version:
        raise SyncError(f"{path.name} has version {state.get('version')}, expected {version}")
    state.setdefault("messages", {})
    return state


def save_state(path: Path, state: dict, order: list[str]) -> None:
    messages = state["messages"]
    ranked = sorted(messages, key=lambda k: (order.index(k) if k in order else len(order), k))
    state["messages"] = {key: messages[key] for key in ranked}
    path.write_text(json.dumps(state, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def sync(
    webhook: str,
    entries: list[Entry],
    state: dict,
    state_file: Path,
    dry_run: bool,
    suppress_embeds: bool = False,
) -> int:
    """Post or edit one message per entry, in the order they are given."""
    messages = state["messages"]
    created = updated = unchanged = 0
    changed_state = False

    for entry in entries:
        key, content = entry.key, entry.content
        digest = hashlib.sha256(content.encode("utf-8")).hexdigest()
        known = messages.get(key)

        if known and known.get("hash") == digest:
            unchanged += 1
            continue

        action = "update" if known else "post"
        note = f" [{entry.note}]" if entry.note else ""
        print(f"{'would ' if dry_run else ''}{action}: {key}{note}", flush=True)
        if dry_run:
            created += action == "post"
            updated += action == "update"
            continue

        if known:
            if patch(webhook, known["message_id"], content):
                messages[key] = {"message_id": known["message_id"], "hash": digest}
                updated += 1
            else:
                print(f"  message {known['message_id']} is gone, posting a new one", flush=True)
                message_id = post(webhook, content, suppress_embeds)
                messages[key] = {"message_id": message_id, "hash": digest}
                created += 1
        else:
            message_id = post(webhook, content, suppress_embeds)
            messages[key] = {"message_id": message_id, "hash": digest}
            created += 1

        changed_state = True
        time.sleep(REQUEST_PAUSE)

    order = [entry.key for entry in entries]
    for orphan in sorted(set(messages) - set(order)):
        print(
            f"warning: '{orphan}' has a Discord message ({messages[orphan]['message_id']}) "
            "but no source entry any more; the message is left untouched",
            file=sys.stderr,
        )

    if changed_state and not dry_run:
        save_state(state_file, state, order)

    print(f"done: {created} posted, {updated} updated, {unchanged} unchanged")
    return 0
