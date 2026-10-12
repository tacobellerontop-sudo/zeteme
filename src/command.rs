//! Messages from the page script (`keys.js`) and URL handling.

const SEARCH_URL: &str = "https://duckduckgo.com/?q=";

#[derive(Debug, PartialEq)]
pub enum Command {
    Open(String),
    TabOpen(String),
    Close,
    Undo,
    NextTab,
    PrevTab,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Fullscreen(bool),
    Settings,
    SettingsReady,
    SetCheckUpdates(bool),
    CheckUpdate,
    InstallUpdate,
    Quit,
}

impl Command {
    /// Commands only the settings page may send, so that web pages can't
    /// change settings or start an install.
    pub fn settings_only(&self) -> bool {
        matches!(
            self,
            Command::SettingsReady
                | Command::SetCheckUpdates(_)
                | Command::CheckUpdate
                | Command::InstallUpdate
        )
    }
}

/// Parses a message sent by `keys.js` through `window.ipc.postMessage`.
pub fn parse_ipc(msg: &str) -> Option<Command> {
    let (name, arg) = msg.split_once(' ').unwrap_or((msg, ""));
    let arg = arg.trim();
    let command = match name {
        "open" if !arg.is_empty() => Command::Open(normalize_url(arg)),
        "tabopen" if !arg.is_empty() => Command::TabOpen(normalize_url(arg)),
        "close" => Command::Close,
        "undo" => Command::Undo,
        "tab-next" => Command::NextTab,
        "tab-prev" => Command::PrevTab,
        "zoom-in" => Command::ZoomIn,
        "zoom-out" => Command::ZoomOut,
        "zoom-reset" => Command::ZoomReset,
        "fullscreen" => Command::Fullscreen(arg == "on"),
        "settings" => Command::Settings,
        "settings-ready" => Command::SettingsReady,
        "set" => match arg {
            "check_updates on" => Command::SetCheckUpdates(true),
            "check_updates off" => Command::SetCheckUpdates(false),
            _ => return None,
        },
        "update-check" => Command::CheckUpdate,
        "update-install" => Command::InstallUpdate,
        "quit" => Command::Quit,
        _ => return None,
    };
    Some(command)
}

/// Turns what the user typed into a URL: full URLs pass through, things that
/// look like hosts get `https://`, and anything else becomes a web search.
pub fn normalize_url(input: &str) -> String {
    let input = input.trim();
    if input.contains("://") || input.starts_with("about:") || input.starts_with("data:") {
        input.to_string()
    } else if input == "localhost"
        || input.starts_with("localhost:")
        || input.starts_with("localhost/")
    {
        // Local dev servers rarely have TLS.
        format!("http://{input}")
    } else if looks_like_host(input) {
        format!("https://{input}")
    } else {
        format!("{SEARCH_URL}{}", encode_query(input))
    }
}

fn looks_like_host(input: &str) -> bool {
    if input.contains(char::is_whitespace) {
        return false;
    }
    let host = input.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
}

fn encode_query(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_https_to_hosts() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(
            normalize_url("  example.com/a?b "),
            "https://example.com/a?b"
        );
        assert_eq!(normalize_url("localhost:8080"), "http://localhost:8080");
    }

    #[test]
    fn keeps_existing_scheme() {
        assert_eq!(normalize_url("http://example.com"), "http://example.com");
        assert_eq!(normalize_url("about:blank"), "about:blank");
    }

    #[test]
    fn searches_everything_else() {
        assert_eq!(
            normalize_url("rust webview"),
            "https://duckduckgo.com/?q=rust+webview"
        );
        assert_eq!(normalize_url("c++"), "https://duckduckgo.com/?q=c%2B%2B");
        assert_eq!(
            normalize_url("what is example.com"),
            "https://duckduckgo.com/?q=what+is+example.com"
        );
    }

    #[test]
    fn parses_ipc_messages() {
        assert_eq!(
            parse_ipc("open example.com"),
            Some(Command::Open("https://example.com".into()))
        );
        assert_eq!(
            parse_ipc("tabopen rust"),
            Some(Command::TabOpen("https://duckduckgo.com/?q=rust".into()))
        );
        assert_eq!(parse_ipc("close"), Some(Command::Close));
        assert_eq!(parse_ipc("quit"), Some(Command::Quit));
        assert_eq!(parse_ipc("fullscreen on"), Some(Command::Fullscreen(true)));
        assert_eq!(
            parse_ipc("fullscreen off"),
            Some(Command::Fullscreen(false))
        );
        assert_eq!(parse_ipc("settings"), Some(Command::Settings));
        assert_eq!(
            parse_ipc("set check_updates off"),
            Some(Command::SetCheckUpdates(false))
        );
        assert_eq!(parse_ipc("set bogus on"), None);
        assert!(parse_ipc("update-install").unwrap().settings_only());
        assert!(!parse_ipc("settings").unwrap().settings_only());
        assert_eq!(parse_ipc("open "), None);
        assert_eq!(parse_ipc("bogus"), None);
    }
}
