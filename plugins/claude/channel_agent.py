#!/usr/bin/env python3
"""Answer channel cards, so a card can *start* a turn.

A channel card carries a conversation, and until this existed the only thing that
ever read one was an agent the operator had already summoned in a terminal. That
makes the card strictly worse than the terminal it was supposed to replace: you
would have to go and say "I replied" in order to be replied to. The transport was
built and the thing that makes it useful was not.

This is that thing. Trellis already runs plugins **on-change**, already mints a
scoped token, and already has an approval gate — so the waker is a plugin rather
than a daemon, and needs no new process to remember to start.

**What it does.** On every change, ask `GET /api/channels?agent=claude` for the
conversations addressed to it, read each one past a stored per-card cursor, and
for any message that is *not its own* run `claude -p` with the thread and post the
answer back with `POST …/say`.

**The working directory belongs to the PROJECT, not to this plugin.** v1.0.0 had
one `cwd` for every channel in the document, which meant a card in a NodeJS
project was answered by an agent sitting in the Trellis repo — the answer would be
confidently about the wrong codebase, and twelve projects would all share one
working tree. The operator said so; they were right.

A channel knows its project: `GET /api/channels` returns `node_path`, whose first
segment is the root basket. `roots` maps those names to directories, one per line,
and each channel is answered in its own. **A channel whose project is not mapped
is skipped, never answered from somewhere else** — a reply computed in the wrong
repository is worse than no reply, and unlike no reply it looks like an answer.

The map lives here rather than on the card because a filesystem path belongs to
one machine. That is the same call Trellis already made for desktop-mode window
placement: a coordinate — or a path — must not ride in a document that syncs to
the phone and into every backup.

**Why the cursor is per card and stored here.** The change log rotates, and its
sequence counts something else entirely. A channel's own `seq` is the durable
position, and keeping it in the plugin's state is what makes a restart resume
rather than replay.

**The loop guard is the whole safety story.** Posting into a channel *is* a
document change, which fires this trigger again. Nothing here answers a message
whose sender is this plugin's own name, so the second run finds nothing new and
stops. Get that wrong and it is an infinite sequence of model calls.
"""

import json
import os
import subprocess
import sys
import urllib.error
import urllib.request

def me():
    """The agent name this install answers as — its own manifest `name`.

    Read rather than hardcoded so the plugin can be **copied per agent**. A
    channel is addressed to a name, and an operator with twelve agents needs
    twelve answerers, not one that grabs everything: copy this folder to
    `plugins/<agent>/`, change `name` and `title` in its `plugin.json`, and that
    install answers `GET /api/channels?agent=<agent>` and posts under that name.
    Each gets its own approval, its own token, its own `roots` map and its own
    cursor, which is what keeps two agents out of one working tree.

    The manifest name is the right source because the server takes a scoped
    token's own label as authoritative and ignores any `X-Agent` a caller
    declares — the credential wins, and the credential is minted per plugin name.
    Guessing anything else here would post under a name the server then overrides.
    """
    try:
        path = os.path.join(os.environ.get("TRELLIS_PLUGIN_DIR", "."), "plugin.json")
        with open(path) as fh:
            name = (json.load(fh).get("name") or "").strip()
        if name:
            return name
    except Exception:  # noqa: BLE001
        pass
    return "claude"


ME = me()
UA = "TrellisChannelAgent/1.1"


def out(msg):
    print(msg, flush=True)


def die(msg):
    print(msg, file=sys.stderr)
    out(f"Channel agent failed: {msg}")
    sys.exit(1)


