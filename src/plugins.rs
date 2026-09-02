//! Out-of-process plugins: third-party integrations that are not Trellis code.
//!
//! ## Why out-of-process, and why no new data API
//!
//! A plugin is an **executable** that Trellis launches, handed a base URL and a
//! token, and which drives the existing agent HTTP API. Two consequences follow,
//! and both are the point:
//!
//! - **A bad plugin cannot corrupt the document.** It has no pointer into it. An
//!   in-process `cdylib` that segfaults takes the app and the open document with
//!   it; a plugin that segfaults is a non-zero exit code in a log pane.
//! - **No second data surface to keep in sync.** Trellis already maintains its
//!   API across three documented places. A bespoke plugin API would double that
//!   burden forever, to expose capabilities the HTTP API already has.
//!
//! WASM was considered and rejected on the same grounds: a heavy dependency, and
//! it would still need a host API written from scratch.
//!
//! ## Tokens are minted here, never by the user
//!
//! A plugin **declares** what it needs in its manifest. Trellis shows that in
//! plain words, and on approval mints a token scoped to the declaration. The user
//! approves a *scope*, once; they never see, copy, rotate or paste a token.
//! Making people manage a credential per plugin would be worse than the single
//! shared key it replaces — the failure to avoid is `curl | sh`, where something
//! gets full access because narrowing it was inconvenient.

use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What a plugin's token is allowed to do.
///
/// Deliberately small. Every field here has to be enforced somewhere, and a
/// permission that is declared but not checked is worse than one that doesn't
/// exist — it reads as a guarantee.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Scope {
    /// `true` = GET only. Enforced on the API thread, before the request ever
    /// reaches the document.
    #[serde(default)]
    pub read_only: bool,
    /// Confine the plugin to one node and its descendants. `None` = the whole
    /// document. Enforced in the app loop, which is the only place the tree is
    /// available to resolve ancestry.
    #[serde(default)]
    pub subtree: Option<crate::model::NodeId>,
}

impl Scope {
    /// The scope in the words a person needs to decide with. This is the whole
    /// user-facing security surface, so it says what the plugin can *do*, not
    /// which flags are set.
    pub fn describe(&self, doc_title: &str) -> String {
        let what = if self.read_only { "read" } else { "read and change" };
        match self.subtree {
            Some(_) => format!("{what} one basket and everything under it"),
            None => format!("{what} your whole {doc_title} document"),
        }
    }

    /// The same sentence, but naming the basket. Used where the basket is known
    /// — "read and change **SCOUT** and everything under it" is a far better
    /// thing to check a token against than "one basket".
    pub fn describe_named(&self, doc_title: &str, basket: Option<&str>) -> String {
        let what = if self.read_only { "read" } else { "read and change" };
        match (self.subtree, basket) {
            (Some(_), Some(name)) => format!("{what} {name} and everything under it"),
            (Some(_), None) => format!("{what} one basket and everything under it"),
            (None, _) => format!("{what} your whole {doc_title} document"),
        }
    }

    /// Whether a request method is permitted. The only check cheap enough to run
    /// on the API thread, and the one that matters most.
    pub fn allows_method(&self, is_read: bool) -> bool {
        is_read || !self.read_only
    }
}

/// When Trellis runs a plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// Tools → Plugins → *name*. Always available.
    Manual,
    /// Right-click a basket → *name*, invoked with that node's id.
    NodeMenu,
    /// Right-click a card → *name*, invoked with that card's id and its basket's.
    /// A plugin gets the ids, not the card — it reads what it needs over the API
    /// under its own scope, so this trigger widens nothing.
    CardMenu,
    /// Every `interval_mins` while Trellis is open. Nothing fires while it is
    /// closed — this is a desktop app, not a service, and pretending otherwise
    /// would make a schedule people rely on quietly unreliable.
    Schedule,
    /// When the document changes. Reads the change log, so the plugin is told
    /// *what* moved rather than merely that something did — that is the whole
    /// reason the log was built. Debounced, or a burst of typing would launch a
    /// process per keystroke.
    OnChange,
}

