//! The Krita half of the link: a small Python plugin ("Animator Link") that
//! reloads a linked document in place when this app sends it again — Krita
//! has no reload of its own — and the mailbox files the two talk through.
//!
//! The plugin ships inside the app (`krita-plugin/`) and is written into
//! Krita's user plugin folder by File ▸ Install Krita helper. The protocol
//! itself is described in `krita-plugin/animator_link/mailbox.py`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Bumped with any change to the plugin files; the installed `.desktop`
/// carries it, so the menu can offer a reinstall.
pub const VERSION: u32 = 1;

const DESKTOP: &str = include_str!("../krita-plugin/animator_link.desktop");
const FILES: &[(&str, &str)] = &[
    ("animator_link/__init__.py", include_str!("../krita-plugin/animator_link/__init__.py")),
    ("animator_link/extension.py", include_str!("../krita-plugin/animator_link/extension.py")),
    ("animator_link/mailbox.py", include_str!("../krita-plugin/animator_link/mailbox.py")),
];

/// Krita's user plugin folder: `pykrita` under Krita's data folder — `%APPDATA%`
/// on Windows, `~/Library/Application Support` on macOS, `~/.local/share` on
/// Linux.
pub fn plugin_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("krita").join("pykrita"))
}

/// The plugin version installed in `dir`, if any.
pub fn installed_version(dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(dir.join("animator_link.desktop")).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("X-Animator-Version="))
        .and_then(|v| v.trim().parse().ok())
}

/// Whether some version of the plugin is in Krita's plugin folder. (Whether
/// it is *enabled* in Krita only shows when it answers.)
pub fn installed() -> bool {
    plugin_dir().is_some_and(|d| installed_version(&d).is_some())
}

/// Whether the installed plugin is older than the one this app carries.
pub fn outdated() -> bool {
    plugin_dir().and_then(|d| installed_version(&d)).is_some_and(|v| v < VERSION)
}

/// Write the plugin into `dir` (Krita's `pykrita` folder).
pub fn install_into(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join("animator_link")).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(dir.join("animator_link.desktop"), DESKTOP).context("writing animator_link.desktop")?;
    for (rel, text) in FILES {
        std::fs::write(dir.join(rel), text).with_context(|| format!("writing {rel}"))?;
    }
    Ok(())
}

/// Install into Krita's user plugin folder; returns where.
pub fn install() -> Result<PathBuf> {
    let dir = plugin_dir().context("no user data folder to install the Krita plugin into")?;
    install_into(&dir)?;
    Ok(dir)
}

// --- mailbox ---------------------------------------------------------------

fn sibling(kra: &Path, suffix: &str) -> PathBuf {
    let mut s = OsString::from(kra.as_os_str());
    s.push(suffix);
    PathBuf::from(s)
}

/// What this app writes for the plugin.
pub fn to_krita(kra: &Path) -> PathBuf {
    sibling(kra, ".to-krita")
}

/// What the plugin writes for this app.
pub fn to_app(kra: &Path) -> PathBuf {
    sibling(kra, ".to-app")
}

/// Write `key=value` lines atomically: the plugin never reads half a message.
pub fn write_msg(path: &Path, fields: &[(&str, String)]) -> Result<()> {
    let text: String = fields.iter().map(|(k, v)| format!("{k}={}\n", v.replace('\n', " "))).collect();
    let tmp = sibling(path, ".tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

pub fn read_msg(path: &Path) -> Option<HashMap<String, String>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect(),
    )
}

/// Tell the plugin about round `seq`: `"request"` (get ready: save Krita's
/// edits if any) or `"sent"` (the new file is on disk: reload it).
pub fn tell(kra: &Path, seq: u64, msg: &str) -> Result<()> {
    write_msg(&to_krita(kra), &[("seq", seq.to_string()), ("msg", msg.to_string())])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Krita is ready for the new file; `saved` = it saved edits first.
    Ready { seq: u64, saved: bool },
    Reloaded { seq: u64 },
    Error { seq: u64, text: String },
}

/// The plugin's latest message about `kra`, if any.
pub fn reply(kra: &Path) -> Option<Reply> {
    let m = read_msg(&to_app(kra))?;
    let seq = m.get("seq")?.parse().ok()?;
    match m.get("msg")?.as_str() {
        "ready" => Some(Reply::Ready { seq, saved: m.get("saved").is_some_and(|s| s == "1") }),
        "reloaded" => Some(Reply::Reloaded { seq }),
        "error" => Some(Reply::Error { seq, text: m.get("text").cloned().unwrap_or_default() }),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("animator-helper-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Stand in for the plugin: write its reply.
    pub(crate) fn plugin_says(kra: &Path, fields: &[(&str, &str)]) {
        let f: Vec<(&str, String)> = fields.iter().map(|(k, v)| (*k, v.to_string())).collect();
        write_msg(&to_app(kra), &f).unwrap();
    }

    #[test]
    fn installs_every_file_with_its_version() {
        let d = tmp_dir("install");
        assert_eq!(installed_version(&d), None);
        install_into(&d).unwrap();
        assert_eq!(installed_version(&d), Some(VERSION));
        for (rel, text) in FILES {
            assert_eq!(std::fs::read_to_string(d.join(rel)).unwrap(), *text);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Whatever ends up in the plugin's reply file — junk, half a message,
    /// an unknown kind — reads as "no answer", never as a wrong one.
    #[test]
    fn junk_replies_read_as_no_answer() {
        let d = tmp_dir("junk");
        let kra = d.join("j.kra");
        for junk in ["", "\u{0}\u{1}garbage", "seq=abc\nmsg=ready", "msg=reloaded", "seq=3\nmsg=launch"] {
            std::fs::write(to_app(&kra), junk).unwrap();
            assert_eq!(reply(&kra), None, "{junk:?}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mailbox_round_trip_and_no_temp_left() {
        let d = tmp_dir("mailbox");
        let kra = d.join("shot-1.kra");
        tell(&kra, 7, "request").unwrap();
        let m = read_msg(&to_krita(&kra)).unwrap();
        assert_eq!((m["seq"].as_str(), m["msg"].as_str()), ("7", "request"));
        let names: Vec<String> =
            std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["shot-1.kra.to-krita"]);

        assert_eq!(reply(&kra), None);
        plugin_says(&kra, &[("seq", "7"), ("msg", "ready"), ("saved", "1")]);
        assert_eq!(reply(&kra), Some(Reply::Ready { seq: 7, saved: true }));
        plugin_says(&kra, &[("seq", "7"), ("msg", "error"), ("text", "no = luck")]);
        assert_eq!(reply(&kra), Some(Reply::Error { seq: 7, text: "no = luck".into() }));
        let _ = std::fs::remove_dir_all(&d);
    }
}
