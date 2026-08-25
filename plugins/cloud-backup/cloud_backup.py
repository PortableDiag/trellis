#!/usr/bin/env python3
"""Copy the newest local Trellis backup archive to a CloudAPI gateway, prove it,
and keep the cloud copy bounded.

The pipeline, per run:

  1. Find the newest ``trellis-backup-*`` file in ``backup_dir`` (skipping the
     ``.part`` a write-in-progress leaves). These are what the app's own backup
     module writes — gpg-encrypted before they touch disk, so the cloud prefix
     only ever holds ciphertext and the gateway app is ``encrypted: false``
     (its documented "already sealed" case: envelope-encrypting again would put
     the KEK inside the thing being backed up).
  2. Skip if that exact file was already uploaded (state.json remembers).
  3. Upload as ``backups/<document>/<filename>`` via cloudapi-cli, which mints
     the short-lived session, streams the bytes straight to R2 (multipart when
     large — R2's equal-part-length rule lives there, not here) and records the
     metadata. The document name comes from GET /api/instance, because both of
     the operator's instances write archives with identical name patterns.
  4. **Prove the restore**: download the object back to a temp file and compare
     SHA-256 against the local archive. An upload nobody has read back is a
     belief, not a backup. The temp copy is removed either way.
  5. Retention: list ``backups/<document>/`` (following the pagination cursor —
     a listing that stops at one page is a file browser quietly lying), and
     purge archives beyond ``keep``, oldest first, each by its explicit name.
     Never a sweep: only names this run listed under this document's own path.

Trellis supplies TRELLIS_API / TRELLIS_TOKEN / TRELLIS_PLUGIN_DIR in the
environment. The CloudAPI credential is this plugin's secret, not Trellis's, so
it lives in config.json beside this script and is passed to the CLI through the
subprocess environment — never argv, where the process list would show it.

Stdout is the run log; the last non-empty line becomes the status Trellis shows.
Exit non-zero on failure.
"""

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

ARCHIVE = re.compile(r"^trellis-backup-\d{8}-\d{6}\..*(?<!\.part)$")


def die(msg):
    print(msg, file=sys.stderr)
    print(f"Cloud backup failed: {msg}")
    sys.exit(1)


def plugin_dir():
    d = os.environ.get("TRELLIS_PLUGIN_DIR")
    if not d:
        die("not launched by Trellis (TRELLIS_PLUGIN_DIR unset)")
    return d


def load_json(path, default):
    try:
        with open(path) as f:
            return json.load(f)
    except FileNotFoundError:
        return default
    except Exception as e:  # noqa: BLE001
        die(f"unreadable {os.path.basename(path)}: {e}")


def trellis_document():
    """The serving document's file name, to namespace the cloud path."""
    base = os.environ.get("TRELLIS_API")
    token = os.environ.get("TRELLIS_TOKEN")
    if not base or not token:
        die("not launched by Trellis (TRELLIS_API / TRELLIS_TOKEN unset)")
    req = urllib.request.Request(f"{base}/instance", headers={"X-API-Key": token})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.loads(r.read().decode())["document"]
    except Exception as e:  # noqa: BLE001
        die(f"GET /api/instance: {e}")