def api(method, path, body=None):
    base, token = os.environ.get("TRELLIS_API"), os.environ.get("TRELLIS_TOKEN")
    if not base or not token:
        die("not launched by Trellis (TRELLIS_API / TRELLIS_TOKEN unset)")
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"{base}{path}",
        data=data,
        method=method,
        headers={
            "X-API-Key": token,
            "User-Agent": UA,
            "Content-Type": "application/json",
            # **Say who we are, or the loop guard cannot hold.** A scoped plugin
            # token carries its own label and the server prefers that, so this is
            # redundant in normal use — but run with the instance key (Run button,
            # a test, an unscoped install) there is no label, the reply is
            # attributed to `operator`, and the guard below then reads this
            # plugin's own words as a message to answer. Measured, not imagined:
            # without this header the third run answered its own second message.
            "X-Agent": ME,
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read().decode()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")[:200]
        die(f"Trellis {method} {path} -> HTTP {e.code}: {detail}")
    except Exception as e:  # noqa: BLE001
        die(f"Trellis {method} {path}: {e}")


def cfg():
    p = os.path.join(os.environ.get("TRELLIS_PLUGIN_DIR", "."), "config.json")
    if os.path.isfile(p):
        with open(p) as f:
            return json.load(f)
    return {}


def state_path():
    return os.path.join(os.environ.get("TRELLIS_PLUGIN_DIR", "."), "state.json")


def load_state():
    try:
        with open(state_path()) as f:
            return json.load(f)
    except Exception:  # noqa: BLE001
        return {}


def save_state(s):
    tmp = state_path() + ".tmp"
    with open(tmp, "w") as f:
        json.dump(s, f)
    os.replace(tmp, state_path())


def yes(c, key, default=True):
    v = (c.get(key) or "").strip().lower()
    return default if not v else v not in ("no", "off", "false", "0")


def num(c, key, default):
    try:
        return int(str(c.get(key) or "").strip())
    except (TypeError, ValueError):
        return default


def roots(c):
    """`Project = /path` per line, folded for case-insensitive lookup.

    Keyed on the ROOT basket name rather than the full `node_path` so that every
    channel anywhere under a project inherits it — a project's A2A card sits in
    the same tree as its workspace channel and wants the same checkout.
    """
    out = {}
    for line in (c.get("roots") or "").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        name, _, path = line.partition("=")
        name, path = name.strip(), path.strip()
        if name and path:
            out[name.casefold()] = path
    return out


def project_of(ch):
    """The root basket a channel lives under — the first segment of node_path."""
    return (ch.get("node_path") or "").split(" › ")[0].strip()


def transcript(messages, limit=24):
    """The thread, as plain text, newest last.

    Trimmed to the last `limit` because a long-running channel would otherwise
    grow the prompt without bound, and the recent exchange is what a reply needs.
    """
    lines = []
    for m in messages[-limit:]:
        who = m.get("from") or "?"
        lines.append(f"[{who}] {m.get('text', '')}")
    return "\n\n".join(lines)


def ask_claude(prompt, cwd, timeout):
    """Run one headless turn. Returns the text, or raises with a usable reason."""
    try:
        p = subprocess.run(
            ["claude", "-p", prompt],
            cwd=cwd,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except FileNotFoundError:
        raise RuntimeError(
            "`claude` is not on PATH for the Trellis process. Install the CLI, or "
            "launch Trellis from a shell that has it."
        )
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"gave up after {timeout}s")
    if p.returncode != 0:
        err = (p.stderr or "").strip().splitlines()
        raise RuntimeError(err[-1] if err else f"claude exited {p.returncode}")
    text = (p.stdout or "").strip()
    if not text:
        raise RuntimeError("claude returned nothing")
    return text


def main():
    c = cfg()
    trigger = os.environ.get("TRELLIS_TRIGGER", "manual")
    if trigger == "change" and not yes(c, "enabled"):
        out("Automatic answering is off")
        return

    timeout = num(c, "timeout_secs", 240)
    budget = num(c, "max_turns_per_run", 3)
    where = roots(c)
    # Deliberately no default. v1.0.0 fell back to the plugin's own folder, so an
    # unconfigured install answered every channel from a directory containing
    # nothing but this script — which reads as the model being useless rather than
    # as the plugin being unconfigured.
    fallback = (c.get("cwd") or "").strip()

    channels = api("GET", f"/channels?agent={ME}").get("channels") or []
    if not channels:
        out(f"No channels addressed to '{ME}'")
        return

    state = load_state()
    seen = state.setdefault("cursor", {})
    answered = 0
    looked = 0
    problems = []

    for ch in channels:
        if answered >= budget:
            out(f"Stopped at {budget} answers this run; the rest wait for the next change")
            break
        cid = ch["card"]
        key = str(cid)
        since = seen.get(key)
        # First sight of a channel is a starting point, not a backlog to work
        # through: adopt its current position and answer only what comes next.
        if since is None:
            seen[key] = ch.get("seq", 0)
            out(f"Watching #{cid} {ch.get('title','')} from seq {seen[key]}")
            continue

        data = api("GET", f"/cards/{cid}/channel?since={since}")
        msgs = data.get("messages") or []
        # The loop guard: never answer this plugin's own words. Posting is itself
        # a document change, so this trigger fires again on every reply — get this
        # wrong and it is an unbounded sequence of model calls.
        fresh = [m for m in msgs if (m.get("from") or "") != ME]
        looked += 1
        if not fresh:
            seen[key] = data.get("seq", since)
            continue

        # Where this project's agent works. Unmapped is a SKIP, not a guess: the
        # cursor is left alone so the message is answered as soon as the map
        # names its project, rather than being burned on a wrong-directory reply.
        project = project_of(ch)
        cwd = where.get(project.casefold()) or fallback
        if not cwd:
            out(
                f"#{cid}: no working directory for project '{project}' — add "
                f"'{project} = /path/to/checkout' to this plugin's `roots` setting. "
                f"Not answering from somewhere else."
            )
            problems.append(f"#{cid}: project '{project}' is not mapped")
            continue
        if not os.path.isdir(cwd):
            out(f"#{cid}: '{project}' maps to {cwd}, which is not a directory")
            problems.append(f"#{cid}: {cwd} is not a directory")
            continue

        out(f"#{cid} {ch.get('title','')}: {len(fresh)} to answer, in {cwd}")
        prompt = (
            "You are answering inside a Trellis channel card — a conversation the "
            "operator reads on their desktop and phone. Reply to the most recent "
            "message. Be brief and concrete; this is a chat, not a report. Do not "
            "greet, do not restate the question, and do not sign your name — the "
            "card already shows who is speaking.\n\n"
            f"The conversation so far (you are '{ME}'):\n\n"
            f"{transcript(data.get('messages') or [])}\n\n"
            "Write only your reply."
        )
        try:
            reply = ask_claude(prompt, cwd, timeout)
        except RuntimeError as e:
            # Reported, never posted: a channel full of error text is worse than a
            # channel that stayed quiet, and the plugin log is where a failure
            # belongs. The cursor is deliberately not advanced, so the message is
            # answered on the next change rather than lost.
            #
            # And it moves on. v1.0.0 exited here, so one project's failure
            # abandoned every channel after it in the list — with a single global
            # directory that was invisible, because there was only ever one thing
            # to fail.
            out(f"#{cid}: {e}")
            problems.append(f"#{cid}: {e}")
            save_state(state)
            continue

        api("POST", f"/cards/{cid}/say", {"text": reply})
        # Re-read rather than assume: the answer's own seq is what must be stored,
        # and the operator may have typed again while the model was thinking.
        seen[key] = api("GET", f"/cards/{cid}/channel?since=0").get("seq", since)
        answered += 1
        save_state(state)

    save_state(state)
    if answered:
        out(f"Answered {answered} message(s)")
    elif looked and not problems:
        out("Nothing new in any channel")
    if problems:
        # Non-zero so the Plugins window shows it failed. Every channel that could
        # be answered already has been by this point.
        die("; ".join(problems))

if __name__ == "__main__":
    main()
