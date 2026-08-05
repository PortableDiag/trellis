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
use std::path::{Path, PathBuf};

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
}

impl Trigger {
    pub fn label(&self) -> &'static str {
        match self {
            Trigger::Manual => "Tools → Plugins",
            Trigger::NodeMenu => "right-click a basket",
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
}

fn default_triggers() -> Vec<Trigger> {
    vec![Trigger::Manual]
}

/// An installed plugin: its manifest, where it lives, and whether it's approved.
#[derive(Clone, Debug)]
pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
    /// A plugin is inert until approved — installing one must not be the same
    /// act as granting it access.
    pub enabled: bool,
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
                found.push(Plugin { manifest, dir: p, enabled: false });
            }
            Err(err) => errors.push(format!("{}: {err}", mf.display())),
        }
    }
    found.sort_by(|a, b| a.manifest.title.to_lowercase().cmp(&b.manifest.title.to_lowercase()));
    (found, errors)
}

/// A minted token and the scope it carries. Stored in the app's config, keyed by
/// plugin name, so approval survives a restart and revocation is real.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Grant {
    pub plugin: String,
    pub token: String,
    pub scope: Scope,
}

/// Mint a token. Same CSPRNG as the API key — a plugin token is a credential to
/// the same API, so it gets the same strength.
pub fn mint_token() -> String {
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    format!("plug_{}", buf.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The result of one plugin run, for the log pane.
#[derive(Clone, Debug)]
pub struct RunResult {
    pub plugin: String,
    pub ok: bool,
    pub summary: String,
    pub output: String,
}

/// Run a plugin, capturing what it printed.
///
/// The plugin is told everything it needs through the environment rather than
/// argv, so a token can't be read out of the process list by anything else on
/// the machine.
///
/// Runs on a worker thread — a plugin may take minutes, and blocking the UI
/// thread on one would freeze the window and stall autosave.
pub fn run(
    plugin: &Plugin,
    token: &str,
    base_url: &str,
    ctx: &[(String, String)],
) -> RunResult {
    let mut cmd = std::process::Command::new(plugin.program());
    cmd.args(&plugin.manifest.args)
        .current_dir(&plugin.dir)
        .env("TRELLIS_API", base_url)
        .env("TRELLIS_TOKEN", token)
        .env("TRELLIS_PLUGIN_DIR", &plugin.dir)
        .stdin(std::process::Stdio::null());
    for (k, v) in ctx {
        cmd.env(k, v);
    }

    match cmd.output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            let ok = out.status.success();
            // The last non-empty line of stdout is the plugin's headline: short
            // enough for a status bar, and it keeps the convention trivial to
            // follow from any language.
            let summary = stdout
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| {
                    if ok {
                        format!("{} finished", plugin.manifest.title)
                    } else {
                        format!("{} failed ({})", plugin.manifest.title, out.status)
                    }
                });
            let mut output = stdout;
            if !stderr.trim().is_empty() {
                output.push_str("\n--- stderr ---\n");
                output.push_str(&stderr);
            }
            RunResult { plugin: plugin.manifest.name.clone(), ok, summary, output }
        }
        Err(e) => RunResult {
            plugin: plugin.manifest.name.clone(),
            ok: false,
            // Naming the program is the difference between a usable error and
            // "it didn't work" — the usual cause is a missing interpreter or a
            // file that isn't executable.
            summary: format!("could not start {}: {e}", plugin.program().display()),
            output: String::new(),
        },
    }
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
            Plugin { manifest: m, dir: PathBuf::from("/plugins/dry"), enabled: true }
        };
        assert_eq!(plug("./run.py").program(), PathBuf::from("/plugins/dry/./run.py"));
        assert_eq!(plug("bin/tool").program(), PathBuf::from("/plugins/dry/bin/tool"));
        assert_eq!(plug("/usr/bin/env").program(), PathBuf::from("/usr/bin/env"));
        // A bare name is PATH — never a file inside the plugin, even if one
        // exists there with that name.
        assert_eq!(plug("python3").program(), PathBuf::from("python3"));
    }

    #[test]
    fn plugins_live_beside_the_instances_settings() {
        let d = plugins_dir(Some(Path::new("/data/work"))).unwrap();
        assert_eq!(d, PathBuf::from("/data/work/trellis/plugins"));
    }
}
