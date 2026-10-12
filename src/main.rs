// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

const DEFAULT_URL: &str = "https://duckduckgo.com";

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

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("zeteme")
        .build(&event_loop)
        .expect("failed to create window");

    let _webview = WebViewBuilder::new().with_url(&url).build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::normalize_url;

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
    fn trims_whitespace() {
        assert_eq!(normalize_url("  example.com "), "https://example.com");
    }
}
