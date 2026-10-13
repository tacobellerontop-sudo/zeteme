//! User settings, kept as `key=value` lines in `settings.txt`, and the
//! settings page that edits them.

use std::{fs, path::Path};

use crate::{
    extensions::Extension,
    tabbar::{Position, Show},
    update::{CURRENT_VERSION, Status},
};

const PAGE_HTML: &str = include_str!("settings.html");

/// The URL wry serves the `zeteme` custom protocol under.
#[cfg(windows)]
pub const SETTINGS_URL: &str = "http://zeteme.settings/";
#[cfg(not(windows))]
pub const SETTINGS_URL: &str = "zeteme://settings/";

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub check_updates: bool,
    pub tabs_show: Show,
    pub tabs_position: Position,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            check_updates: true,
            tabs_show: Show::Auto,
            tabs_position: Position::Top,
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
            let value = value.trim();
            match key.trim() {
                "check_updates" => settings.check_updates = value != "false",
                "tabs_show" => {
                    settings.tabs_show = Show::parse(value).unwrap_or(settings.tabs_show)
                }
                "tabs_position" => {
                    settings.tabs_position =
                        Position::parse(value).unwrap_or(settings.tabs_position)
                }
                _ => {}
            }
        }
        settings
    }

    pub fn save(&self, file: &Path) {
        let _ = fs::write(
            file,
            format!(
                "check_updates={}\ntabs_show={}\ntabs_position={}\n",
                self.check_updates,
                self.tabs_show.as_str(),
                self.tabs_position.as_str()
            ),
        );
    }
}

/// The settings page, filled in with the current values.
pub fn page(settings: &Settings) -> String {
    PAGE_HTML
        .replace("{{version}}", CURRENT_VERSION)
        .replace(
            "{{check_updates}}",
            if settings.check_updates {
                "checked"
            } else {
                ""
            },
        )
        .replace("{{tabs_show}}", settings.tabs_show.as_str())
        .replace("{{tabs_position}}", settings.tabs_position.as_str())
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

/// An installed extension as the settings page lists it.
pub struct ExtensionRow<'a> {
    pub extension: &'a Extension,
    pub popup: bool,
    pub options: bool,
}

/// A script that lists `rows` on an open settings page, with `status` about
/// the last install or removal.
pub fn extensions_script(rows: &[ExtensionRow], status: &str) -> String {
    let rows: Vec<String> = rows
        .iter()
        .map(|row| {
            format!(
                "[{}, {}, {}, {}, {}]",
                js_string(&row.extension.id),
                js_string(&row.extension.name),
                row.extension.enabled,
                row.popup,
                row.options
            )
        })
        .collect();
    format!(
        "window.zetemeExtensions && zetemeExtensions([{}], {})",
        rows.join(", "),
        js_string(status)
    )
}

pub fn js_string(text: &str) -> String {
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
        let tabs = Settings::parse("tabs_show=never\ntabs_position=left\n");
        assert_eq!(tabs.tabs_show, Show::Never);
        assert_eq!(tabs.tabs_position, Position::Left);
        assert_eq!(Settings::parse("tabs_show=bogus").tabs_show, Show::Auto);
    }

    #[test]
    fn fills_in_page() {
        let page = page(&Settings {
            check_updates: false,
            ..Settings::default()
        });
        assert!(page.contains(CURRENT_VERSION));
        assert!(!page.contains("{{"));
    }

    #[test]
    fn lists_extensions() {
        let extension = Extension {
            id: "abc".into(),
            name: "uBlock \"Lite\"".into(),
            enabled: true,
        };
        let rows = [ExtensionRow {
            extension: &extension,
            popup: true,
            options: false,
        }];
        assert_eq!(
            extensions_script(&rows, ""),
            r#"window.zetemeExtensions && zetemeExtensions([["abc", "uBlock \"Lite\"", true, true, false]], "")"#
        );
    }

    #[test]
    fn escapes_status() {
        assert_eq!(
            status_script(&Status::Failed("bad \"x\"\n".into())),
            r#"window.zetemeUpdate && zetemeUpdate("failed", "bad \"x\"\u000a")"#
        );
    }
}
