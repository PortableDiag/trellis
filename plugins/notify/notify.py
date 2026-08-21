#!/usr/bin/env python3
"""Tell the operator what the workspace needs, without them opening it.

Two things get sent, matching the two triggers Trellis offers:

- **schedule** → a digest of overdue / due-today tasks, from `GET /api/tasks`.
- **on-change** → a nudge when an *agent* changed something, from
  `GET /api/changes?since=…` filtered to `actor: "api"`. This half was
  impossible until the change log existed: the old signal was a bare revision
  counter, which says *that* the document moved and never *what*.
- **on-change, for a channel card** → the message itself, quoted. A change-log
  entry holds no content, so it can only ever say *something changed* — which is
  the one thing you cannot act on from the sofa. Since v1.2.0 a `channel.say`
  entry is followed to `GET /api/cards/{cid}/channel?since=…` and the text is
  quoted with the name of whoever said it. That makes a channel a real
  conversation from the phone: you type into the card, the agent replies, and
  the reply arrives as words.

**With no bot token it prints the message instead of sending it.** That isn't a
placeholder — it's how you check the wording before wiring a bot up, and it makes
the whole plugin testable without a Telegram account.

Known limitation, deliberately not hidden: nothing fires while Trellis is closed.
It is a desktop app, not a service. A digest arrives only if the app happens to
be running when the schedule comes round.
"""

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

UA = "Mozilla/5.0 (X11; Linux x86_64) TrellisNotify/1.0"
TELEGRAM = "https://api.telegram.org/bot{token}/sendMessage"


def die(msg):
    print(msg, file=sys.stderr)
    print(f"Notifications failed: {msg}")
    sys.exit(1)


