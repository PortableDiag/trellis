# Back up to Dry

Copies a Trellis document's baskets and cards into a [Dry](https://dry.ai) space.
One-way, and safe to run as often as you like: every item is keyed by its Trellis
id, so re-running **updates** the same items instead of duplicating them.

## Install

Copy this folder into your instance's plugins directory — the path is shown at
the top of **Tools → Plugins…**:

```
<data-dir>/trellis/plugins/dry-backup/
```

Then open **Tools → Plugins…** and fill in the settings under *Back up to Dry*:

- **Dry MCP token** — recommended, sent as a Bearer header.
- **Dry access key** — the alternative, from your Dry profile. **Regenerating it
  invalidates the old one**, so if the plugin starts failing with "No user exists
  with that access key", that's why. The MCP token doesn't have this problem.
- **Dry space name** — optional; defaults to `Trellis backup — <document>.ron`.

Either credential works; fill in one. **Save settings** writes them to
`config.json` beside this script at mode `600`. That file is this plugin's own
secret, not Trellis's — Trellis renders the form but the values live with the
plugin, and it's git-ignored.

Finally, open **Tools → Plugins…** and **Approve** it. The plugin asks only to
*read* your document; Trellis enforces that, so it cannot change your notes even
if it tried.

## Running it

- **Tools → Plugins… → Run now** — backs up the whole document.
- **Right-click a basket → Plugins → Back up to Dry** — backs up just that basket
  and everything under it.
- **Right-click a card → Plugins → Back up to Dry** — backs that one card up and
  then **publishes it**, printing a public link as the run's last line (so it
  becomes the status Trellis shows).

  The card is re-backed-up first, so what gets published is current rather than
  whatever a previous run left behind. Publishing writes only to Dry — this
  plugin stays read-only against your document.

### The link is checked anonymously before you are given it

Two different things have to be true for a share link to work, and for a while
only the first one was:

1. **Dry marks the item public.** `op: "publish"` sets `isPublicObject` and
   returns a URL. The plugin reads that flag back from Dry's own state rather
   than trusting the echo.
2. **The viewer actually serves that URL to a stranger.** The plugin now fetches
   the returned URL **with no credentials at all** and follows the redirect
   chain. If it lands on a sign-in page, you get an error naming exactly that,
   and the URL is *not* presented as usable.

Step 2 exists because step 1 passed while the link was dead — for a period the
flag was set and an anonymous request still ended at `/signIn?rU=…`. A share link
that only works for the person who made it is worse than an error, because you
send it to someone before you find out.

A redirect on its own is not treated as failure: Dry's viewer canonicalises
`/v?t=tsr&oc=$…` to another `/v?…` even for a nonsense id, so what is judged is
where the chain **lands**. A 2xx means the viewer served us something rather than
turning us away — a necessary condition, not proof the card's text is on the
page. Open the link in a private window once to confirm that.

If the check cannot run at all (no network), the plugin says so rather than
passing by default: *unverified* and *verified good* must not look the same.

**As of this writing the fix is patched on Dry's side but not yet deployed.**
Until it is, publishing a card reports the sign-in bounce and exits non-zero. No
change here is needed when it deploys — just re-run the plugin on the card and
the same check will pass.

## What lands in Dry

| Trellis | Dry |
|---|---|
| the document | a space |
| each basket | a `TrellisBasket` item (title, full path, card count) |
| each card | a `TrellisCard` item (title, body, kind, basket path, tags, `due::`, `status::`) |

`TrellisId` is marked as Dry's `uniqueKey`, which is what makes re-running an
upsert rather than an import.

## Deliberate limits

- **One-way.** Trellis is the source of truth; nothing is read back.
- **Nothing is ever deleted from Dry.** A card you delete in Trellis stays in the
  Dry space. A backup tool that removes things it didn't see this run is how
  backups lose work — decide those by hand.
- **Text only.** Images and sketches aren't copied; their cards appear with the
  rest of their metadata.
- `Basket` and `Tags` are plain text on purpose. Dry's `reference` fields
  **auto-create** a new item for any value they can't match, so a single odd
  value would litter the space.

## If it fails

The run log is in **Tools → Plugins…**, under the run's *output*.

- **"blocked before reaching the API"** — Dry's edge rejects unrecognised HTTP
  clients before the request arrives. The script sends a browser-like
  `User-Agent` for exactly this reason; don't remove it.
- **Dry reported failures importing…** — a `200` from Dry does *not* mean every
  item landed; the importer reports per-item outcomes inside the response. The
  plugin checks that and fails loudly rather than claiming success.

## Writing your own plugin

This is a worked example of the plugin contract. Trellis passes everything in the
environment (never argv, so tokens can't be read from the process list):

| Variable | Meaning |
|---|---|
| `TRELLIS_API` | base URL, e.g. `http://127.0.0.1:7373/api` |
| `TRELLIS_TOKEN` | your scoped token — send as `X-API-Key` |
| `TRELLIS_PLUGIN_DIR` | this folder, for your own config |
| `TRELLIS_NODE` / `TRELLIS_NODE_TITLE` | set when launched from a basket's or a card's menu |
| `TRELLIS_CARD` / `TRELLIS_CARD_TITLE` | card-menu only: which card you were invoked on |
| `TRELLIS_TRIGGER` | `manual`, `card-menu`, `schedule` or `change` |
| `TRELLIS_SINCE` / `TRELLIS_REV` | on-change only: read `GET /api/changes?since=$TRELLIS_SINCE` for exactly what you haven't seen |

Trigger kinds are declared in `plugin.json`: `manual`, `node-menu`, `card-menu`,
`schedule` (with `interval_mins`) and `on-change` (with `debounce_secs`).
Scheduled and on-change plugins run only while Trellis is open.

A card-menu plugin is handed the card's **id**, never its contents — read what
you need over the API, under the scope you were approved for.

### Progress, and being cancelled

Print to stdout as you go: Trellis reads it **line by line while you run**, and
the last non-empty line is the status it shows.

A line that is a JSON object with `progress` (a **percentage**, 0–100) and/or
`message` drives the progress bar in the Plugins window:

```
{"progress": 40, "message": "page 4 of 10"}
```

Anything else is taken as the message verbatim, so a plain `echo` works. Report a
percentage if you know one — a plugin with no percentage shows a spinner, which
cannot tell "working" from "stuck".

**Cancel kills your process group**, so anything you shelled out to dies with
you. Nothing is rolled back: whatever you already wrote to the document stays.
If a partial run would leave a mess, write in an order where stopping early is
still consistent.

Exit non-zero to be reported as a failure.
