# Notifications

Tells you what the workspace needs without opening it:

- **A task digest** — overdue and due-today cards, on a schedule.
- **An agent nudge** — when something changes the document over the API rather
  than in the app, including `key:: value` changes like `status → done`.

Sends to Telegram. **With no bot token it prints the message instead**, which is
how to check the wording before wiring a bot up — and it means the whole plugin
works and can be tested without a Telegram account.

## Install

Copy this folder to `<data-dir>/trellis/plugins/notify/` (the path is at the top
of **Tools → Plugins…**), press **Rescan**, then **Approve**. It asks only to
read the document.

Settings are in the Plugins window — no file editing:

| Setting | |
|---|---|
| Telegram bot token | From [@BotFather](https://t.me/botfather). **Leave empty to preview.** |
| Telegram chat id | Message your bot once, then open `https://api.telegram.org/bot<TOKEN>/getUpdates` |
| Send task digest | `no` to turn it off |
| Notify on agent edits | `no` to turn it off |
| Only this basket id | Optional — limits the digest to one project |

## How it decides what to send

`schedule` sends the digest (every 3 h by default; change `interval_mins` in
`plugin.json`). `on-change` sends the agent nudge, debounced 60 s so a burst of
edits is one message. **Running it by hand shows the digest**, since that's what
someone pressing *Run now* wants to see.

It keeps a small `state.json` so it doesn't repeat itself: an unchanged digest
isn't re-sent on the same day, and agent changes resume from the last change-log
position it handled. A notifier that repeats itself gets muted, and a muted
notifier may as well not exist.

## The limitation, stated plainly

**Nothing fires while Trellis is closed.** It is a desktop app, not a service, so
a digest only arrives if the app happens to be running when the schedule comes
round. Close the laptop for two days and you get nothing for two days.
