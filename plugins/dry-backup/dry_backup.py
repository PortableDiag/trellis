#!/usr/bin/env python3
"""Back up a Trellis document into a Dry space.

One-way and idempotent: every basket and card becomes a Dry item keyed by its
Trellis id, so running it again **updates** rather than duplicating. Nothing is
ever deleted from Dry — a card removed in Trellis is left in place rather than
disappearing, because a backup that deletes things it didn't see this run is how
backup tools lose work.

Trellis supplies these in the environment; nothing is passed on argv, so the
token can't be read out of the process list:

    TRELLIS_API         base URL, e.g. http://127.0.0.1:7373/api
    TRELLIS_TOKEN       this plugin's scoped token (read-only)
    TRELLIS_PLUGIN_DIR  this directory — where config.json lives
    TRELLIS_NODE        set only when run from a basket's right-click menu

The Dry credential is *this plugin's* secret, not Trellis's, so it lives in
`config.json` beside this script:

    {"demoAuthKey": "…", "space": "Trellis backup"}

Stdout is the run log; the last non-empty line becomes the status Trellis shows.
Exit non-zero on failure.
"""

import json
import os
import sys
import urllib.error
import urllib.request

DRY_API = "https://dry.ai/api/dbcrud"

# Dry's edge rejects a default urllib/curl User-Agent with an HTML 403 before the
# request ever reaches the application. The failure is confusing precisely
# because the body is HTML rather than the API's JSON envelope, so it looks like
# an outage rather than a blocked client. Always send one.
USER_AGENT = "Mozilla/5.0 (X11; Linux x86_64) TrellisDryBackup/1.0"

# Read pages so a large document doesn't silently stop at Dry's default of 1000.
PAGE = 500


def die(msg):
    print(msg, file=sys.stderr)
    print(f"Dry backup failed: {msg}")
    sys.exit(1)


# --- Trellis ----------------------------------------------------------------

