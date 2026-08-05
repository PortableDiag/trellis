//! Trellis — a hierarchical, spatial note-taking app.
//!
//! A tree of nodes (the structure) where every node's body is a free-form
//! basket of draggable, editable cards (the spatial surface).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod backup;
mod canvas;
mod changelog;
mod deps;
mod images;
mod model;
mod tree;

use std::path::PathBuf;

const USAGE: &str = "\
Trellis — a hierarchical, spatial note-taking app.

USAGE:
    trellis [FILE] [OPTIONS]

ARGS:
    <FILE>    Document to open. If the path doesn't exist yet, Trellis starts a
              new document and saves it there.

OPTIONS:
    -p, --port <PORT>     Port for the agent HTTP API (default 7373). Overrides
                          the saved setting for this run.
    -d, --data-dir <DIR>  Keep this instance's settings under <DIR>: its own API
                          key, port, theme, backup config and autosave slot.
                          Give each instance a different directory so they don't
                          overwrite each other's settings.
    -h, --help            Print this help.
    -V, --version         Print the version.

Run independent instances side by side — each with its own document, API port
and settings — so an agent reaches a given document by its port:

    trellis ~/work.ron     --port 7373 --data-dir ~/.local/share/trellis-work
    trellis ~/personal.ron --port 7374 --data-dir ~/.local/share/trellis-personal

GET /api/instance reports which document an instance is serving.
";

/// Startup overrides parsed from the command line. Deliberately tiny — enough to
/// run several independent instances, not a general config surface (everything
/// else lives in Settings).
#[derive(Default)]
struct Args {
    doc: Option<PathBuf>,
    port: Option<u16>,
    data_dir: Option<PathBuf>,
}

fn main() -> eframe::Result<()> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("trellis: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    // Point this instance's settings at its own directory *before* eframe reads
    // them, so two instances don't clobber each other's port/key/theme/backup
    // config and template library.
    //
    // This used to be done by setting XDG_DATA_HOME, which eframe honours **only
    // on Linux/BSD** — on macOS it reads $HOME/Library/Application Support and on
    // Windows the Roaming AppData known-folder, neither of which takes an
    // environment override. So `--data-dir` silently moved nothing but the
    // autosave slot there, and two instances shared one API key and port:
    // one-instance-per-document did not actually work off Linux.
    //
    // `persistence_path` names the settings file outright, on every platform.
    // The path deliberately reproduces the layout XDG_DATA_HOME produced
    // (`<dir>/trellis/app.ron`), so existing Linux instances keep their settings
    // with nothing to migrate.
    let persistence_path = match &args.data_dir {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("trellis: could not create data dir {}: {e}", dir.display());
                std::process::exit(2);
            }
            Some(dir.join("trellis").join("app.ron"))
        }
        None => None,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // The open document's name, so several instances are tellable apart
            // in the taskbar from the first frame (kept current in `update`).
            .with_title(app::window_title(args.doc.as_deref()))
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([720.0, 460.0])
            .with_icon(load_icon()),
        persistence_path,
        ..Default::default()
    };

    let startup = app::Startup { doc: args.doc, port: args.port, data_dir: args.data_dir };
    eframe::run_native(
        "Trellis",
        options,
        Box::new(move |cc| Ok(Box::new(app::TrellisApp::new(cc, startup)))),
    )
}

fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    parse_argv(&argv)
}

