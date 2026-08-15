//! The external command-line tools Trellis can use, and how to get them.
//!
//! Every one of these is **optional** — the app runs without any of them, and a
//! missing tool only disables the one feature that shells out to it. But "install
//! tesseract-ocr" is not help: it names the thing without saying where to get it,
//! and the answer is different on every platform. So each dependency is declared
//! once, here, with what it enables and a per-OS way to get it — a real installer
//! command where the platform has a package manager we can drive (winget, brew),
//! and a copyable command plus a download link everywhere else.
//!
//! Nothing here elevates privileges. On Linux the install is `sudo`-shaped and a
//! GUI app has no business prompting for a root password, so that path shows the
//! command rather than running it.

use std::path::PathBuf;

/// Which package manager, if any, we can drive on this machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Manager {
    /// Windows: ships with Windows 10 21H1+ / 11. Runs in its own console.
    Winget,
    /// macOS: only when the user already has Homebrew.
    Brew,
    /// Linux family — we can *write* the command but not run it (needs sudo).
    Apt,
    Dnf,
    Pacman,
    Zypper,
    /// Nothing we recognise: fall back to the download page.
    None,
}

/// How the user can get a missing tool.
#[derive(Clone, Debug)]
pub enum Install {
    /// We can run this ourselves, in a visible console the user can watch.
    Run { label: String, bin: String, args: Vec<String> },
    /// The install needs a root password, so hand over the exact command.
    Copy { label: String, cmd: String },
    /// No package we can name — send them to the download page.
    Link,
}

/// One external tool, what it buys, and where it comes from.
pub struct Dep {
    /// Stable id, used by the feature that needs it.
    pub id: &'static str,
    /// Human name, e.g. "Tesseract OCR".
    pub label: &'static str,
    /// Any one of these on PATH satisfies the dependency. Several because some
    /// jobs have interchangeable tools (xclip *or* xsel).
    pub bins: &'static [&'static str],
    /// The Trellis feature this enables, in the user's terms.
    pub enables: &'static str,
    /// Where a human goes to read about it / download it.
    pub url: &'static str,
    /// Package name per manager, for building the install command.
    pub pkg: Pkg,
    /// True when the platform provides this out of the box, so "missing" is
    /// unusual rather than expected.
    pub builtin_here: bool,
}

/// The package name for this tool under each manager we know about. They
/// genuinely differ (`gnupg` vs `gnupg2` vs `gpg2`), which is exactly why
/// telling the user "install gpg" doesn't work.
#[derive(Default)]
pub struct Pkg {
    pub winget: &'static str,
    pub brew: &'static str,
    pub apt: &'static str,
    pub dnf: &'static str,
    pub pacman: &'static str,
    pub zypper: &'static str,
}

impl Dep {
    /// The first of this dep's binaries found on PATH.
    pub fn found(&self) -> Option<PathBuf> {
        self.bins.iter().find_map(|b| which(b))
    }

    pub fn present(&self) -> bool {
        self.found().is_some()
    }

    /// How to install this tool on this machine right now.
    pub fn install(&self, mgr: Manager) -> Install {
        let name = |s: &'static str| (!s.is_empty()).then(|| s.to_string());
        match mgr {
            Manager::Winget => match name(self.pkg.winget) {
                // `-e` matches the id exactly; `--source winget` avoids the
                // Microsoft Store prompt. Accepting the agreements up front
                // keeps it from stalling on an invisible y/n.
                Some(id) => Install::Run {
                    label: format!("Install with winget ({id})"),
                    bin: "winget".into(),
                    args: vec![
                        "install".into(),
                        "--id".into(),
                        id,
                        "-e".into(),
                        "--source".into(),
                        "winget".into(),
                        "--accept-package-agreements".into(),
                        "--accept-source-agreements".into(),
                    ],
                },
                None => Install::Link,
            },
            Manager::Brew => match name(self.pkg.brew) {
                Some(f) => Install::Run {
                    label: format!("Install with Homebrew ({f})"),
                    bin: "brew".into(),
                    args: vec!["install".into(), f],
                },
                None => Install::Link,
            },
            Manager::Apt => self.copy_cmd("sudo apt install -y", self.pkg.apt),
            Manager::Dnf => self.copy_cmd("sudo dnf install -y", self.pkg.dnf),
            Manager::Pacman => self.copy_cmd("sudo pacman -S --needed", self.pkg.pacman),
            Manager::Zypper => self.copy_cmd("sudo zypper install -y", self.pkg.zypper),
            Manager::None => Install::Link,
        }
    }

    fn copy_cmd(&self, prefix: &str, pkg: &'static str) -> Install {
        if pkg.is_empty() {
            return Install::Link;
        }
        Install::Copy {
            label: "Run this in a terminal".into(),
            cmd: format!("{prefix} {pkg}"),
        }
    }
}

