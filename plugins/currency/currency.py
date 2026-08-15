#!/usr/bin/env python3
"""Say when the workspace has started asserting things nobody has re-checked.

Trellis cards carry `verify:: YYYY-MM-DD` on anything that states how something
*is* — a version, a count, what somebody owes you. `GET /api/claims` reports the
ones past their date. This plugin is the off-app half of that: the desktop shows
a count in **View → Claims**, and this tells you on a schedule, wherever you are,
so a stale card is caught by a check rather than by an agent quoting it back at
somebody.

**It never runs a `check::`.** That property names the command or endpoint that
would settle a claim, and it is written *inside a card* — which an agent, the web
clipper, or anything else with API access can write. Executing it would turn any
card in the document into arbitrary code on this machine. It is reported for a
human or an agent to run, and that is the whole of it.

**It writes nothing.** The token is read-only, so a bug here cannot damage the
document it is watching. The in-app surfacing (the panel, the menu count) is the
app's job and needs no plugin.

With no bot token it prints the message instead of sending it — how you check
the wording, and how the plugin is testable without a Telegram account.
"""

import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

UA = "Mozilla/5.0 (X11; Linux x86_64) TrellisCurrency/1.0"
TELEGRAM = "https://api.telegram.org/bot{token}/sendMessage"
MAX_LISTED = 12


def die(msg):
    print(msg, file=sys.stderr)
    print(f"Currency check failed: {msg}")
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
        body = e.read().decode(errors="replace")[:200]
        if e.code == 403:
            die("this token is confined to a basket, and /api/claims reads the whole "
                "document. Approve the plugin with document scope, or set a project id.")
        die(f"Trellis {path} → HTTP {e.code}: {body}")
    except Exception as e:  # noqa: BLE001
        die(f"Trellis {path}: {e}")


def yes(cfg, key, default=True):
    v = (cfg.get(key) or "").strip().lower()
    if not v:
        return default
    return v not in ("no", "off", "false", "0")


def send(cfg, text):
    token = (cfg.get("telegram_token") or "").strip()
    chat = (cfg.get("telegram_chat_id") or "").strip()
    if not token or not chat:
        print("--- not sent (no bot token / chat id — this is a preview) ---")
        print(text)
        print("-" * 56)
        return "Previewed the currency report — add a Telegram bot token to send it"

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
        if e.code == 401:
            die("Telegram rejected the bot token (401). Check it in Tools → Plugins.")
        if e.code == 400 and "chat not found" in detail.lower():
            die("Telegram doesn't know that chat id — message your bot once first.")
        die(f"Telegram → HTTP {e.code}: {detail}")
    except Exception as e:  # noqa: BLE001
        die(f"Telegram: {e}")
    if not out.get("ok"):
        die(f"Telegram refused the message: {out.get('description', out)}")
    return "Currency report sent"


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


def esc(s):
    return (s or "").replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def report(cfg, doc):
    q = "/claims?expired=true"
    project = (cfg.get("project") or "").strip()
    if project:
        q += f"&project={urllib.parse.quote(project)}"
    data = trellis(q)
    claims = data.get("claims") or []
    if not claims:
        return None, "clean"

    expired = [c for c in claims if c.get("bucket") == "expired"]
    unparsed = [c for c in claims if c.get("bucket") == "unparsed"]

    def line(c):
        where = esc(c.get("node_path") or c.get("node_title") or "")
        title = esc(c.get("title") or "(untitled)")
        out = [f"• {title} — <i>{where}</i>"]
        # The card said how to settle it. Repeat that, never run it.
        if c.get("check"):
            out.append(f"   check: <code>{esc(c['check'])}</code>")
        return "\n".join(out)

    parts = [f"<b>{esc(doc)}</b>", f"\n{len(claims)} claim(s) need re-checking."]
    if expired:
        parts.append(f"\n<b>Past their verify:: date ({len(expired)})</b>")
        parts += [line(c) for c in expired[:MAX_LISTED]]
        if len(expired) > MAX_LISTED:
            parts.append(f"…and {len(expired) - MAX_LISTED} more")
    if unparsed:
        # Worth calling out separately: this one is not "out of date", it is
        # "was never going to expire", which is the failure that hides longest.
        parts.append(f"\n<b>verify:: is not a readable date ({len(unparsed)})</b>")
        parts += [line(c) for c in unparsed[:MAX_LISTED]]
    parts.append("\nRe-check, correct the card, then push its verify:: date out.")

    # Keyed by which cards are stale, so an unchanged list is not re-sent every
    # interval. A notifier that repeats itself gets muted, and then it may as
    # well not exist.
    key = ",".join(sorted(str(c.get("card")) for c in claims))
    return "\n".join(parts), key


def main():
    cfg_path = os.path.join(os.environ.get("TRELLIS_PLUGIN_DIR", "."), "config.json")
    cfg = {}
    if os.path.isfile(cfg_path):
        with open(cfg_path) as f:
            cfg = json.load(f)

    inst = trellis("/instance")
    doc = inst.get("document") or "Trellis"

    text, key = report(cfg, doc)
    state = load_state()

    if text is None:
        state["last"] = "clean"
        save_state(state)
        if yes(cfg, "quiet_when_clean"):
            print(f"{doc}: every claim is current — nothing to send")
            return
        print(send(cfg, f"<b>{esc(doc)}</b>\n\nEvery claim is current."))
        return

    if state.get("last") == key:
        print(f"{doc}: the same {key.count(',') + 1} claim(s) as last time — not re-sending")
        return

    status = send(cfg, text)
    state["last"] = key
    save_state(state)
    print(status)


if __name__ == "__main__":
    main()
