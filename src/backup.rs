//! Full-document backups to external destinations on a schedule.
//!
//! This is **backup, not version control** — each run writes a complete,
//! self-contained copy of the document (the same gzip-compressed RON the app
//! saves), optionally encrypted, to one or more destinations:
//!
//! - **Disk** — a local or mounted directory (covers NAS/SMB/NFS shares that are
//!   mounted as a filesystem path).
//! - **Network (SFTP)** — `scp` to `[user@]host:/dir` using your existing SSH keys.
//! - **Cloud (rclone)** — `rclone copyto` to a configured remote (`remote:path`),
//!   which covers S3, Google Drive, Dropbox, Backblaze B2, etc.
//!
//! Encryption is optional and uses `gpg` symmetric AES-256; the passphrase is
//! fed to gpg on a short-lived file (never on argv/env), and the plaintext is
//! streamed through a pipe so it never touches disk unencrypted.
//!
//! [`run`] takes the already-serialized document bytes, so it is `Send` and runs
//! on a worker thread off the UI.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where a backup goes. Each kind interprets `target` differently.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DestKind {
    /// A local/mounted directory path.
    Disk,
    /// `[user@]host:/dir` reached with `scp` (SSH keys).
    Sftp,
    /// An `rclone` remote path `remote:dir` (S3, Drive, Dropbox, …).
    Rclone,
}

impl DestKind {
    pub fn label(self) -> &'static str {
        match self {
            DestKind::Disk => "Disk",
            DestKind::Sftp => "Network (SFTP)",
            DestKind::Rclone => "Cloud (rclone)",
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BackupDest {
    pub kind: DestKind,
    #[serde(default)]
    pub name: String,
    /// Disk: a directory. SFTP: `[user@]host:/dir`. Rclone: `remote:dir`.
    #[serde(default)]
    pub target: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl BackupDest {
    pub fn new(kind: DestKind) -> Self {
        Self { kind, name: String::new(), target: String::new(), enabled: true }
    }
    /// A human label for status lines: the name if set, else the target.
    pub fn display(&self) -> String {
        let base = if self.name.trim().is_empty() { self.target.clone() } else { self.name.clone() };
        format!("{} [{}]", if base.is_empty() { "(unset)".into() } else { base }, self.kind.label())
    }
}

/// Persisted backup settings (stored as JSON in the app config).
#[derive(Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    /// Master switch for *scheduled* backups. "Back up now" works regardless.
    #[serde(default)]
    pub enabled: bool,
    /// Minutes between scheduled backups. 0 = manual only.
    #[serde(default)]
    pub interval_mins: u64,
    /// Keep only the newest N backups per **disk** destination (0 = keep all).
    /// Remote destinations aren't pruned automatically.
    #[serde(default)]
    pub retention: usize,
    #[serde(default)]
    pub encrypt: bool,
    /// gpg symmetric passphrase (stored with the config; keep the config volume
    /// protected). Only used when `encrypt` is on.
    #[serde(default)]
    pub passphrase: String,
    #[serde(default)]
    pub destinations: Vec<BackupDest>,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_mins: 360,
            retention: 14,
            encrypt: false,
            passphrase: String::new(),
            destinations: Vec::new(),
        }
    }
}

impl BackupConfig {
    pub fn parse(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
    fn enabled_dests(&self) -> impl Iterator<Item = &BackupDest> {
        self.destinations.iter().filter(|d| d.enabled)
    }
}

/// Outcome of delivering to one destination.
pub struct DestOutcome {
    pub dest: String,
    pub ok: bool,
    pub detail: String,
}

/// Build the artifact (optionally encrypted) and deliver it to every enabled
/// destination. `artifact` is the already-serialized (gzip RON) document;
/// `stamp` is a filesystem-safe timestamp like `20260730-142530`. Returns one
/// outcome per attempted destination.
pub fn run(artifact: &[u8], stamp: &str, cfg: &BackupConfig) -> Vec<DestOutcome> {
    if cfg.enabled_dests().next().is_none() {
        return vec![DestOutcome {
            dest: "(none)".into(),
            ok: false,
            detail: "no enabled destinations configured".into(),
        }];
    }

    // Optionally encrypt, once, up front.
    let (payload, ext) = if cfg.encrypt {
        match encrypt_gpg(artifact, &cfg.passphrase) {
            Ok(b) => (b, "ron.gz.gpg"),
            Err(e) => {
                return vec![DestOutcome { dest: "encryption".into(), ok: false, detail: e }];
            }
        }
    } else {
        (artifact.to_vec(), "ron.gz")
    };
    let filename = format!("trellis-backup-{stamp}.{ext}");

    // Stage a temp copy for the command-based destinations (sftp/rclone).
    let needs_tmp = cfg.enabled_dests().any(|d| d.kind != DestKind::Disk);
    let staged: Result<PathBuf, String> = if needs_tmp {
        let tmp = std::env::temp_dir().join(&filename);
        std::fs::write(&tmp, &payload).map(|_| tmp).map_err(|e| e.to_string())
    } else {
        Err("not staged".into())
    };

    let mut outs = Vec::new();
    for d in cfg.enabled_dests() {
        let res = match d.kind {
            DestKind::Disk => deliver_disk(&payload, &d.target, &filename, cfg.retention),
            DestKind::Sftp => staged
                .as_ref()
                .map_err(|e| e.clone())
                .and_then(|tmp| run_cmd("scp", &["-q", path_str(tmp)?, sftp_target(&d.target)?.as_str()])),
            DestKind::Rclone => staged.as_ref().map_err(|e| e.clone()).and_then(|tmp| {
                let dest = format!("{}/{}", d.target.trim_end_matches('/'), filename);
                run_cmd("rclone", &["copyto", path_str(tmp)?, &dest])
            }),
        };
        outs.push(DestOutcome {
            dest: d.display(),
            ok: res.is_ok(),
            detail: res.err().unwrap_or_else(|| "ok".into()),
        });
    }

    if let Ok(tmp) = &staged {
        let _ = std::fs::remove_file(tmp);
    }
    outs
}

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str().ok_or_else(|| "non-UTF-8 temp path".to_string())
}