/// Every optional tool that matters **on this platform**.
///
/// Platform-specific entries are filtered out rather than listed as "not
/// applicable": a Windows user has no use for a line about xclip, and a
/// requirements list that is mostly noise stops being read.
pub fn all() -> Vec<Dep> {
    let mut v = vec![
        Dep {
            id: "tesseract",
            label: "Tesseract OCR",
            bins: &["tesseract"],
            enables: "Extract text from image cards, so screenshots and scans \
                      turn up in search (right-click an image card → Extract \
                      text, and Tools → OCR all images).",
            url: "https://tesseract-ocr.github.io/tessdoc/Installation.html",
            pkg: Pkg {
                winget: "UB-Mannheim.TesseractOCR",
                brew: "tesseract",
                apt: "tesseract-ocr",
                dnf: "tesseract",
                pacman: "tesseract tesseract-data-eng",
                zypper: "tesseract-ocr",
            },
            builtin_here: false,
        },
        Dep {
            id: "gpg",
            label: "GnuPG",
            bins: &["gpg"],
            enables: "Encrypt backups with a passphrase (AES-256). Only needed \
                      if you tick Encrypt in Tools → Backup.",
            url: "https://gnupg.org/download/",
            pkg: Pkg {
                winget: "GnuPG.Gpg4win",
                brew: "gnupg",
                apt: "gnupg",
                dnf: "gnupg2",
                pacman: "gnupg",
                zypper: "gpg2",
            },
            builtin_here: false,
        },
        Dep {
            id: "rclone",
            label: "rclone",
            bins: &["rclone"],
            enables: "Back up to cloud storage — S3, Google Drive, Dropbox, B2 \
                      and others. Configure a remote with `rclone config` first.",
            url: "https://rclone.org/downloads/",
            pkg: Pkg {
                winget: "Rclone.Rclone",
                brew: "rclone",
                apt: "rclone",
                dnf: "rclone",
                pacman: "rclone",
                zypper: "rclone",
            },
            builtin_here: false,
        },
        Dep {
            id: "scp",
            label: "OpenSSH client (scp)",
            bins: &["scp"],
            enables: "Back up to another machine over SFTP.",
            url: "https://www.openssh.com/",
            pkg: Pkg {
                // Windows ships this as an optional feature rather than a
                // winget package, so the link goes to Microsoft's instructions.
                winget: "",
                brew: "openssh",
                apt: "openssh-client",
                dnf: "openssh-clients",
                pacman: "openssh",
                zypper: "openssh-clients",
            },
            builtin_here: true,
        },
    ];
    if cfg!(target_os = "linux") {
        v.push(Dep {
            id: "notify",
            label: "notify-send",
            bins: &["notify-send"],
            enables: "Desktop notifications: what is due when the document opens, \
                      and a nudge when an agent changes something while you are in \
                      another window (Settings → Canvas → Notifications).",
            url: "https://gitlab.gnome.org/GNOME/libnotify",
            pkg: Pkg {
                winget: "",
                brew: "",
                apt: "libnotify-bin",
                dnf: "libnotify",
                pacman: "libnotify",
                zypper: "libnotify-tools",
            },
            builtin_here: false,
        });
        v.push(Dep {
            id: "clipboard",
            label: "xclip or xsel",
            bins: &["xclip", "xsel"],
            enables: "Middle-click paste and the X11 primary selection — \
                      selecting text in a card offers it to other apps.",
            url: "https://github.com/astrand/xclip",
            pkg: Pkg {
                winget: "",
                brew: "",
                apt: "xclip",
                dnf: "xclip",
                pacman: "xclip",
                zypper: "xclip",
            },
            builtin_here: false,
        });
        v.push(Dep {
            id: "snip",
            label: "A region screenshot tool",
            bins: &["spectacle", "gnome-screenshot", "maim", "scrot", "import"],
            enables: "Tools → Snip to card: drag a screen region straight into \
                      an image card. Any one of spectacle, gnome-screenshot, \
                      maim, scrot or ImageMagick's import will do.",
            url: "https://github.com/naelstrof/maim",
            pkg: Pkg {
                winget: "",
                brew: "",
                apt: "maim",
                dnf: "maim",
                pacman: "maim",
                zypper: "maim",
            },
            builtin_here: false,
        });
    }
    v
}

/// Look one dependency up by id.
pub fn get(id: &str) -> Option<Dep> {
    all().into_iter().find(|d| d.id == id)
}

