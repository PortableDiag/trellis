# Workspace currency

Tells you when your workspace has started asserting things nobody has
re-checked.

A card that states how something *is* — "both instances run v0.109.0", "the
keystore is backed up", "the operator still owes a bot token" — was true when it
was written. Nothing else in a document distinguishes a fact from a fact **as of
a date**, so it gets read in the same voice a year later. That is not a
hypothetical: it has cost sessions, and it is why `verify::` exists.

Mark such a card with the date its claim should be re-checked, and optionally how:

```
verify:: 2026-09-01
check:: GET /api/instance
```

The desktop shows the count in **View → Claims**. This plugin is the off-app
half: on a schedule, it reports the ones past their date wherever you are, so a
stale card is caught by a check rather than by an agent repeating it.

Sends to Telegram. **With no bot token it prints the message instead**, which is
how to check the wording first — and it means the plugin works and can be tested
without a Telegram account.

## Install

Copy this folder to `<data-dir>/trellis/plugins/currency/` (the path is at the
top of **Tools → Plugins…**), press **Rescan**, then **Approve**. It asks only to
read the document. Configuration is per instance, so work and personal each get
their own — which is correct, since they are different documents with different
claims.

| Setting | |
|---|---|
| Telegram bot token | From [@BotFather](https://t.me/botfather). The same bot as `notify` is fine. **Leave empty to preview.** |
| Telegram chat id | Message your bot once, then open `https://api.telegram.org/bot<TOKEN>/getUpdates` |
| Only this basket id | Optional — limits the check to one project |
| Stay silent when nothing is stale | Default `yes`. `no` gets you a confirmation instead. |

## Two things it deliberately does not do

**It never runs a `check::`.** That property names the command or endpoint that
would settle a claim — and it lives *inside a card*, which an agent, the web
clipper, or anything else holding an API key can write. Executing it would turn
every card in the document into arbitrary code on this machine. It is reported
for a person or an agent to run. That is the whole of it.

**It never writes.** The token is read-only, so a bug here cannot damage the
document it is watching. Surfacing claims *inside* the app is the app's job
(the panel and the menu count), and needs no plugin at all.

## How it decides what to send

- `GET /api/claims?expired=true` — past their date, plus any whose `verify::`
  is not a readable `YYYY-MM-DD`. Those are called out separately, because that
  card is not "out of date", it is "was never going to expire" — the failure
  that hides longest.
- **An unchanged list is not re-sent.** The set of stale card ids is kept in
  `state.json` beside the plugin; the same set next interval sends nothing. A
  notifier that repeats itself gets muted, and then it may as well not exist.
- Nothing fires while Trellis is closed. It is a desktop app, not a service —
  the same honest limitation the `notify` plugin carries.