def trellis(path):
    base, token = os.environ.get("TRELLIS_API"), os.environ.get("TRELLIS_TOKEN")
    if not base or not token:
        die("not launched by Trellis (TRELLIS_API / TRELLIS_TOKEN unset)")
    req = urllib.request.Request(f"{base}{path}", headers={"X-API-Key": token, "User-Agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        die(f"Trellis {path} → HTTP {e.code}: {e.read().decode(errors='replace')[:200]}")
    except Exception as e:  # noqa: BLE001
        die(f"Trellis {path}: {e}")


def esc(text):
    """Escape for Telegram's HTML parse mode.

    Nothing escaped anything before v1.2.0, which was survivable while every
    label was a card title. Quoting a **message** changes that: a channel carries
    whatever someone typed, and one `<` in it makes the Bot API reject the whole
    notification with a 400 that names a byte offset. Ampersand first, or the
    escapes escape each other.
    """
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def clip(text, limit):
    """One line, at most `limit` characters, with an ellipsis when it was longer."""
    flat = " ".join(str(text).split())
    return flat if len(flat) <= limit else flat[: limit - 1].rstrip() + "…"


def go_link(base_host, port, kind, ident, label):
    """An `http` link that opens `kind/ident` in Trellis on the reader's device.

    **Why http and not trellis://.** Measured against the Bot API on 2026-08-18:
    a message containing `<a href="trellis://…">` is accepted with `ok: true` and
    arrives with **no link entity at all** — Telegram strips a custom scheme
    silently, and does not auto-link a bare one either. Only http(s) survives. So
    the link points at the desktop's own `/go/…` page, which is a page whose only
    job is to offer the `trellis://` link the phone's app is registered for.

    Returns the label unlinked when there is no host to build on, because a link
    to `127.0.0.1` from a phone is a link to the phone.
    """
    if not base_host:
        return esc(label)
    return f'<a href="http://{base_host}:{port}/go/{kind}/{ident}">{esc(label)}</a>'


def link_host(cfg, inst):
    """Where the phone should reach this desktop.

    Config wins, because the auto-detected answer is the address on the route out
    of this machine and that can be a VPN rather than the LAN the phone is on —
    measured here as a 100.64/10 CGNAT address on a box whose LAN is 192.168.
    `lan_host` is null when LAN access is off, and then there is honestly nothing
    to link to.
    """
    return (cfg.get("link_host") or "").strip() or (inst.get("lan_host") or "")


def yes(cfg, key, default=True):
    v = (cfg.get(key) or "").strip().lower()
    if not v:
        return default
    return v not in ("no", "off", "false", "0")


def send(cfg, text):
    """Deliver, or print when there's nothing to deliver with.

    Returns the headline for Trellis's status line. Printing rather than failing
    is the point: an unconfigured plugin that shows you the message is useful,
    while one that just errors teaches you nothing.
    """
    token = (cfg.get("telegram_token") or "").strip()
    chat = (cfg.get("telegram_chat_id") or "").strip()
    if not token or not chat:
        print("--- not sent (no bot token / chat id — this is a preview) ---")
        print(text)
        print("-" * 56)
        return "Previewed a notification — add a Telegram bot token to send it"

    body = urllib.parse.urlencode(
        {"chat_id": chat, "text": text, "parse_mode": "HTML", "disable_web_page_preview": "true"}
    ).encode()
    req = urllib.request.Request(
        TELEGRAM.format(token=token), data=body,
        headers={"Content-Type": "application/x-www-form-urlencoded", "User-Agent": UA},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            out = json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")[:200]
        # Telegram's own errors are far more useful than the status code, and
        # the two usual causes are worth naming outright.
        if e.code == 401:
            die("Telegram rejected the bot token (401). Check it in Tools → Plugins.")
        if e.code == 400 and "chat not found" in detail.lower():
            die("Telegram doesn't know that chat id — message your bot once first.")
        die(f"Telegram → HTTP {e.code}: {detail}")
    except Exception as e:  # noqa: BLE001
        die(f"Telegram: {e}")
    if not out.get("ok"):
        die(f"Telegram refused the message: {out.get('description', out)}")
    return "Notification sent"


# --- state, so the same thing isn't sent twice --------------------------------

def state_path():
    return os.path.join(os.environ.get("TRELLIS_PLUGIN_DIR", "."), "state.json")


def load_state():
    try:
        with open(state_path()) as f:
            return json.load(f)
    except Exception:  # noqa: BLE001
        return {}


def save_state(s):
    try:
        with open(state_path(), "w") as f:
            json.dump(s, f)
    except Exception as e:  # noqa: BLE001
        print(f"(could not save state: {e})", file=sys.stderr)


# --- the two messages ---------------------------------------------------------

def task_digest(cfg, doc, host="", port=0):
    q = "/tasks"
    project = (cfg.get("project") or "").strip()
    if project:
        q += f"?project={urllib.parse.quote(project)}"
    data = trellis(q)
    tasks = data.get("tasks") or []
    overdue = [t for t in tasks if t.get("bucket") == "overdue"]
    today = [t for t in tasks if t.get("bucket") == "today"]
    if not overdue and not today:
        return None, "nothing due"

    def line(t):
        where = t.get("node_path") or t.get("node_title") or ""
        due = f" · {t['due']}" if t.get("due") else ""
        title = t.get("title") or "(untitled)"
        # The title itself is the link, so the message does not grow a column of
        # "open" words. A task with no card id (there should be none) still reads.
        if t.get("card") and host:
            title = go_link(host, port, "card", t["card"], title)
        return f"• {title}{due}\n   <i>{where}</i>"

    parts = [f"<b>{doc}</b>"]
    if overdue:
        parts.append(f"\n<b>Overdue ({len(overdue)})</b>")
        parts += [line(t) for t in overdue[:10]]
        if len(overdue) > 10:
            parts.append(f"…and {len(overdue) - 10} more")
    if today:
        parts.append(f"\n<b>Due today ({len(today)})</b>")
        parts += [line(t) for t in today[:10]]
        if len(today) > 10:
            parts.append(f"…and {len(today) - 10} more")
    # The digest is keyed by its content, so an unchanged list isn't re-sent
    # every interval — a notifier that repeats itself gets muted, and then it
    # may as well not exist.
    key = f"{len(overdue)}/{len(today)}/" + ",".join(
        sorted(str(t.get("card")) for t in overdue + today)
    )
    return "\n".join(parts), key


def channel_messages(changes, state, host, port):
    """The actual text of any channel messages in this batch.

    The change log says a message *happened* — card, basket, and since v0.143.0
    which agent — but never what it said, because an entry holds no content. That
    is right for a log and useless for a notification: "an agent changed
    something" is exactly the message you cannot act on from the sofa.

    So the text is fetched per card, from a **per-card cursor** kept in state
    rather than from the change-log position. The two count different things, and
    a channel's own `seq` is the one that survives the log rotating.

    Messages from `operator` are skipped: they are the reader's own, typed on the
    phone a moment ago, and telling someone what they just said is noise.
    """
    cards = {}
    for c in changes:
        if "channel.say" in (c.get("fields") or []) and c.get("entity") == "card" and c.get("id"):
            cards[c["id"]] = c.get("title") or "a channel"
    if not cards:
        return [], set()

    seen = state.setdefault("channel_seq", {})
    lines = []
    for cid, title in sorted(cards.items()):
        key = str(cid)
        first_time = key not in seen
        since = seen.get(key, 0)
        data = trellis(f"/cards/{cid}/channel?since={since}")
        msgs = [m for m in (data.get("messages") or []) if m.get("from") != "operator"]
        # A card we have never reported on would otherwise dump its whole history
        # the first time it is touched. Only the newest is news.
        if first_time:
            msgs = msgs[-1:]
        seen[key] = data.get("seq", since)
        if not msgs:
            continue
        lines.append(f"💬 {go_link(host, port, 'card', cid, title)}")
        for m in msgs[-3:]:
            lines.append(f"   <b>{esc(m.get('from') or '?')}</b>: {esc(clip(m.get('text'), 220))}")
        if len(msgs) > 3:
            lines.append(f"   …and {len(msgs) - 3} more")
    return lines, set(cards)


def agent_changes(doc, since, state, host="", port=0):
    data = trellis(f"/changes?since={since}&limit=500")
    if data.get("truncated"):
        print("(change log had rotated past our position — reporting a count only)")
    changes = [c for c in (data.get("changes") or []) if c.get("actor") == "api"]
    if not changes:
        return None, data.get("rev", since)

    # A message is quoted, not counted. Everything else is still summarised, and
    # the two are kept apart so a conversation does not read as "3 edits".
    said, said_cards = channel_messages(changes, state, host, port)
    changes = [c for c in changes if "channel.say" not in (c.get("fields") or [])]
    if not changes:
        if not said:
            return None, data.get("rev", since)
        return "\n".join([f"<b>{esc(doc)}</b>", ""] + said), data.get("rev", since)

    baskets, titles = set(), []
    # The card each title belongs to, so the line can link to the thing that
    # changed rather than to the document in general — which is the whole point
    # of a notification you can act on from the sofa.
    card_of = {}
    for c in changes:
        if c.get("node"):
            baskets.add(c["node"])
        t = c.get("title")
        if t and t not in titles:
            titles.append(t)
        if t and c.get("entity") == "card" and c.get("id") and t not in card_of:
            card_of[t] = c["id"]
    props = [c for c in changes if c.get("property")]

    parts = [f"<b>{esc(doc)}</b>"]
    if said:
        parts += [""] + said
    parts.append(f"\nAn agent made {len(changes)} other change(s)" if said
                 else f"\nAn agent made {len(changes)} change(s)")
    if baskets:
        parts.append(f"in {len(baskets)} basket(s).")
    titles = [t for t in titles if card_of.get(t) not in said_cards]
    for t in titles[:8]:
        parts.append("• " + go_link(host, port, "card", card_of[t], t) if t in card_of else f"• {esc(t)}")
    if len(titles) > 8:
        parts.append(f"…and {len(titles) - 8} more")
    for c in props[:5]:
        k, v = c["property"]
        name = c.get("title") or "a card"
        if c.get("entity") == "card" and c.get("id"):
            name = go_link(host, port, "card", c["id"], name)
        else:
            name = esc(name)
        parts.append(f"• <i>{name}</i> — {esc(k)} → {esc(v)}")
    return "\n".join(parts), data.get("rev", since)


def main():
    cfg_path = os.path.join(os.environ.get("TRELLIS_PLUGIN_DIR", "."), "config.json")
    cfg = {}
    if os.path.isfile(cfg_path):
        with open(cfg_path) as f:
            cfg = json.load(f)

    inst = trellis("/instance")
    doc = inst.get("document") or "Trellis"
    host = link_host(cfg, inst)
    port = inst.get("port") or 0
    trigger = os.environ.get("TRELLIS_TRIGGER", "manual")
    state = load_state()

    # An agent edit is only interesting as it happens, so it belongs to the
    # change trigger; a digest is a standing summary, so it belongs to the
    # schedule. Running by hand shows the digest, because that's what someone
    # clicking "Run now" wants to see.
    if trigger == "change":
        if not yes(cfg, "agent_edits"):
            print("Agent-edit notifications are off")
            return
        since = os.environ.get("TRELLIS_SINCE") or state.get("since") or 0
        text, new_since = agent_changes(doc, since, state, host, port)
        state["since"] = new_since
        save_state(state)
        if not text:
            print("Nothing an agent did — no notification")
            return
        print(send(cfg, text))
        return

    if not yes(cfg, "digest"):
        print("Task digest is off")
        return
    text, key = task_digest(cfg, doc, host, port)
    if not text:
        print("Nothing overdue or due today — no notification")
        return
    today_stamp = time.strftime("%Y-%m-%d")
    if trigger == "schedule" and state.get("digest_key") == key and state.get("digest_day") == today_stamp:
        print("Digest unchanged since the last one — not repeating it")
        return
    state["digest_key"], state["digest_day"] = key, today_stamp
    save_state(state)
    print(send(cfg, text))


if __name__ == "__main__":
    main()