def gateway(cfg, method, path, key):
    """One JSON call to the CloudAPI gateway itself (list / purge)."""
    req = urllib.request.Request(
        cfg["gateway"].rstrip("/") + path,
        method=method,
        headers={"X-API-Key": key},
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        body = e.read().decode(errors="replace")[:200]
        die(f"gateway {method} {path} → HTTP {e.code}: {body}")
    except Exception as e:  # noqa: BLE001
        die(f"gateway {method} {path}: {e}")


def run_cli(cfg, key, args):
    """Run cloudapi-cli with the credential in the environment, never argv."""
    cli = cfg.get("cli") or os.path.join(plugin_dir(), "cloudapi-cli")
    env = dict(os.environ)
    env["CLOUDAPI_URL"] = cfg["gateway"]
    env["CLOUDAPI_KEY"] = key
    # No CLOUDAPI_KEK_FILE on purpose: the archive is already gpg ciphertext
    # and the prefix policy is encrypted:false. See the module doc.
    env.pop("CLOUDAPI_KEK_FILE", None)
    p = subprocess.run(
        [cli, *args], env=env, capture_output=True, text=True, timeout=1800
    )
    if p.returncode != 0:
        die(f"cloudapi-cli {args[0]}: {(p.stderr or p.stdout).strip()[:300]}")
    return p.stdout


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def newest_archive(backup_dir):
    try:
        names = [n for n in os.listdir(backup_dir) if ARCHIVE.match(n)]
    except OSError as e:
        die(f"backup folder {backup_dir}: {e}")
    if not names:
        die(f"no trellis-backup-* archive in {backup_dir} yet")
    # The stamp in the name sorts chronologically; mtime would trust the disk.
    return max(names)


def main():
    pdir = plugin_dir()
    cfg = load_json(os.path.join(pdir, "config.json"), {})
    for k in ("gateway", "app_key", "backup_dir"):
        if not cfg.get(k):
            # Not an error: an unconfigured instance deliberately does nothing.
            print(f"cloud-backup: not configured ({k} unset) — nothing sent")
            return
    keep = int(cfg.get("keep") or 14)
    key = cfg["app_key"]

    doc = trellis_document()
    fname = newest_archive(cfg["backup_dir"])
    local = os.path.join(cfg["backup_dir"], fname)
    logical = f"backups/{doc}/{fname}"

    state_path = os.path.join(pdir, "state.json")
    state = load_json(state_path, {})
    if state.get(doc) == fname:
        print(f"{fname} already in the cloud — nothing new to send")
        return

    size_mb = os.path.getsize(local) / (1024 * 1024)
    print(f"uploading {fname} ({size_mb:.1f} MB) as {logical}")
    run_cli(cfg, key, ["put", local, logical, f"trellis,backup,{doc}"])

    # Prove the restore: bytes back, hashes equal. Every run, because the cost
    # is one read of a file this size and the alternative is trust.
    with tempfile.NamedTemporaryFile(prefix="cloud-backup-verify-", delete=False) as tf:
        tmp = tf.name
    try:
        run_cli(cfg, key, ["get", logical, tmp])
        up, down = sha256_file(local), sha256_file(tmp)
        if up != down:
            die(f"restore verification FAILED for {logical}: hashes differ")
    finally:
        try:
            os.unlink(tmp)
        except OSError:
            pass

    state[doc] = fname
    with open(state_path, "w") as f:
        json.dump(state, f)

    # Retention. List this document's own path, cursor-complete, then purge
    # beyond `keep` — oldest first, each by the explicit key we just listed.
    keys = []
    cursor = ""
    while True:
        q = urllib.parse.urlencode(
            {"prefix": f"trellis/backups/{doc}/", "cursor": cursor, "limit": 500}
        )
        page = gateway(cfg, "GET", f"/v1/objects?{q}", key)
        keys += [o["key"] for o in page.get("objects", [])]
        if not page.get("truncated"):
            break
        cursor = page.get("next_cursor") or ""
        if not cursor:
            break
    archives = sorted(k for k in keys if ARCHIVE.match(k.rsplit("/", 1)[-1]))
    purged = 0
    for k in archives[:-keep] if keep else []:
        q = urllib.parse.quote(k, safe="/")
        gateway(cfg, "DELETE", f"/v1/objects/{q}?purge=true", key)
        purged += 1
    kept = len(archives) - purged
    want = f", purged {purged} old" if purged else ""
    print(
        f"Off-site backup ✓ {fname} → {logical} — restore verified byte-for-byte"
        f" ({kept} kept{want})"
    )


if __name__ == "__main__":
    main()