/// The package manager we can actually drive here.
///
/// Probed rather than assumed from the OS: winget is absent on older Windows,
/// and plenty of macs have no Homebrew. Whichever we return, its binary is on
/// PATH — so the button we offer will not fail with "command not found".
pub fn manager() -> Manager {
    if cfg!(target_os = "windows") {
        return if which("winget").is_some() { Manager::Winget } else { Manager::None };
    }
    if cfg!(target_os = "macos") {
        return if which("brew").is_some() { Manager::Brew } else { Manager::None };
    }
    for (bin, m) in [
        ("apt", Manager::Apt),
        ("dnf", Manager::Dnf),
        ("pacman", Manager::Pacman),
        ("zypper", Manager::Zypper),
    ] {
        if which(bin).is_some() {
            return m;
        }
    }
    Manager::None
}

/// Find `bin` on PATH, the way the OS would when we spawn it.
///
/// On Windows a bare name is not enough: `winget` is `winget.exe`, and some
/// tools arrive as `.cmd` shims, so every extension in PATHEXT is tried.
pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(target_os = "windows") {
        let pathext = std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        // The bare name first, so an extensionless shim still resolves.
        std::iter::once(String::new())
            .chain(pathext.split(';').map(|e| e.to_ascii_lowercase()))
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let cand = dir.join(format!("{bin}{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Run an install command in a console the user can watch.
///
/// Deliberately fire-and-forget: an install can take minutes and prompt for
/// elevation, and blocking the UI thread on it would look like a freeze. The
/// requirements window re-probes PATH when it's reopened, so the result shows up
/// on its own.
pub fn run_install(bin: &str, args: &[String]) -> Result<(), String> {
    let spawn = |b: &str, a: Vec<String>| {
        std::process::Command::new(b)
            .args(a)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not start `{b}`: {e}"))
    };
    if cfg!(target_os = "windows") {
        // `start` gives it its own console window, so winget's progress and any
        // UAC prompt are visible instead of happening invisibly behind the app.
        // The empty "" is `start`'s title argument — without it, a quoted path
        // would be taken as the title and nothing would run.
        let mut a = vec!["/C".to_string(), "start".to_string(), String::new(), bin.to_string()];
        a.extend(args.iter().cloned());
        return spawn("cmd", a);
    }
    if cfg!(target_os = "macos") {
        return spawn(bin, args.to_vec());
    }
    spawn(bin, args.to_vec())
}

/// Open a URL in the user's browser.
///
/// Trellis builds eframe without its link feature (it pulls a browser-opening
/// dependency in for the one thing we do here), so this is our own shell-out.
pub fn open_url(url: &str) -> Result<(), String> {
    let (bin, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else {
        ("xdg-open", vec![url])
    };
    std::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {url} ({e})"))
}

/// The message shown when a feature can't run because its tool is missing.
/// Always names the tool *and* points at the one place that installs it.
pub fn missing_msg(dep: &Dep) -> String {
    format!("{} isn't installed — Tools → Requirements… to get it", dep.label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dep_has_a_way_to_get_it() {
        for d in all() {
            assert!(!d.bins.is_empty(), "{} probes nothing", d.id);
            assert!(d.url.starts_with("https://"), "{} needs a download page", d.id);
            assert!(!d.enables.is_empty(), "{} doesn't say what it's for", d.id);
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = all().iter().map(|d| d.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate dependency id");
    }

    /// A manager with no package name for a tool must fall back to the download
    /// link — never to an install command with an empty package, which would run
    /// `sudo apt install -y` and hang on a prompt.
    #[test]
    fn empty_package_falls_back_to_the_link() {
        let d = get("clipboard");
        if let Some(d) = d {
            assert!(matches!(d.install(Manager::Brew), Install::Link));
        }
        let scp = get("scp").unwrap();
        assert!(matches!(scp.install(Manager::Winget), Install::Link));
    }

    #[test]
    fn linux_install_is_copyable_not_run() {
        // A GUI app must not prompt for a root password; the Linux path hands
        // over the command instead of running it.
        let t = get("tesseract").unwrap();
        match t.install(Manager::Apt) {
            Install::Copy { cmd, .. } => assert!(cmd.contains("tesseract-ocr")),
            other => panic!("expected a copyable command, got {other:?}"),
        }
    }

    #[test]
    fn winget_command_is_non_interactive() {
        let t = get("tesseract").unwrap();
        match t.install(Manager::Winget) {
            Install::Run { args, .. } => {
                assert!(args.contains(&"--accept-package-agreements".to_string()));
                assert!(args.contains(&"-e".to_string()), "match the id exactly");
            }
            other => panic!("expected a runnable install, got {other:?}"),
        }
    }

    /// `which` must find something that certainly exists, and reject something
    /// that certainly doesn't — otherwise every dependency reads as missing.
    #[test]
    fn which_finds_a_real_binary() {
        let real = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
        assert!(which(real).is_some(), "{real} should be on PATH");
        assert!(which("trellis-definitely-not-a-real-binary").is_none());
    }
}
