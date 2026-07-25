#!/usr/bin/env bash
# Install the Trellis snip watcher's files as a systemd --user service.
# OFF BY DEFAULT: this copies the files and registers the unit, but does NOT
# start or enable it. You turn it on explicitly (see the printed commands).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
dest="$HOME/.local/share/trellis-snip"
unit_dir="$HOME/.config/systemd/user"

mkdir -p "$dest" "$unit_dir"
cp "$here/snip-watcher.py" "$dest/snip-watcher.py"
cp "$here/trellis-snip.service" "$unit_dir/trellis-snip.service"
systemctl --user daemon-reload

echo "Installed (not started). The watcher is OFF until you enable it."
echo
echo "Turn it on now + on every login:   systemctl --user enable --now trellis-snip"
echo "Try it once (this session only):   systemctl --user start trellis-snip"
echo "Turn it off:                       systemctl --user disable --now trellis-snip"
echo "Logs:                              journalctl --user -u trellis-snip -f"
echo
echo "Watches ~/Pictures/screenshots by default (override with TRELLIS_SNIP_DIR in the unit)."