impl Trigger {
    pub fn label(&self) -> &'static str {
        match self {
            Trigger::Manual => "Tools → Plugins",
            Trigger::NodeMenu => "right-click a basket",
            Trigger::CardMenu => "right-click a card",
            Trigger::Schedule => "on a schedule",
            Trigger::OnChange => "when the document changes",
        }
    }
}

/// A plugin's `plugin.json`, as authored by whoever wrote the plugin.
///
/// JSON rather than TOML purely to avoid adding a dependency for one small file;
/// `serde_json` is already here.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    /// Stable identifier, also the directory name. Lowercase, no spaces.
    pub name: String,
    /// Shown in menus.
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// The program to run, relative to the plugin's directory (or an absolute
    /// path / a name on PATH).
    pub command: String,
    /// Extra arguments, before the trigger's own.
    #[serde(default)]
    pub args: Vec<String>,
    /// Where this plugin can be invoked from.
    #[serde(default = "default_triggers")]
    pub triggers: Vec<Trigger>,
    /// What it says it needs. The user approves this, not a token.
    #[serde(default)]
    pub scope: Scope,
    /// Minutes between runs for a `schedule` trigger. Floored at 1 — a plugin
    /// asking for zero would spawn continuously.
    #[serde(default = "default_interval")]
    pub interval_mins: u64,
    /// Seconds of quiet before an `on-change` trigger fires. Without a debounce
    /// every keystroke is a process launch.
    #[serde(default = "default_debounce")]
    pub debounce_secs: u64,
    /// Settings this plugin needs from the user, rendered in the Plugins window
    /// and saved to `config.json` beside the plugin.
    ///
    /// Declared rather than assumed because the alternative is telling people to
    /// hand-edit a JSON file in a directory they'd have to be told how to find —
    /// which is not a setting anyone will use. Trellis owns the form; the plugin
    /// still owns the file, so its credentials never enter Trellis's own config.
    #[serde(default)]
    pub config: Vec<ConfigField>,
}

/// One setting a plugin asks the user for.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConfigField {
    /// Key written into `config.json`.
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub help: String,
    /// Render masked and never log it. A credential shown in plain text in a
    /// window someone screenshots is a credential leaked.
    #[serde(default)]
    pub secret: bool,
    /// At least one field in a group of alternatives must be filled — used for
    /// "either of these credentials".
    #[serde(default)]
    pub required: bool,
}

fn default_interval() -> u64 {
    60
}

fn default_debounce() -> u64 {
    10
}

fn default_triggers() -> Vec<Trigger> {
    vec![Trigger::Manual]
}

/// An installed plugin: its manifest and where it lives.
///
/// **Approval is not recorded here.** A plugin is inert until approved —
/// installing one must not be the same act as granting it access — and the
/// authority for that is the [`Grant`] list, read through `is_approved`. This
/// struct used to carry an `enabled` flag as well, always constructed `false`
/// and never read by anything: a field that looked like the gate while the real
/// gate lived elsewhere, which is the kind of duplicate a future reader trusts
/// instead of checking.
#[derive(Clone, Debug)]
pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
}

impl Plugin {
    /// The executable to run.
    ///
    /// The rule is deliberately unambiguous, because the ambiguous version is a
    /// security problem in both directions:
    ///
    /// - **Anything containing a separator** (`./run.py`, `bin/tool`) resolves
    ///   **inside the plugin's own directory**.
    /// - **A bare name** (`python3`, `node`) is looked up on PATH — that is what
    ///   naming an interpreter means.
    ///
    /// An earlier version preferred a local file for a bare name and fell back to
    /// PATH. That let a plugin shipping a file called `python3` shadow the real
    /// interpreter, and — worse in the other direction — silently run *something
    /// else* off PATH when the plugin's own script was missing, instead of
    /// failing. Neither is acceptable in a path that launches a process.
    pub fn program(&self) -> PathBuf {
        let c = Path::new(&self.manifest.command);
        if c.is_absolute() {
            c.to_path_buf()
        } else if self.manifest.command.contains(std::path::MAIN_SEPARATOR) || self.manifest.command.contains('/') {
            self.dir.join(c)
        } else {
            c.to_path_buf()
        }
    }
}