/// scp needs a trailing slash on the remote dir so the file lands *inside* it.
fn sftp_target(target: &str) -> Result<String, String> {
    let t = target.trim();
    if t.is_empty() || !t.contains(':') {
        return Err("SFTP target must look like host:/dir or user@host:/dir".into());
    }
    Ok(if t.ends_with('/') { t.to_string() } else { format!("{t}/") })
}

fn deliver_disk(payload: &[u8], dir: &str, filename: &str, retention: usize) -> Result<(), String> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Err("disk destination has no directory set".into());
    }
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let final_path = dir.join(filename);
    let tmp = dir.join(format!("{filename}.part"));
    std::fs::write(&tmp, payload)
        .and_then(|_| std::fs::rename(&tmp, &final_path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("write {}: {e}", final_path.display())
        })?;
    if retention > 0 {
        prune_disk(&dir, retention);
    }
    Ok(())
}

/// Keep only the newest `keep` `trellis-backup-*` files in `dir`. Names embed a
/// sortable timestamp, so lexical order is chronological.
fn prune_disk(dir: &Path, keep: usize) {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("trellis-backup-") && !n.ends_with(".part"))
            })
            .collect(),
        Err(_) => return,
    };
    files.sort();
    if files.len() > keep {
        for p in &files[..files.len() - keep] {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn run_cmd(bin: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("could not run `{bin}` ({e}) — is it installed and on PATH?"))?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr);
        let msg = msg.trim();
        Err(format!("`{bin}` failed: {}", if msg.is_empty() { "(no error output)" } else { msg }))
    }
}

/// Encrypt `data` with `gpg` symmetric AES-256. The plaintext is streamed
/// through gpg's stdin (never written to disk); the passphrase is passed via a
/// 0600 temp file that is deleted immediately after, keeping it off argv/env and
/// out of the process list.
fn encrypt_gpg(data: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    if passphrase.is_empty() {
        return Err("encryption is on but the passphrase is empty".into());
    }
    let pp = write_secret_temp(passphrase.as_bytes())?;
    let result = (|| {
        let mut child = Command::new("gpg")
            .args([
                "--batch",
                "--yes",
                "--pinentry-mode",
                "loopback",
                "--passphrase-file",
                path_str(&pp)?,
                "--cipher-algo",
                "AES256",
                "--compress-algo",
                "none", // input is already gzip-compressed
                "-c",
                "--output",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run `gpg` ({e}) — install it or turn off encryption"))?;

        // Feed the plaintext on a thread so a large document can't deadlock the
        // pipe while we read the ciphertext back.
        let mut stdin = child.stdin.take().ok_or("gpg stdin unavailable")?;
        let data = data.to_vec();
        let writer = std::thread::spawn(move || stdin.write_all(&data));
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let _ = writer.join();

        if out.status.success() {
            Ok(out.stdout)
        } else {
            let msg = String::from_utf8_lossy(&out.stderr);
            Err(format!("gpg encryption failed: {}", msg.trim()))
        }
    })();
    let _ = std::fs::remove_file(&pp);
    result
}

/// Write `bytes` to a uniquely named temp file created with 0600 permissions
/// (owner-only) on Unix.
fn write_secret_temp(bytes: &[u8]) -> Result<PathBuf, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("trellis-pp-{}-{n}", std::process::id()));

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path).map_err(|e| format!("temp passphrase file: {e}"))?;
    f.write_all(bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// UTC timestamp `YYYYMMDD-HHMMSS` from a `SystemTime`, for backup filenames.
/// Computed without a calendar crate (civil-from-days, after Howard Hinnant).
pub fn stamp(now: std::time::SystemTime) -> String {
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // days since 1970-01-01 -> civil (y, m, d)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_matches_known_epoch() {
        // 2026-07-30 14:25:30 UTC = 1785421530
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_785_421_530);
        assert_eq!(stamp(t), "20260730-142530");
        // epoch itself
        assert_eq!(stamp(std::time::UNIX_EPOCH), "19700101-000000");
    }

    #[test]
    fn config_round_trips_and_defaults() {
        let mut c = BackupConfig::default();
        c.enabled = true;
        c.destinations.push(BackupDest {
            kind: DestKind::Disk,
            name: "usb".into(),
            target: "/mnt/usb/backups".into(),
            enabled: true,
        });
        let back = BackupConfig::parse(&c.to_json());
        assert!(back.enabled);
        assert_eq!(back.destinations.len(), 1);
        assert!(matches!(back.destinations[0].kind, DestKind::Disk));
        // Unknown/empty JSON falls back to defaults, not a panic.
        assert_eq!(BackupConfig::parse("nonsense").interval_mins, BackupConfig::default().interval_mins);
    }

    #[test]
    fn disk_delivery_writes_and_prunes() {
        let dir = std::env::temp_dir().join(format!("trellis-bk-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Three backups, retention 2 -> oldest pruned.
        for stamp in ["20260101-000000", "20260102-000000", "20260103-000000"] {
            let name = format!("trellis-backup-{stamp}.ron.gz");
            deliver_disk(b"data", dir.to_str().unwrap(), &name, 2).unwrap();
        }
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "trellis-backup-20260102-000000.ron.gz".to_string(),
                "trellis-backup-20260103-000000.ron.gz".to_string(),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
