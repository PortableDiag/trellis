# Notifications

Tells you what the workspace needs without opening it:

- **A task digest** — overdue and due-today cards, on a schedule.
- **An agent nudge** — when something changes the document over the API rather
  than in the app, including `key:: value` changes like `status → done`.
- **A channel message, quoted** — when an agent posts to a
  [channel card](../../API.md), the notification carries **what it said** and who
  said it, not just that something moved. A change-log entry holds no content, so
  the text is fetched from `GET /api/cards/{cid}/channel?since=…` against a cursor
  kept per card. Your own messages are skipped: you typed them a moment ago.
  A card seen for the first time reports only its newest message rather than
  replaying its history.

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

A digest longer than Telegram's 4096-character cap is **split into numbered
parts**, on line boundaries so each part is still valid HTML on its own. Before
v1.3.0 the Bot API refused the whole thing with *"Bad Request: message is too
long"* and the notification was simply lost — a digest grows with the document,
so this was only ever a matter of time.

## The limitation, stated plainly

**Nothing fires while Trellis is closed.** It is a desktop app, not a service, so
a digest only arrives if the app happens to be running when the schedule comes
round. Close the laptop for two days and you get nothing for two days.