def trellis(path):
    base = os.environ.get("TRELLIS_API")
    token = os.environ.get("TRELLIS_TOKEN")
    if not base or not token:
        die("not launched by Trellis (TRELLIS_API / TRELLIS_TOKEN unset)")
    req = urllib.request.Request(
        f"{base}{path}",
        headers={"X-API-Key": token, "User-Agent": USER_AGENT},
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        body = e.read().decode(errors="replace")[:300]
        if e.code == 403:
            die(f"Trellis refused the request ({e.code}). This plugin is "
                f"read-only; it should not be writing. {body}")
        die(f"Trellis {path} → HTTP {e.code}: {body}")
    except Exception as e:  # noqa: BLE001 - report whatever went wrong
        die(f"Trellis {path}: {e}")


# --- Dry --------------------------------------------------------------------

def dry(key, op, **body):
    payload = dict(body)
    payload["demoAuthKey"] = key
    payload["op"] = op
    req = urllib.request.Request(
        DRY_API,
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json", "User-Agent": USER_AGENT},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            raw = r.read().decode()
    except urllib.error.HTTPError as e:
        raw = e.read().decode(errors="replace")
        try:
            parsed = json.loads(raw)
        except ValueError:
            # An unparseable body means the edge answered, not the API — most
            # often the User-Agent block above.
            die(f"Dry returned HTTP {e.code} with a non-JSON body "
                f"(blocked before reaching the API?): {raw[:200]}")
        die(f"Dry {op} → HTTP {e.code}: {parsed.get('error', raw[:200])}")
    except Exception as e:  # noqa: BLE001
        die(f"Dry {op}: {e}")

    try:
        out = json.loads(raw)
    except ValueError:
        die(f"Dry {op}: response was not JSON: {raw[:200]}")
    if not out.get("success"):
        die(f"Dry {op}: {out.get('error', 'unknown error')}")
    return out.get("data", {})


def check_import_report(data, what):
    """A 200 from `create` does not mean every item landed.

    The importer reports per-item outcomes inside `data.report`; trusting the
    status code alone would report a successful backup that silently dropped
    rows. Fail loudly instead.
    """
    report = data.get("report") or []
    bad = [line for line in report if "Error" in line or "error for" in line.lower()]
    if bad:
        die(f"Dry reported failures importing {what}: {' | '.join(bad[:3])}")
    return report


# --- the backup -------------------------------------------------------------

# `uniqueKey` on the Trellis id turns `create` into an upsert: re-running matches
# the existing item and edits it, with no read-first round trip and no race.
NODE_FIELDS = [
    {"name": "TrellisId", "type": "shortText", "uniqueKey": True},
    {"name": "Title", "type": "shortText"},
    {"name": "Path", "type": "shortText", "optional": True},
    {"name": "Cards", "type": "number", "optional": True},
]

CARD_FIELDS = [
    {"name": "TrellisId", "type": "shortText", "uniqueKey": True},
    {"name": "Title", "type": "shortText", "optional": True},
    {"name": "Body", "type": "longText", "optional": True},
    {"name": "Kind", "type": "shortText", "optional": True},
    # Basket and Tags stay **scalar** on purpose. A `reference` field
    # auto-creates a new referenced item for any value it can't match, so one
    # odd value would litter the space with junk items.
    {"name": "Basket", "type": "shortText", "optional": True},
    {"name": "Tags", "type": "shortText", "optional": True},
    {"name": "Due", "type": "shortText", "optional": True},
    {"name": "Status", "type": "shortText", "optional": True},
]


def ensure_space(key, name):
    """Find the backup space by name, creating it the first time."""
    try:
        dry(key, "describe", space=name)
        print(f"Using existing Dry space “{name}”")
        return name
    except SystemExit:
        pass  # describe failing means it isn't there yet
    data = dry(key, "create-space", name=name,
               description="Backup of a Trellis document. One item per basket and card.")
    print(f"Created Dry space “{name}” ({data.get('id')})")
    return data.get("id") or name


def ensure_types(key, space):
    existing = {t.get("name") for t in dry(key, "describe", space=space).get("types", [])}
    for name, fields in (("TrellisBasket", NODE_FIELDS), ("TrellisCard", CARD_FIELDS)):
        if name in existing:
            continue
        dry(key, "create-type", space=space, name=name, fields=fields)
        print(f"Created type {name}")


def node_path(nodes, nid):
    """Breadcrumb for a node, so a card's basket is identifiable when names repeat."""
    parts, seen = [], set()
    cur = nid
    while cur is not None and cur not in seen:
        seen.add(cur)
        n = nodes.get(cur)
        if not n:
            break
        parts.append(n.get("title") or str(cur))
        cur = n.get("parent")
    return " › ".join(reversed(parts))


def main():
    key_file = os.path.join(os.environ.get("TRELLIS_PLUGIN_DIR", "."), "config.json")
    if not os.path.isfile(key_file):
        die(f"no config.json in this plugin's folder. Create {key_file} containing "
            '{"demoAuthKey": "<your Dry access key>"}')
    with open(key_file) as f:
        cfg = json.load(f)
    key = cfg.get("demoAuthKey")
    if not key:
        die("config.json has no demoAuthKey")

    inst = trellis("/instance")
    doc = inst.get("document") or "untitled"
    space_name = cfg.get("space") or f"Trellis backup — {doc}"

    only = os.environ.get("TRELLIS_NODE")  # set when run from a basket's menu

    all_nodes = {n["id"]: n for n in trellis("/nodes").get("nodes", [])}
    if only:
        # Invoked on one basket: back up that subtree only.
        want, stack = set(), [int(only)]
        while stack:
            nid = stack.pop()
            if nid in want:
                continue
            want.add(nid)
            stack.extend(all_nodes.get(nid, {}).get("children", []) or [])
        nodes = {k: v for k, v in all_nodes.items() if k in want}
        print(f"Backing up “{os.environ.get('TRELLIS_NODE_TITLE', only)}” "
              f"({len(nodes)} baskets) from {doc}")
    else:
        nodes = all_nodes
        print(f"Backing up all of {doc} ({len(nodes)} baskets)")

    space = ensure_space(key, space_name)
    ensure_types(key, space)

    basket_items, card_items = [], []
    for nid, n in nodes.items():
        path = node_path(all_nodes, nid)
        basket_items.append({"type": "TrellisBasket", "fields": {
            "TrellisId": str(nid),
            "Title": n.get("title") or "",
            "Path": path,
            "Cards": n.get("cards") or 0,
        }})
        for c in trellis(f"/nodes/{nid}/cards").get("cards", []):
            props = {p["key"]: p["value"] for p in (c.get("properties") or [])}
            card_items.append({"type": "TrellisCard", "fields": {
                "TrellisId": str(c["id"]),
                "Title": c.get("title") or "",
                "Body": c.get("body") or "",
                "Kind": c.get("kind") or "",
                "Basket": path,
                "Tags": " ".join(c.get("tags") or []),
                "Due": props.get("due", ""),
                "Status": props.get("status", ""),
            }})

    sent = 0
    for label, items in (("baskets", basket_items), ("cards", card_items)):
        for i in range(0, len(items), PAGE):
            batch = items[i:i + PAGE]
            data = dry(key, "create", space=space, items=batch)
            check_import_report(data, label)
            sent += len(batch)
            print(f"  sent {len(batch)} {label} ({i + len(batch)}/{len(items)})")

    print(f"Backed up {len(basket_items)} baskets and {len(card_items)} cards "
          f"to Dry space “{space_name}”")


if __name__ == "__main__":
    main()
