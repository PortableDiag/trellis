#!/usr/bin/env python3
"""
Trellis snip watcher — tool-agnostic screenshot ingest.

Watches a folder and turns any image dropped there (by Kazam, Print Screen,
Spectacle, Flameshot, a file manager — anything) into a Trellis image card, so
your capture habit is unchanged and captures just land in your notes.

No third-party dependencies (Python stdlib only). Talks to the Trellis agent API,
reading the key/port from Trellis's own settings, exactly like Meet Scribe.

Config via environment variables (all optional):
  TRELLIS_SNIP_DIR    folder to watch         (default: ~/Pictures/screenshots)
  TRELLIS_SNIP_NODE   target node title       (default: "Screenshots")
  TRELLIS_API_HOST    API host                (default: 127.0.0.1)
  TRELLIS_SNIP_POLL   poll seconds            (default: 1.5)

Pre-existing files in the folder are ignored at startup — only images that appear
while the watcher runs are imported.
"""

import base64
import json
import os
import re
import sys
import time
import urllib.request
from pathlib import Path

IMAGE_EXTS = {".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp"}
STATE_DIR = Path.home() / ".local/share/trellis-snip"
LOG_FILE = STATE_DIR / "snip-watcher.log"
SETTINGS = Path.home() / ".local/share/trellis/app.ron"


def log(msg):
    line = f"{time.strftime('%Y-%m-%d %H:%M:%S')}  {msg}"
    print(line, flush=True)
    try:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        with open(LOG_FILE, "a") as f:
            f.write(line + "\n")
    except OSError:
        pass


def read_settings():
    """Pull api_key and api_port out of Trellis's RON settings file."""
    try:
        text = SETTINGS.read_text()
    except OSError:
        return None, None
    key = re.search(r'"api_key"\s*:\s*"([^"]*)"', text)
    port = re.search(r'"api_port"\s*:\s*"?(\d+)"?', text)
    return (key.group(1) if key else None,
            int(port.group(1)) if port else 7373)


class Trellis:
    def __init__(self, host, port, key):
        self.base = f"http://{host}:{port}/api"
        self.key = key

    def _req(self, method, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.base + path, data=data, method=method)
        req.add_header("X-API-Key", self.key)
        if data is not None:
            req.add_header("Content-Type", "application/json")
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.loads(r.read().decode())

    def ensure_node(self, title):
        """Find a root node with this title, or create one. Returns its id."""
        tree = self._req("GET", "/tree")
        for n in tree.get("roots", []):
            if n.get("title") == title:
                return n["id"]
        created = self._req("POST", "/nodes", {"title": title})
        log(f"created target node '{title}' (id {created['id']})")
        return created["id"]

    def add_image(self, node_id, name, image_bytes):
        b64 = base64.b64encode(image_bytes).decode()
        return self._req(
            "POST", f"/nodes/{node_id}/cards",
            {"kind": "image", "title": name, "image_base64": b64},
        )


def is_stable(path, prev_sizes):
    """True once a file's size has stopped changing (finished being written)."""
    try:
        size = path.stat().st_size
    except OSError:
        return False
    stable = prev_sizes.get(path) == size and size > 0
    prev_sizes[path] = size
    return stable


def main():
    watch_dir = Path(os.environ.get("TRELLIS_SNIP_DIR",
                                    Path.home() / "Pictures/screenshots")).expanduser()
    node_title = os.environ.get("TRELLIS_SNIP_NODE", "Screenshots")
    host = os.environ.get("TRELLIS_API_HOST", "127.0.0.1")
    poll = float(os.environ.get("TRELLIS_SNIP_POLL", "1.5"))

    key, port = read_settings()
    if not key:
        log("No API key in Trellis settings — enable the API in Tools → Settings. Exiting.")
        sys.exit(1)

    watch_dir.mkdir(parents=True, exist_ok=True)
    log(f"watching {watch_dir} → node '{node_title}' via {host}:{port}")

    trellis = Trellis(host, port, key)
    # Ignore whatever is already in the folder.
    seen = {p.name for p in watch_dir.iterdir() if p.is_file()}
    prev_sizes = {}
    pending = set()  # stable-check across polls before importing
    node_id = None

    while True:
        try:
            for path in sorted(watch_dir.iterdir()):
                if not path.is_file() or path.suffix.lower() not in IMAGE_EXTS:
                    continue
                if path.name in seen:
                    continue
                if not is_stable(path, prev_sizes):
                    pending.add(path.name)
                    continue
                # Stable and new — import it.
                try:
                    data = path.read_bytes()
                    if node_id is None:
                        node_id = trellis.ensure_node(node_title)
                    trellis.add_image(node_id, path.name, data)
                    log(f"imported {path.name} ({len(data)} bytes)")
                except Exception as e:  # noqa: BLE001 — never let one file kill the loop
                    log(f"failed to import {path.name}: {e}")
                    node_id = None  # re-resolve node next time (server may have restarted)
                    continue
                seen.add(path.name)
                pending.discard(path.name)
                prev_sizes.pop(path, None)
        except Exception as e:  # noqa: BLE001
            log(f"scan error: {e}")
        time.sleep(poll)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
