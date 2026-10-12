// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod command;

use std::{fs, path::PathBuf};

use command::{Command, normalize_url, parse_ipc};
use tao::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Fullscreen, Window, WindowBuilder},
};
use wry::{NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder, http::Request};

const DEFAULT_URL: &str = "https://duckduckgo.com";
const ZOOM_STEP: f64 = 0.1;

/// Injected into every page; handles the vim keybindings, hints and command bar.
const KEYS_JS: &str = include_str!("keys.js");

enum UserEvent {
    Command(Command),
    Title(u64, String),
    NewTab(String),
}

struct Tab {
    id: u64,
    webview: WebView,
    title: String,
    zoom: f64,
}

struct Browser {
    window: Window,
    context: WebContext,
    proxy: EventLoopProxy<UserEvent>,
    tabs: Vec<Tab>,
    current: usize,
    closed: Vec<String>,
    next_id: u64,
    session_file: PathBuf,
}

/// Where cookies, logins and the saved session live: `%LOCALAPPDATA%\zeteme`.
fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zeteme")
}

impl Browser {
    fn bounds(&self) -> Rect {
        let size = self.window.inner_size();
        Rect {
            position: PhysicalPosition::new(0, 0).into(),
            size: PhysicalSize::new(size.width, size.height).into(),
        }
    }

    fn open_tab(&mut self, url: &str) -> wry::Result<()> {
        let id = self.next_id;
        self.next_id += 1;

        let command_proxy = self.proxy.clone();
        let title_proxy = self.proxy.clone();
        let popup_proxy = self.proxy.clone();
        let bounds = self.bounds();
        let webview = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_url(url)
            .with_bounds(bounds)
            .with_initialization_script(KEYS_JS)
            .with_ipc_handler(move |req: Request<String>| {
                if let Some(command) = parse_ipc(req.body()) {
                    let _ = command_proxy.send_event(UserEvent::Command(command));
                }
            })
            .with_document_title_changed_handler(move |title| {
                let _ = title_proxy.send_event(UserEvent::Title(id, title));
            })
            .with_new_window_req_handler(move |url, features| {
                // Sized popups (e.g. "Sign in with ..." flows) need a real
                // window to talk back to their opener; links open as tabs.
                if features.size.is_some() {
                    NewWindowResponse::Allow
                } else {
                    let _ = popup_proxy.send_event(UserEvent::NewTab(url));
                    NewWindowResponse::Deny
                }
            })
            .build_as_child(&self.window)?;

        self.tabs.push(Tab {
            id,
            webview,
            title: String::new(),
            zoom: 1.0,
        });
        self.select(self.tabs.len() - 1);
        Ok(())
    }

    fn select(&mut self, index: usize) {
        self.current = index;
        for (i, tab) in self.tabs.iter().enumerate() {
            let _ = tab.webview.set_visible(i == index);
        }
        let _ = self.tabs[index].webview.focus();
        self.update_title();
    }

    fn update_title(&self) {
        let tab = &self.tabs[self.current];
        let title = if tab.title.is_empty() {
            "zeteme".to_string()
        } else {
            format!("{} - zeteme", tab.title)
        };
        if self.tabs.len() > 1 {
            self.window.set_title(&format!(
                "[{}/{}] {title}",
                self.current + 1,
                self.tabs.len()
            ));
        } else {
            self.window.set_title(&title);
        }
    }

    /// Returns false when the last tab was closed and the browser should exit.
    fn close_current(&mut self) -> bool {
        if self.tabs.len() == 1 {
            // Closing the last tab quits, like :q, and keeps it for next time.
            return false;
        }
        let tab = self.tabs.remove(self.current);
        if let Ok(url) = tab.webview.url() {
            self.closed.push(url);
        }
        self.select(self.current.min(self.tabs.len() - 1));
        true
    }

    fn set_zoom(&mut self, zoom: f64) {
        let tab = &mut self.tabs[self.current];
        tab.zoom = zoom.clamp(0.3, 5.0);
        let _ = tab.webview.zoom(tab.zoom);
    }

    fn save_session(&self) {
        let urls: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|tab| tab.webview.url().ok())
            .filter(|url| !url.is_empty())
            .collect();
        let _ = fs::write(&self.session_file, urls.join("\n"));
    }

    /// Returns false when the browser should exit.
    fn run(&mut self, command: Command) -> bool {
        match command {
            Command::Open(url) => {
                let _ = self.tabs[self.current].webview.load_url(&url);
            }
            Command::TabOpen(url) => {
                let _ = self.open_tab(&url);
            }
            Command::Close => return self.close_current(),
            Command::Undo => {
                if let Some(url) = self.closed.pop() {
                    let _ = self.open_tab(&url);
                }
            }
            Command::NextTab => self.select((self.current + 1) % self.tabs.len()),
            Command::PrevTab => self.select((self.current + self.tabs.len() - 1) % self.tabs.len()),
            Command::ZoomIn => self.set_zoom(self.tabs[self.current].zoom + ZOOM_STEP),
            Command::ZoomOut => self.set_zoom(self.tabs[self.current].zoom - ZOOM_STEP),
            Command::ZoomReset => self.set_zoom(1.0),
            Command::Fullscreen(on) => self
                .window
                .set_fullscreen(on.then_some(Fullscreen::Borderless(None))),
            Command::Quit => return false,
        }
        true
    }
}

fn main() -> wry::Result<()> {
    let data_dir = data_dir();
    let _ = fs::create_dir_all(&data_dir);
    let session_file = data_dir.join("session.txt");

    // Restore the last session, then open any URL given on the command line.
    let mut urls: Vec<String> = fs::read_to_string(&session_file)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    if let Some(arg) = std::env::args().nth(1) {
        urls.push(normalize_url(&arg));
    }
    if urls.is_empty() {
        urls.push(DEFAULT_URL.to_string());
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("zeteme")
        .build(&event_loop)
        .expect("failed to create window");

    let mut browser = Browser {
        window,
        context: WebContext::new(Some(data_dir.join("webview"))),
        proxy: event_loop.create_proxy(),
        tabs: Vec::new(),
        current: 0,
        closed: Vec::new(),
        next_id: 0,
        session_file,
    };
    for url in &urls {
        browser.open_tab(url)?;
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        let keep_running = match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => false,
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                let bounds = browser.bounds();
                for tab in &browser.tabs {
                    let _ = tab.webview.set_bounds(bounds);
                }
                true
            }
            Event::WindowEvent {
                event: WindowEvent::Focused(true),
                ..
            } => {
                let _ = browser.tabs[browser.current].webview.focus();
                true
            }
            Event::UserEvent(UserEvent::Command(command)) => browser.run(command),
            Event::UserEvent(UserEvent::NewTab(url)) => browser.run(Command::TabOpen(url)),
            Event::UserEvent(UserEvent::Title(id, title)) => {
                if let Some(tab) = browser.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.title = title;
                }
                browser.update_title();
                // Titles change on every navigation, so this keeps the
                // session file current even if zeteme is killed.
                browser.save_session();
                true
            }
            _ => true,
        };

        if !keep_running {
            browser.save_session();
            *control_flow = ControlFlow::Exit;
        }
    });
}