/// Where plugins live for this instance.
///
/// Per **instance**, beside the settings, like templates and backup config — so
/// two documents can have different plugins, and a plugin approved for personal
/// notes is not thereby approved for work.
pub fn plugins_dir(data_dir: Option<&Path>) -> Option<PathBuf> {
    match data_dir {
        Some(d) => Some(d.join("trellis").join("plugins")),
        None => directories::ProjectDirs::from("dev", "Trellis", "Trellis")
            .map(|p| p.data_dir().join("plugins")),
    }
}

/// Read every plugin in `dir`. A malformed manifest is reported and skipped, not
/// fatal: one bad plugin must not stop the others loading.
pub fn scan(dir: &Path) -> (Vec<Plugin>, Vec<String>) {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (found, errors);
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let mf = p.join("plugin.json");
        if !mf.is_file() {
            continue;
        }
        match std::fs::read_to_string(&mf).map_err(|e| e.to_string()).and_then(|s| {
            serde_json::from_str::<Manifest>(&s).map_err(|e| e.to_string())
        }) {
            Ok(manifest) => {
                found.push(Plugin { manifest, dir: p });
            }
            Err(err) => errors.push(format!("{}: {err}", mf.display())),
        }
    }
    found.sort_by(|a, b| a.manifest.title.to_lowercase().cmp(&b.manifest.title.to_lowercase()));
    (found, errors)
}

/// Where plugin *releases* live: the repo's `plugins/` directory, found from the
/// running binary's own path.
///
/// A plugin release does not install itself — plugins run from
/// `<data-dir>/plugins/`, the repo only *ships* them — so a release can be
/// tagged, changelogged and documented while every instance keeps executing the
/// old copy, with no symptom beyond a feature that silently does nothing. That
/// cost a day of link-less notifications once and recurred twice in a week. The
/// fix is to *say so*: compare the installed manifest's version against the
/// repo's and report the gap.
///
/// The launch scripts `exec <repo>/target/release/trellis`, so the repo is two
/// directories above the executable's own. When the binary runs from anywhere
/// else there is honestly no release copy to compare against, and the answer is
/// `None` — the staleness report quietly says nothing rather than guessing.
pub fn source_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // <repo>/target/release/trellis → pop the binary, `release`, `target`.
    let repo = exe.parent()?.parent()?.parent()?;
    let dir = repo.join("plugins");
    dir.is_dir().then_some(dir)
}

/// The release copies beside this binary, as `name → version`, plus where they
/// were found. Empty when the binary is not running out of a repo checkout —
/// then there is nothing to compare against and nothing is reported stale.
pub fn release_versions() -> (Option<PathBuf>, std::collections::BTreeMap<String, String>) {
    let Some(dir) = source_dir() else {
        return (None, Default::default());
    };
    let (plugins, _) = scan(&dir);
    let map = plugins.into_iter().map(|p| (p.manifest.name, p.manifest.version)).collect();
    (Some(dir), map)
}

/// Is `available` a newer version than `installed`?
///
/// Dotted segments compared numerically (`1.10` beats `1.9`, which a string
/// compare gets wrong), non-numeric segments as strings, missing segments as
/// zero. Strictly *newer* — an installed copy ahead of the repo (a build not
/// yet released) is not stale, and equal is equal.
pub fn version_newer(available: &str, installed: &str) -> bool {
    let seg = |s: &str| -> Vec<String> { s.trim().split('.').map(|p| p.trim().to_string()).collect() };
    let (a, b) = (seg(available), seg(installed));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).map(String::as_str).unwrap_or("0"), b.get(i).map(String::as_str).unwrap_or("0"));
        match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(m), Ok(n)) if m != n => return m > n,
            (Ok(_), Ok(_)) => {}
            _ if x != y => return x > y,
            _ => {}
        }
    }
    false
}

