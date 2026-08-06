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
| `TRELLIS_NODE` / `TRELLIS_NODE_TITLE` | set only when launched from a basket's menu |
| `TRELLIS_TRIGGER` | `manual`, `schedule` or `change` |
| `TRELLIS_SINCE` / `TRELLIS_REV` | on-change only: read `GET /api/changes?since=$TRELLIS_SINCE` for exactly what you haven't seen |

Trigger kinds are declared in `plugin.json`: `manual`, `node-menu`, `schedule`
(with `interval_mins`) and `on-change` (with `debounce_secs`). Scheduled and
on-change plugins run only while Trellis is open.

Print progress to stdout; the **last non-empty line becomes the status Trellis
shows**. Exit non-zero to be reported as a failure.
