// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::{WebViewBuilder, http::Request};

const DEFAULT_URL: &str = "https://duckduckgo.com";

/// Injected into every page; handles the vim keybindings and command bar.
const KEYS_JS: &str = include_str!("keys.js");

/// Requests from the page script that need the native side.
enum UserEvent {
    Open(String),
    Title(String),
    Quit,
}

/// Parses a message sent by `keys.js` through `window.ipc.postMessage`.
fn parse_ipc(msg: &str) -> Option<UserEvent> {
    match msg.split_once(' ') {
        Some(("open", url)) => Some(UserEvent::Open(normalize_url(url))),
        None if msg == "quit" => Some(UserEvent::Quit),
        _ => None,
    }
}

/// Turns user input into a loadable URL, adding `https://` when no scheme is given.
fn normalize_url(input: &str) -> String {
    let input = input.trim();
    if input.contains("://") || input.starts_with("about:") || input.starts_with("data:") {
        input.to_string()
    } else {
        format!("https://{input}")
    }
}

fn main() -> wry::Result<()> {
    let url = std::env::args()
        .nth(1)
        .map(|arg| normalize_url(&arg))
        .unwrap_or_else(|| DEFAULT_URL.to_string());

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("zeteme")
        .build(&event_loop)
        .expect("failed to create window");

    let ipc_proxy = event_loop.create_proxy();
    let title_proxy = event_loop.create_proxy();
    let webview = WebViewBuilder::new()
        .with_url(&url)
        .with_initialization_script(KEYS_JS)
        .with_ipc_handler(move |req: Request<String>| {
            if let Some(event) = parse_ipc(req.body()) {
                let _ = ipc_proxy.send_event(event);
            }
        })
        .with_document_title_changed_handler(move |title| {
            let _ = title_proxy.send_event(UserEvent::Title(title));
        })
        .build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            }
            | Event::UserEvent(UserEvent::Quit) => *control_flow = ControlFlow::Exit,
            Event::UserEvent(UserEvent::Open(url)) => {
                let _ = webview.load_url(&url);
            }
            Event::UserEvent(UserEvent::Title(title)) => {
                window.set_title(&format!("{title} - zeteme"));
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{UserEvent, normalize_url, parse_ipc};

    #[test]
    fn adds_https_when_scheme_missing() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
    }

    #[test]
    fn keeps_existing_scheme() {
        assert_eq!(normalize_url("http://example.com"), "http://example.com");
        assert_eq!(normalize_url("about:blank"), "about:blank");
    }

    #[test]
    fn parses_ipc_messages() {
        assert!(matches!(
            parse_ipc("open example.com"),
            Some(UserEvent::Open(url)) if url == "https://example.com"
        ));
        assert!(matches!(parse_ipc("quit"), Some(UserEvent::Quit)));
        assert!(parse_ipc("bogus").is_none());
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(normalize_url("  example.com "), "https://example.com");
    }
}
