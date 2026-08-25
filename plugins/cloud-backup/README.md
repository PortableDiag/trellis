# cloud-backup — off-site backup to CloudAPI

Copies the newest local backup archive to a [CloudAPI](https://github.com/PortableDiag)
gateway — an object-storage gateway in front of Cloudflare R2 that mints
short-lived, prefix-scoped S3 credentials — then **proves the copy** by
downloading it back and comparing SHA-256, and keeps the cloud copy bounded.

It uploads what the app's own backup module already wrote, so with **Encrypt
backups** on, the archive is gpg ciphertext before it leaves the machine. The
gateway app should therefore be onboarded `encrypted: false` — the documented
"already sealed" case: envelope-encrypting again would put the key inside the
thing being backed up.

## Why a plugin

The credential is personal. A plugin's config lives in
`<data-dir>/plugins/cloud-backup/config.json`, per instance, outside the
document and outside this repo — so the integration ships with the app while
the *access* exists only where a gateway URL and app key have been configured.
Unconfigured, the plugin prints one line and does nothing.

## Setup

1. Onboard an app on your gateway (master key, once):
   `POST /v1/apps {"name":"trellis","verbs":"read,write,delete","encrypted":false,`
   `"quota_bytes":…,"quota_ops_day":…}` — pass the quotas; there is no default.
2. Copy this folder into `<data-dir>/plugins/cloud-backup/`, along with a
   `cloudapi-cli` binary (or set the `cli` config key to its path).
3. Configure: gateway URL, the `sk_trellis_…` key, and the **local backup
   folder** this document's scheduled backups write to. Optional: `keep`
   (default 14, matching the local retention).
4. Tools → Plugins → Rescan → **Approve**. It runs on the schedule (default
   every 6 h) and from the Plugins window's Run.

## What a run does

1. Newest `trellis-backup-*` in the backup folder (never a `.part`).
2. Skips if already uploaded (`state.json`).
3. Uploads as `backups/<document>/<filename>` — the document name namespaces
   the path because two instances write identically-named archives.
4. Downloads it back and compares hashes. A failed compare is a failed run.
5. Lists `backups/<document>/` cursor-complete and purges archives beyond
   `keep`, oldest first, each by explicit name — never a sweep.

The status line reads e.g.:
`Off-site backup ✓ trellis-backup-….gpg → backups/Personal.ron/… — restore verified byte-for-byte (14 kept, purged 1 old)`

## Restore

```
export CLOUDAPI_URL=https://your-gateway   CLOUDAPI_KEY=sk_trellis_…
cloudapi-cli ls backups/
cloudapi-cli get backups/<document>/<archive> ./archive.ron.gz.gpg
gpg -d archive.ron.gz.gpg | gunzip > Restored.ron
```

The gpg passphrase is the backup passphrase in the app's Backup settings —
which lives in your password manager, not in the cloud, which is the point.
