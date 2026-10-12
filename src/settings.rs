//! User settings, kept as `key=value` lines in `settings.txt`, and the
//! settings page that edits them.

use std::{fs, path::Path};

use crate::update::{CURRENT_VERSION, Status};

const PAGE_HTML: &str = include_str!("settings.html");

/// The URL wry serves the `zeteme` custom protocol under.
#[cfg(windows)]
pub const SETTINGS_URL: &str = "http://zeteme.settings/";
#[cfg(not(windows))]
pub const SETTINGS_URL: &str = "zeteme://settings/";

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            check_updates: true,
        }
    }
}

impl Settings {
    pub fn load(file: &Path) -> Settings {
        Settings::parse(&fs::read_to_string(file).unwrap_or_default())
    }

    fn parse(text: &str) -> Settings {
        let mut settings = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() == "check_updates" {
                settings.check_updates = value.trim() != "false";
            }
        }
        settings
    }

    pub fn save(&self, file: &Path) {
        let _ = fs::write(file, format!("check_updates={}\n", self.check_updates));
    }
}

/// The settings page, filled in with the current values.
pub fn page(settings: &Settings) -> String {
    PAGE_HTML.replace("{{version}}", CURRENT_VERSION).replace(
        "{{check_updates}}",
        if settings.check_updates {
            "checked"
        } else {
            ""
        },
    )
}

/// A script that shows `status` on an open settings page.
pub fn status_script(status: &Status) -> String {
    let (state, detail) = match status {
        Status::Idle => ("idle", ""),
        Status::Checking => ("checking", ""),
        Status::UpToDate => ("uptodate", ""),
        Status::Available(tag) => ("available", tag.as_str()),
        Status::Downloading(tag) => ("downloading", tag.as_str()),
        Status::Failed(err) => ("failed", err.as_str()),
    };
    format!(
        "window.zetemeUpdate && zetemeUpdate({}, {})",
        js_string(state),
        js_string(detail)
    )
}

fn js_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_settings() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert!(!Settings::parse("check_updates=false\n").check_updates);
        assert!(Settings::parse("check_updates = true").check_updates);
        assert!(Settings::parse("junk\nother=1").check_updates);
    }

    #[test]
    fn fills_in_page() {
        let page = page(&Settings {
            check_updates: false,
        });
        assert!(page.contains(CURRENT_VERSION));
        assert!(!page.contains("{{"));
    }

    #[test]
    fn escapes_status() {
        assert_eq!(
            status_script(&Status::Failed("bad \"x\"\n".into())),
            r#"window.zetemeUpdate && zetemeUpdate("failed", "bad \"x\"\u000a")"#
        );
    }
}