/// Copy a plugin's code and manifest from `src` into `dst`, leaving the
/// installed copy's `config.json` and `state.json` alone.
///
/// This is exactly the update that was done by hand three times: the release's
/// files overwrite their installed counterparts, and everything the *instance*
/// owns — credentials, state, logs, caches — stays. Nothing in `dst` is ever
/// deleted, so a file the release dropped lingers harmlessly rather than a
/// file the instance needs vanishing. Subdirectories shipped by the release are
/// copied whole. Returns the paths written, relative to the plugin directory.
pub fn update_from(src: &Path, dst: &Path) -> Result<Vec<String>, String> {
    fn walk(src: &Path, dst: &Path, rel: &Path, out: &mut Vec<String>) -> Result<(), String> {
        let entries = std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
        for e in entries.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            // The instance's own files, never the release's to overwrite.
            if rel.as_os_str().is_empty() && (n == "config.json" || n == "state.json") {
                continue;
            }
            let from = e.path();
            let to = dst.join(&name);
            let rel_path = rel.join(&name);
            if from.is_dir() {
                std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
                walk(&from, &to, &rel_path, out)?;
            } else {
                std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", to.display()))?;
                // Forward slashes on every platform: these are report lines,
                // not paths to reopen, and the report should read the same in a
                // status bar on Windows as in a test on Linux.
                out.push(rel_path.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(())
    }
    let mut written = Vec::new();
    walk(src, dst, Path::new(""), &mut written)?;
    written.sort();
    Ok(written)
}


/// Read a plugin's `config.json`, or an empty map if it has none yet.
pub fn read_config(dir: &Path) -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write a plugin's `config.json` with owner-only permissions.
///
/// These files hold credentials, so the mode is set explicitly rather than left
/// to the process umask — a key readable by every account on the machine is a
/// key leaked, and nothing about writing a settings form suggests otherwise.
pub fn write_config(
    dir: &Path,
    values: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let path = dir.join("config.json");
    let body = serde_json::to_string_pretty(values).map_err(|e| e.to_string())?;
    std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A minted token and the scope it carries. Stored in the app's config, keyed by
/// name, so approval survives a restart and revocation is real.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Grant {
    /// The installed plugin's name, or — for a standalone token — the label the
    /// user gave it. The field keeps its old name so configs written before
    /// standalone tokens existed still load.
    pub plugin: String,
    pub token: String,
    pub scope: Scope,
    /// Minted for something that is **not** an installed plugin: an agent or
    /// service elsewhere on the network, holding the token itself.
    ///
    /// Kept as a flag rather than inferred, so a plugin can never inherit an
    /// agent's grant by being installed under the same name — that would hand a
    /// local executable a credential the user issued to something else.
    #[serde(default)]
    pub standalone: bool,
}

/// Mint a token for an installed plugin.
pub fn mint_token() -> String {
    mint("plug")
}

/// Mint a token for an agent or service that is not a plugin.
///
/// A different prefix because these are the ones a person handles: a token in a
/// config file somewhere on the network should say at a glance what it is, and
/// which of the two lists to revoke it from.
pub fn mint_agent_token() -> String {
    mint("agent")
}

/// Same CSPRNG as the API key — every one of these is a credential to the same
/// API, so they all get the same strength.
fn mint(prefix: &str) -> String {
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    format!("{prefix}_{}", buf.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The result of one plugin run, for the log pane.
#[derive(Clone, Debug)]
pub struct RunResult {
    pub plugin: String,
    pub ok: bool,
    /// Stopped by the user rather than by failing. Kept apart from `ok` because
    /// painting a deliberate cancel as an error trains people to ignore errors.
    pub cancelled: bool,
    pub summary: String,
    pub output: String,
}

/// A line a running plugin printed, as it prints it.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub plugin: String,
    /// Percent complete, when the plugin reported one. `None` means "still
    /// working" with no estimate — which is most plugins, and is fine.
    pub percent: Option<f32>,
    pub message: String,
}

/// Read one line of a plugin's stdout as progress.
///
/// A JSON object carrying `progress` (**percent**, 0–100) and/or `message` is
/// the structured form; anything else is taken as the message verbatim. Both are
/// supported on purpose: the structured form is what a progress bar needs, and
/// the plain form means `echo` from a shell script already works. Percent rather
/// than a 0–1 fraction because `1` would otherwise be ambiguous between "1%" and
/// "finished", and a plugin author would have to guess which.
fn parse_progress(line: &str) -> (Option<f32>, String) {
    let t = line.trim();
    if t.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
            let pct = v.get("progress").and_then(|p| p.as_f64()).map(|p| p.clamp(0.0, 100.0) as f32);
            let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("").trim().to_string();
            if pct.is_some() || !msg.is_empty() {
                return (pct, msg);
            }
        }
    }
    (None, t.to_string())
}

/// Stop a plugin and everything it started.
///
/// On Unix the child leads its own process group (see `run`), so signalling the
/// negated pid reaches every descendant — which is the only version of "cancel"
/// that is true for a plugin that shells out. TERM first so a plugin can finish
/// the write it is in the middle of, then KILL for anything that ignores it.
///
/// Elsewhere this is the child alone: Windows would need a Job object, and
/// claiming more than that in the UI would be a lie.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as i32;
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
        // A short grace period, then insist. Anything longer would leave the
        // window saying "Stopping…" for long enough to look broken.
        for _ in 0..10 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

/// Run a plugin, streaming what it prints and stopping when asked to.
///
/// The plugin is told everything it needs through the environment rather than
/// argv, so a token can't be read out of the process list by anything else on
/// the machine.
///
/// Runs on a worker thread — a plugin may take minutes, and blocking the UI
/// thread on one would freeze the window and stall autosave. Output is read
/// **as it arrives** rather than collected at exit: a sync of hundreds of items
/// that shows nothing until it finishes is indistinguishable from one that has
/// hung, and there would be no way to stop it.
///
/// Setting `cancel` kills the child. That closes its stdout, which ends the read
/// loop below — so cancelling needs no separate wake-up.
pub fn run(
    plugin: &Plugin,
    token: &str,
    base_url: &str,
    ctx: &[(String, String)],
    cancel: &Arc<AtomicBool>,
    on_progress: &dyn Fn(Progress),
) -> RunResult {
    let name = plugin.manifest.name.clone();
    let mut cmd = std::process::Command::new(plugin.program());
    cmd.args(&plugin.manifest.args)
        .current_dir(&plugin.dir)
        .env("TRELLIS_API", base_url)
        .env("TRELLIS_TOKEN", token)
        .env("TRELLIS_PLUGIN_DIR", &plugin.dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in ctx {
        cmd.env(k, v);
    }
    // Give the plugin its own process group so cancelling can take the whole
    // tree. Without this, a plugin whose command is `./run.sh` puts the actual
    // work in a grandchild: killing the shell leaves it running, and — because
    // it inherited the stdout pipe — the read loop below never ends either, so
    // Cancel would appear to do nothing at all.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return RunResult {
                plugin: name,
                ok: false,
                cancelled: false,
                // Naming the program is the difference between a usable error
                // and "it didn't work" — the usual cause is a missing
                // interpreter or a file that isn't executable.
                summary: format!("could not start {}: {e}", plugin.program().display()),
                output: String::new(),
            }
        }
    };

    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();

    // stderr on its own thread. A plugin that writes more to stderr than the
    // pipe buffer holds would block forever waiting for someone to drain it,
    // while we sat reading stdout — a deadlock that only shows up on chatty
    // plugins, i.e. in production.
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = err_pipe {
            use std::io::Read;
            let _ = std::io::BufReader::new(p).read_to_string(&mut s);
        }
        s
    });

    // The killer needs the child while this thread reads its output, so the
    // handle is shared. The lock is only ever held long enough to signal.
    let child = Arc::new(Mutex::new(child));
    let finished = Arc::new(AtomicBool::new(false));
    let killer = {
        let child = Arc::clone(&child);
        let cancel = Arc::clone(cancel);
        let finished = Arc::clone(&finished);
        std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                if cancel.load(Ordering::Relaxed) {
                    if let Ok(mut c) = child.lock() {
                        kill_tree(&mut c);
                    }
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        })
    };

    let mut stdout = String::new();
    let mut summary: Option<String> = None;
    // A read error used to `break` in silence: the loop ended, the partial
    // output was kept, and the run was still judged by the child's exit status —
    // so a plugin whose stdout failed half way through reported SUCCESS with
    // half its output and whatever summary it had managed to print. Nothing
    // anywhere said the stream had been cut.
    //
    // This is very likely what has been failing
    // `progress_arrives_line_by_line_and_the_last_message_is_the_summary` once
    // every couple of weeks under full-suite load — one callback instead of two.
    // It is not reproducible on demand and was recorded as a known flake on
    // 2026-08-20; rather than guess again, the error is now *kept*, so the next
    // occurrence names itself instead of looking like a miscount.
    let mut read_error: Option<String> = None;
    if let Some(p) = out_pipe {
        for line in std::io::BufReader::new(p).lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    read_error = Some(e.to_string());
                    break;
                }
            };
            stdout.push_str(&line);
            stdout.push('\n');
            let (percent, message) = parse_progress(&line);
            // The last non-empty message is the plugin's headline — the same
            // convention as before streaming, so existing plugins are unchanged.
            if !message.is_empty() {
                summary = Some(message.clone());
            }
            if percent.is_some() || !message.is_empty() {
                on_progress(Progress { plugin: name.clone(), percent, message });
            }
        }
    }

    finished.store(true, Ordering::Relaxed);
    let status = child.lock().ok().and_then(|mut c| c.wait().ok());
    let _ = killer.join();
    let stderr = err_thread.join().unwrap_or_default();

    let was_cancelled = cancel.load(Ordering::Relaxed);
    // A truncated stream is not a success, whatever the child's exit code says:
    // the exit code describes the child, and this describes what we managed to
    // read from it.
    let ok = !was_cancelled && read_error.is_none() && status.map(|s| s.success()).unwrap_or(false);
    let summary = if was_cancelled {
        format!("{} cancelled", plugin.manifest.title)
    } else if let Some(e) = &read_error {
        format!("{} — output was cut short: {e}", plugin.manifest.title)
    } else {
        summary.unwrap_or_else(|| match status {
            Some(s) if s.success() => format!("{} finished", plugin.manifest.title),
            Some(s) => format!("{} failed ({s})", plugin.manifest.title),
            None => format!("{} ended unexpectedly", plugin.manifest.title),
        })
    };
    let mut output = stdout;
    if let Some(e) = &read_error {
        output.push_str(&format!("\n--- output truncated: {e} ---\n"));
    }
    if !stderr.trim().is_empty() {
        output.push_str("\n--- stderr ---\n");
        output.push_str(&stderr);
    }
    RunResult { plugin: name, ok, cancelled: was_cancelled, summary, output }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_json(extra: &str) -> String {
        format!(
            r#"{{"name":"dry-sync","title":"Dry Sync","command":"run.py"{extra}}}"#
        )
    }

    #[test]
    fn a_manifest_needs_only_name_title_and_command() {
        let m: Manifest = serde_json::from_str(&manifest_json("")).unwrap();
        assert_eq!(m.name, "dry-sync");
        assert_eq!(m.triggers, vec![Trigger::Manual], "manual unless it says otherwise");
        assert_eq!(m.scope, Scope { read_only: false, subtree: None });
    }

    /// The scope has to read as a sentence, because it is the whole basis on
    /// which someone decides whether to trust a plugin.
    #[test]
    fn scope_describes_itself_in_plain_words() {
        let whole = Scope { read_only: true, subtree: None };
        assert_eq!(whole.describe("Personal.ron"), "read your whole Personal.ron document");
        let sub = Scope { read_only: false, subtree: Some(62) };
        assert_eq!(sub.describe("x"), "read and change one basket and everything under it");
    }

    #[test]
    fn read_only_scope_rejects_writes_and_allows_reads() {
        let ro = Scope { read_only: true, subtree: None };
        assert!(ro.allows_method(true));
        assert!(!ro.allows_method(false), "a read-only plugin must not write");
        let rw = Scope::default();
        assert!(rw.allows_method(false));
    }

    #[test]
    fn tokens_are_unique_and_marked() {
        let a = mint_token();
        let b = mint_token();
        assert_ne!(a, b);
        assert!(a.starts_with("plug_"), "tellable from the master key at a glance");
        assert!(a.len() > 40);
    }

    /// A bad manifest must not stop the good ones loading — one broken plugin
    /// silently disabling the rest would be very hard to diagnose.
    #[test]
    fn scanning_skips_a_broken_manifest_and_reports_it() {
        let root = std::env::temp_dir().join(format!("trellis-plug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (name, body) in [
            ("good", manifest_json(r#","description":"syncs""#).to_string()),
            ("broken", "{not json".to_string()),
        ] {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("plugin.json"), body).unwrap();
        }
        // A directory with no manifest at all is simply not a plugin.
        std::fs::create_dir_all(root.join("not-a-plugin")).unwrap();

        let (found, errors) = scan(&root);
        assert_eq!(found.len(), 1, "the good one still loads");
        assert_eq!(found[0].manifest.name, "dry-sync");
        assert_eq!(errors.len(), 1, "and the broken one is reported");
        assert!(errors[0].contains("broken"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A path resolves inside the plugin; a bare name is an interpreter on
    /// PATH. Pinned because the ambiguous middle ground lets a plugin shadow an
    /// interpreter, or silently run the wrong thing when its own file is absent.
    #[test]
    fn command_resolution_is_unambiguous() {
        let plug = |cmd: &str| {
            let mut m: Manifest = serde_json::from_str(&manifest_json("")).unwrap();
            m.command = cmd.to_string();
            Plugin { manifest: m, dir: PathBuf::from("/plugins/dry") }
        };
        assert_eq!(plug("./run.py").program(), PathBuf::from("/plugins/dry/./run.py"));
        assert_eq!(plug("bin/tool").program(), PathBuf::from("/plugins/dry/bin/tool"));
        assert_eq!(plug("/usr/bin/env").program(), PathBuf::from("/usr/bin/env"));
        // A bare name is PATH — never a file inside the plugin, even if one
        // exists there with that name.
        assert_eq!(plug("python3").program(), PathBuf::from("python3"));
    }

    /// Both progress forms have to work: the structured one drives the bar, and
    /// the plain one is what a shell script's `echo` already produces.
    #[test]
    fn progress_reads_json_or_a_plain_line() {
        assert_eq!(
            parse_progress(r#"{"progress": 42, "message": "page 3 of 7"}"#),
            (Some(42.0), "page 3 of 7".to_string())
        );
        assert_eq!(parse_progress(r#"{"progress": 10}"#), (Some(10.0), String::new()));
        assert_eq!(parse_progress("  syncing 3 items  "), (None, "syncing 3 items".to_string()));
        // Out-of-range percentages are clamped, not believed.
        assert_eq!(parse_progress(r#"{"progress": 900}"#).0, Some(100.0));
        assert_eq!(parse_progress(r#"{"progress": -5}"#).0, Some(0.0));
        // A line that merely starts with a brace is still a message.
        assert_eq!(parse_progress("{not json"), (None, "{not json".to_string()));
        // JSON carrying neither field is not progress — it's output.
        assert_eq!(parse_progress(r#"{"a":1}"#), (None, r#"{"a":1}"#.to_string()));
    }

    fn shell_plugin(dir: &Path, script: &str) -> Plugin {
        std::fs::create_dir_all(dir).unwrap();
        let s = dir.join("run.sh");
        std::fs::write(&s, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut m: Manifest = serde_json::from_str(&manifest_json("")).unwrap();
        m.command = "./run.sh".into();
        m.title = "Shell".into();
        Plugin { manifest: m, dir: dir.to_path_buf() }
    }

    /// Output must arrive while the plugin is still running, not at exit — the
    /// whole point of streaming is that a long run shows something.
    #[cfg(unix)]
    #[test]
    fn progress_arrives_line_by_line_and_the_last_message_is_the_summary() {
        let dir = std::env::temp_dir().join(format!("trellis-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = shell_plugin(
            &dir,
            "#!/bin/sh\necho '{\"progress\": 50, \"message\": \"halfway\"}'\necho done here\n",
        );
        let seen = Mutex::new(Vec::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let r = run(&p, "tok", "http://127.0.0.1:1/api", &[], &cancel, &|pr| {
            seen.lock().unwrap().push(pr);
        });
        let seen = seen.into_inner().unwrap();
        // If this ever fails on the count again, the run itself now says why:
        // a cut-short stream is reported rather than silently yielding one
        // callback instead of two. Assert that first, so the next failure names
        // the cause instead of only the symptom.
        assert!(
            !r.summary.contains("cut short"),
            "stdout was truncated, which is the suspected cause of this test's \
             historic flakiness: {}",
            r.summary
        );
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].percent, Some(50.0));
        assert_eq!(seen[0].message, "halfway");
        assert!(r.ok && !r.cancelled);
        assert_eq!(r.summary, "done here");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cancelling has to actually stop the process, and say so — a cancel that
    /// only hides the run while it keeps writing is worse than no cancel.
    #[cfg(unix)]
    #[test]
    fn cancelling_kills_the_child_and_is_not_reported_as_a_failure() {
        let dir = std::env::temp_dir().join(format!("trellis-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = shell_plugin(&dir, "#!/bin/sh\necho started\nsleep 60\necho never\n");
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            flag.store(true, Ordering::Relaxed);
        });
        let start = std::time::Instant::now();
        let r = run(&p, "tok", "http://127.0.0.1:1/api", &[], &cancel, &|_| {});
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "did not stop promptly");
        assert!(r.cancelled, "reported as cancelled");
        assert!(!r.ok, "and not as success");
        assert_eq!(r.summary, "Shell cancelled");
        assert!(!r.output.contains("never"), "the rest of the script never ran");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plugins_live_beside_the_instances_settings() {
        let d = plugins_dir(Some(Path::new("/data/work"))).unwrap();
        assert_eq!(d, PathBuf::from("/data/work/trellis/plugins"));
    }

    /// Newer means numerically newer per segment — `1.10` beats `1.9`, which is
    /// exactly what a string compare gets wrong — and the relation is strict:
    /// equal is not stale, and an installed copy *ahead* of the repo (built,
    /// not yet released) is not stale either.
    #[test]
    fn version_compare_is_numeric_and_strict() {
        assert!(version_newer("1.1.0", "1.0.0"));
        assert!(version_newer("1.10", "1.9"));
        assert!(version_newer("2", "1.9.9"));
        assert!(!version_newer("1.0.0", "1.0.0"));
        assert!(!version_newer("1.0.0", "1.1.0"), "installed ahead is not stale");
        assert!(!version_newer("1.0", "1.0.0"), "missing segments read as zero");
        // Non-numeric segments fall back to string order rather than panicking.
        assert!(version_newer("1.0.b", "1.0.a"));
    }

    /// The update is the hand-done one: release files overwrite their installed
    /// counterparts, the instance's `config.json` and `state.json` survive, and
    /// nothing installed is deleted.
    #[test]
    fn update_copies_code_and_manifest_but_never_config_or_state() {
        let base = std::env::temp_dir().join(format!("trellis-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (src, dst) = (base.join("repo"), base.join("installed"));
        std::fs::create_dir_all(src.join("lib")).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("plugin.json"), r#"{"version":"1.1.0"}"#).unwrap();
        std::fs::write(src.join("run.py"), "new code").unwrap();
        std::fs::write(src.join("lib").join("util.py"), "helper").unwrap();
        // A release must never ship these, but if one does, the installed
        // copies still win: they are the instance's, not the release's.
        std::fs::write(src.join("config.json"), "{\"leak\":true}").unwrap();
        std::fs::write(dst.join("plugin.json"), r#"{"version":"1.0.0"}"#).unwrap();
        std::fs::write(dst.join("run.py"), "old code").unwrap();
        std::fs::write(dst.join("config.json"), "{\"api_key\":\"kept\"}").unwrap();
        std::fs::write(dst.join("state.json"), "{\"last_run\":1}").unwrap();

        let written = update_from(&src, &dst).unwrap();
        assert_eq!(
            written,
            vec!["lib/util.py".to_string(), "plugin.json".into(), "run.py".into()]
        );
        assert_eq!(std::fs::read_to_string(dst.join("run.py")).unwrap(), "new code");
        assert_eq!(std::fs::read_to_string(dst.join("lib/util.py")).unwrap(), "helper");
        assert!(std::fs::read_to_string(dst.join("plugin.json")).unwrap().contains("1.1.0"));
        assert_eq!(
            std::fs::read_to_string(dst.join("config.json")).unwrap(),
            "{\"api_key\":\"kept\"}"
        );
        assert_eq!(std::fs::read_to_string(dst.join("state.json")).unwrap(), "{\"last_run\":1}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
