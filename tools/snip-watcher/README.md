# Trellis snip watcher

Tool-agnostic screenshot ingest: watches a folder and turns any image dropped
there into a **Trellis image card**, so your capture tool (Kazam, Print Screen,
Spectacle, Flameshot, a file manager — anything) stays exactly as-is and captures
just land in your notes.

Python stdlib only — no dependencies. Talks to the Trellis agent API, reading the
key/port from Trellis's own settings (same as Meet Scribe).

## Install (off by default)

```sh
./install.sh                              # copies files, registers the unit — does NOT start it
systemctl --user enable --now trellis-snip  # turn it on (now + every login), when you want it
```

`install.sh` only lays down the files; the watcher stays **off** until you enable
it. Once enabled it watches `~/Pictures/screenshots` and imports each new image
into a root node called **Screenshots** (created if missing). Turn it off any time
with `systemctl --user disable --now trellis-snip`.

Point your capture tool at that folder (e.g. Kazam → Preferences → autosave
screenshots to `~/Pictures/screenshots`, or just choose it in the save dialog).

## Configure

Environment variables (set in `~/.config/systemd/user/trellis-snip.service` under
`[Service]` as `Environment=...`, then `systemctl --user restart trellis-snip`):

| Var | Default | Meaning |
|---|---|---|
| `TRELLIS_SNIP_DIR` | `~/Pictures/screenshots` | folder to watch |
| `TRELLIS_SNIP_NODE` | `Screenshots` | target node title |
| `TRELLIS_API_HOST` | `127.0.0.1` | API host |
| `TRELLIS_SNIP_POLL` | `1.5` | poll seconds |

## Notes

- Only images that appear **while it's running** are imported; the existing
  contents of the folder are ignored at startup.
- Requires the Trellis agent API enabled (Tools → Settings) with a key set.
- Logs: `journalctl --user -u trellis-snip -f` or
  `~/.local/share/trellis-snip/snip-watcher.log`.
