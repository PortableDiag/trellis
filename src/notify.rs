//! Desktop notifications — the immediate half of being told things.
//!
//! ## Why this exists alongside the Telegram plugin, rather than instead of it
//!
//! The operator put it exactly right: a desktop or phone notification is
//! **dismissible**, and a Telegram message or an email is not. Swipe one away and
//! it is gone; a message sits in a list until it is dealt with. So the two are
//! not competing implementations of one feature — they answer different
//! questions, and shipping only one of them would leave the other unanswered:
//!
//! - **A notification says "this just happened, look now."** It is immediate, it
//!   is local (no internet, no third party, nothing leaves the machine), and it
//!   can be ignored at no cost. That is the right shape for *an agent just
//!   changed something while you were in another window*.
//! - **A message says "this is outstanding, and it will still be outstanding
//!   later."** That is the right shape for a digest of what is due.
//!
//! Neither is durable in the sense that matters most: **nothing fires while
//! Trellis is closed.** A desktop app is not a service. That is stated here, in
//! the Settings panel, and in the plugin's own window rather than discovered.
//!
//! ## What it will not do
//!
//! Notify while you are looking at the window. If Trellis has focus, an agent's
//! edit is already visible — the canvas is live — so a popup would be telling
//! you what you can see. Every notification here is gated on the window being
//! unfocused.

use std::process::{Command, Stdio};

/// How the notification reached the desktop, for the status line and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// Handed to a notification daemon.
    Ok,
    /// No tool to send it with — not an error, just a machine without one.
    NoTool,
    /// The tool existed and failed.
    Failed,
}

/// Send a desktop notification, if this machine can.
///
/// **Fire-and-forget by design.** A notification that blocks the UI thread to
/// report that it could not be delivered is worse than the notification being
/// missed, so this spawns and does not wait for the child beyond the spawn
/// itself.
pub fn send(summary: &str, body: &str) -> Sent {
    #[cfg(target_os = "linux")]
    {
        // libnotify's CLI is on essentially every desktop Linux, and going
        // through it rather than linking a D-Bus client keeps this an *optional*
        // dependency — the same shape as xclip, tesseract and the screenshot
        // tools. A machine without it loses notifications and nothing else.
        return spawn(
            "notify-send",
            &["--app-name=Trellis", "--icon=dialog-information", summary, body],
        );
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification {} with title {}",
            applescript_string(body),
            applescript_string(summary)
        );
        return spawn("osascript", &["-e", &script]);
    }
    #[cfg(target_os = "windows")]
    {
        // PowerShell's toast API is the only route that needs no install.
        let script = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, \
             ContentType = WindowsRuntime] > $null; \
             $t = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent(\
             [Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $t.GetElementsByTagName('text')[0].AppendChild($t.CreateTextNode({})) > $null; \
             $t.GetElementsByTagName('text')[1].AppendChild($t.CreateTextNode({})) > $null; \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Trellis')\
             .Show([Windows.UI.Notifications.ToastNotification]::new($t))",
            ps_string(summary),
            ps_string(body)
        );
        return spawn("powershell", &["-NoProfile", "-Command", &script]);
    }
    #[allow(unreachable_code)]
    Sent::NoTool
}

fn spawn(cmd: &str, args: &[&str]) -> Sent {
    match Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // Reap it on a detached thread. These exit immediately, but a child
            // nobody waits on is a zombie — the exact bug the X11 selection
            // helper shipped with until v0.103.5.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Sent::Ok
        }
        // The distinction matters to the caller: "no daemon here" is a fact
        // about the machine, not a failure to report.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Sent::NoTool,
        Err(_) => Sent::Failed,
    }
}

#[cfg(target_os = "macos")]
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "windows")]
fn ps_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The digest line for a set of tasks: what is overdue, and what is due today.
///
/// Returns `None` when there is nothing to say. **An empty digest must not be
/// sent** — a notification that says "nothing is due" trains you to ignore the
/// next one, which is the only thing a notifier cannot recover from.
pub fn digest(doc: &crate::model::Document, today: i64) -> Option<(String, String)> {
    let mut overdue = 0usize;
    let mut due_today = 0usize;
    let mut first: Option<String> = None;
    for t in doc.tasks() {
        if t.done {
            continue;
        }
        match t.due_days {
            Some(d) if d < today => {
                overdue += 1;
                first.get_or_insert_with(|| t.title.clone());
            }
            Some(d) if d == today => {
                due_today += 1;
                first.get_or_insert_with(|| t.title.clone());
            }
            _ => {}
        }
    }
    if overdue == 0 && due_today == 0 {
        return None;
    }
    let mut parts = Vec::new();
    if overdue > 0 {
        parts.push(format!("{overdue} overdue"));
    }
    if due_today > 0 {
        parts.push(format!("{due_today} due today"));
    }
    let summary = format!("Trellis — {}", parts.join(", "));
    // Name one of them. A count alone makes you open the app to find out whether
    // it is the thing you already know about.
    let body = first.map(|t| elide(&t, 90)).unwrap_or_default();
    Some((summary, body))
}

/// Cut a title to `max` characters on a char boundary, with an ellipsis.
pub fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CardKind, Document};

    fn doc_with(due: &[(&str, i64)], today: i64) -> Document {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Open Items".into());
        for (title, day) in due {
            let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
            let c = doc.card_mut(n, cid).unwrap();
            c.title = (*title).to_string();
            // Days-since-epoch back to a date, through the same formatter the
            // rest of the app uses, so the fixture cannot disagree with the parser.
            let date = chrono::DateTime::from_timestamp((today + day) * 86_400, 0)
                .unwrap()
                .format("%Y-%m-%d")
                .to_string();
            c.body = format!("due:: {date}");
        }
        doc
    }

    /// Nothing due is not a notification. A notifier that speaks when it has
    /// nothing to say is one you learn to ignore.
    #[test]
    fn an_empty_digest_is_not_sent() {
        let doc = doc_with(&[("Later", 30)], 20_000);
        assert!(digest(&doc, 20_000).is_none());
    }

    #[test]
    fn a_digest_counts_overdue_and_today_separately_and_names_one() {
        let doc = doc_with(&[("Ship it", -2), ("Call back", 0), ("Someday", 9)], 20_000);
        let (summary, body) = digest(&doc, 20_000).expect("something is due");
        assert!(summary.contains("1 overdue"), "{summary}");
        assert!(summary.contains("1 due today"), "{summary}");
        assert!(!body.is_empty(), "a count alone makes you open the app to find out what");
    }

    #[test]
    fn a_long_title_is_elided_on_a_char_boundary() {
        let s = "é".repeat(200);
        let out = elide(&s, 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.ends_with('…'));
    }
}