/// Parse `trellis [FILE] [-p PORT] [-d DIR]`. Accepts both `--flag value` and
/// `--flag=value`. `--help` / `--version` print and exit.
fn parse_argv(argv: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].clone();
        // Split `--flag=value` so both spellings feed the same code below.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with('-') => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        match flag.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("trellis {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-p" | "--port" => {
                let v = value_for(argv, &mut i, inline, &flag)?;
                let port: u16 = v
                    .parse()
                    .map_err(|_| format!("{flag}: '{v}' is not a port number"))?;
                if port == 0 {
                    return Err(format!("{flag}: must be 1-65535"));
                }
                out.port = Some(port);
            }
            "-d" | "--data-dir" => {
                let v = value_for(argv, &mut i, inline, &flag)?;
                let dir = PathBuf::from(v);
                // Anchor a relative path now, against the shell's cwd — the app
                // would otherwise resolve it against its own, which is not the
                // same thing once it's launched from a menu or a shortcut.
                out.data_dir = Some(if dir.is_absolute() {
                    dir
                } else {
                    std::env::current_dir().map(|c| c.join(&dir)).unwrap_or(dir)
                });
            }
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option '{other}'"));
            }
            _ => {
                if out.doc.is_some() {
                    return Err("only one document can be opened".to_string());
                }
                out.doc = Some(PathBuf::from(arg));
            }
        }
        i += 1;
    }
    Ok(out)
}

/// The value of a flag: either the `=value` half or the next argument.
fn value_for(
    argv: &[String],
    i: &mut usize,
    inline: Option<String>,
    flag: &str,
) -> Result<String, String> {
    if let Some(v) = inline {
        return Ok(v);
    }
    *i += 1;
    argv.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Result<Args, String> {
        parse_argv(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    // `--data-dir` anchors a *relative* path against the cwd, so a test that
    // wants the value passed through untouched has to hand it an absolute one —
    // and the platforms disagree about what that means. On Windows a path needs
    // a drive letter: `/d/work` is merely root-relative, and gets the current
    // drive prepended (`D:/d/work`), which is correct and exactly what an
    // earlier Unix-shaped assertion here mistook for a bug.
    #[cfg(windows)]
    const ABS_WORK: &str = r"C:\d\work";
    #[cfg(windows)]
    const ABS_PERSONAL: &str = r"C:\d\personal";
    #[cfg(not(windows))]
    const ABS_WORK: &str = "/d/work";
    #[cfg(not(windows))]
    const ABS_PERSONAL: &str = "/d/personal";

    #[test]
    fn bare_launch_overrides_nothing() {
        let a = args(&[]).unwrap();
        assert!(a.doc.is_none() && a.port.is_none() && a.data_dir.is_none());
    }

    #[test]
    fn parses_document_port_and_data_dir_in_both_spellings() {
        let a = args(&["/n/work.ron", "--port", "7391", "--data-dir", ABS_WORK]).unwrap();
        assert_eq!(a.doc, Some(PathBuf::from("/n/work.ron")));
        assert_eq!(a.port, Some(7391));
        assert_eq!(a.data_dir, Some(PathBuf::from(ABS_WORK)));

        let b = args(&["--port=7392", "-d", ABS_PERSONAL, "/n/personal.ron"]).unwrap();
        assert_eq!(b.doc, Some(PathBuf::from("/n/personal.ron")));
        assert_eq!(b.port, Some(7392));
        assert_eq!(b.data_dir, Some(PathBuf::from(ABS_PERSONAL)));
    }

    #[test]
    fn relative_data_dir_is_made_absolute() {
        // A relative --data-dir must be anchored at parse time, or the settings
        // file lands somewhere that depends on how the app happened to be
        // launched — and two instances can silently share one.
        let a = args(&["-d", "sub/dir"]).unwrap();
        assert!(a.data_dir.as_ref().unwrap().is_absolute());
        assert!(a.data_dir.unwrap().ends_with("sub/dir"));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(args(&["--port", "abc"]).is_err());
        assert!(args(&["--port", "0"]).is_err(), "port 0 would bind an arbitrary port");
        assert!(args(&["--port"]).is_err(), "flag with no value");
        assert!(args(&["--frobnicate"]).is_err());
        assert!(args(&["a.ron", "b.ron"]).is_err(), "one document per instance");
    }
}

/// The window/taskbar icon, baked into the binary at compile time.
fn load_icon() -> egui::IconData {
    let png = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(png)
        .expect("decode embedded icon")
        .to_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}
