// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod command;
mod settings;
mod tabbar;
mod update;

use std::{borrow::Cow, fs, path::PathBuf, thread, time::Duration};

use command::{Command, normalize_url, parse_ipc};
use settings::{SETTINGS_URL, Settings, js_string};
use tabbar::Show;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Fullscreen, Window, WindowBuilder},
};
use update::Status;
#[cfg(windows)]
use wry::WebViewExtWindows;
use wry::{
    NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder,
    http::{Request, Response},
};

const DEFAULT_URL: &str = "https://duckduckgo.com";
const ZOOM_STEP: f64 = 0.1;

/// Injected into every page; handles the vim keybindings, hints and command bar.
const KEYS_JS: &str = include_str!("keys.js");

enum UserEvent {
    Command(Command),
    Title(u64, String),
    NewTab(String),
    Update(Status),
    UpdateDownloaded(PathBuf),
    SelectTab(usize),
    TabBarReady,
    HideTabBar(u64),
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
    settings: Settings,
    settings_file: PathBuf,
    update: Status,
    tab_bar: WebView,
    /// Whether the bar is up in `Show::Auto`, and which timer may hide it.
    tab_bar_up: bool,
    tab_bar_timer: u64,
    fullscreen: bool,
}

/// Where cookies, logins and the saved session live: `%LOCALAPPDATA%\zeteme`.
fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zeteme")
}

/// Lays out the tab bar and pages in `window`, returning (bar, pages).
fn layout(window: &Window, settings: &Settings, fullscreen: bool) -> (Rect, Rect) {
    let size = window.inner_size();
    let reserve = settings.tabs_show == Show::Always && !fullscreen;
    tabbar::layout(
        settings.tabs_position,
        reserve,
        size.width,
        size.height,
        window.scale_factor(),
    )
}

fn build_tab_bar(
    window: &Window,
    context: &mut WebContext,
    proxy: EventLoopProxy<UserEvent>,
    bounds: Rect,
) -> wry::Result<WebView> {
    WebViewBuilder::new_with_web_context(context)
        .with_html(tabbar::HTML)
        .with_bounds(bounds)
        .with_visible(false)
        .with_ipc_handler(move |req: Request<String>| {
            let msg = req.body();
            let event = if msg == "ready" {
                UserEvent::TabBarReady
            } else if let Some(index) = msg.strip_prefix("select ").and_then(|i| i.parse().ok()) {
                UserEvent::SelectTab(index)
            } else {
                return;
            };
            let _ = proxy.send_event(event);
        })
        .build_as_child(window)
}

impl Browser {
    fn bounds(&self) -> Rect {
        layout(&self.window, &self.settings, self.fullscreen).1
    }

    /// Places the bar and pages for the current settings and window size.
    fn layout(&self) {
        let (bar, pages) = layout(&self.window, &self.settings, self.fullscreen);
        for tab in &self.tabs {
            let _ = tab.webview.set_bounds(pages);
        }
        let _ = self.tab_bar.set_bounds(bar);
        let visible = !self.fullscreen
            && match self.settings.tabs_show {
                Show::Always => true,
                Show::Auto => self.tab_bar_up,
                Show::Never => false,
            };
        let _ = self.tab_bar.set_visible(visible);
        // Pages opened later are stacked above the bar.
        #[cfg(windows)]
        tabbar::raise(self.tab_bar.hwnd().0);
    }

    /// Shows every tab's title on the bar.
    fn draw_tab_bar(&self) {
        let titles: Vec<String> = self
            .tabs
            .iter()
            .map(|tab| {
                if tab.title.is_empty() {
                    js_string(&tab.webview.url().unwrap_or_default())
                } else {
                    js_string(&tab.title)
                }
            })
            .collect();
        let _ = self.tab_bar.evaluate_script(&format!(
            "window.zetemeTabs && zetemeTabs([{}], {}, {})",
            titles.join(","),
            self.current,
            self.settings.tabs_position.vertical()
        ));
    }

    /// In `Show::Auto`, shows the bar for a moment.
    fn flash_tab_bar(&mut self) {
        if self.settings.tabs_show != Show::Auto {
            return;
        }
        self.tab_bar_up = true;
        self.tab_bar_timer += 1;
        let timer = self.tab_bar_timer;
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(tabbar::AUTO_HIDE_MS));
            let _ = proxy.send_event(UserEvent::HideTabBar(timer));
        });
        self.layout();
    }

    fn open_tab(&mut self, url: &str) -> wry::Result<()> {
        let id = self.next_id;
        self.next_id += 1;

        let command_proxy = self.proxy.clone();
        let title_proxy = self.proxy.clone();
        let popup_proxy = self.proxy.clone();
        let settings_file = self.settings_file.clone();
        let bounds = self.bounds();
        let webview = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_url(url)
            .with_bounds(bounds)
            .with_initialization_script(KEYS_JS)
            .with_custom_protocol("zeteme".into(), move |_, _| {
                let page = settings::page(&Settings::load(&settings_file));
                Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(Cow::Owned(page.into_bytes()))
                    .unwrap()
            })
            .with_ipc_handler(move |req: Request<String>| {
                let from_settings = req.uri().to_string().starts_with(SETTINGS_URL);
                match parse_ipc(req.body()) {
                    Some(command) if from_settings || !command.settings_only() => {
                        let _ = command_proxy.send_event(UserEvent::Command(command));
                    }
                    _ => {}
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
        self.draw_tab_bar();
        self.flash_tab_bar();
    }

    fn update_title(&self) {
        let tab = &self.tabs[self.current];
        let mut title = if tab.title.is_empty() {
            "zeteme".to_string()
        } else {
            format!("{} - zeteme", tab.title)
        };
        if let Status::Available(tag) = &self.update {
            title.push_str(&format!(" (update {tag} available, :settings)"));
        }
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

    fn open_settings(&mut self) {
        let open = self.tabs.iter().position(|tab| {
            tab.webview
                .url()
                .is_ok_and(|url| url.starts_with(SETTINGS_URL))
        });
        match open {
            Some(index) => self.select(index),
            None => {
                let _ = self.open_tab(SETTINGS_URL);
            }
        }
    }

    /// Records the update status and shows it on any open settings page.
    fn set_update(&mut self, status: Status) {
        self.update = status;
        let script = settings::status_script(&self.update);
        for tab in &self.tabs {
            if tab
                .webview
                .url()
                .is_ok_and(|url| url.starts_with(SETTINGS_URL))
            {
                let _ = tab.webview.evaluate_script(&script);
            }
        }
        self.update_title();
    }

    fn check_for_update(&mut self) {
        if matches!(self.update, Status::Checking | Status::Downloading(_)) {
            return;
        }
        self.set_update(Status::Checking);
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let status = match update::latest_tag() {
                Ok(tag) if update::is_newer(&tag, update::CURRENT_VERSION) => {
                    Status::Available(tag)
                }
                Ok(_) => Status::UpToDate,
                Err(err) => Status::Failed(err),
            };
            let _ = proxy.send_event(UserEvent::Update(status));
        });
    }

    fn install_update(&mut self) {
        let Status::Available(tag) = self.update.clone() else {
            return;
        };
        self.set_update(Status::Downloading(tag.clone()));
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let event = match update::download(&tag) {
                Ok(path) => UserEvent::UpdateDownloaded(path),
                Err(err) => UserEvent::Update(Status::Failed(err)),
            };
            let _ = proxy.send_event(event);
        });
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
            Command::Fullscreen(on) => {
                self.fullscreen = on;
                self.window
                    .set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
                self.layout();
            }
            Command::Settings => self.open_settings(),
            Command::SettingsReady => self.set_update(self.update.clone()),
            Command::SetCheckUpdates(on) => {
                self.settings.check_updates = on;
                self.settings.save(&self.settings_file);
            }
            Command::SetTabsShow(show) => {
                self.settings.tabs_show = show;
                self.settings.save(&self.settings_file);
                self.layout();
            }
            Command::SetTabsPosition(position) => {
                self.settings.tabs_position = position;
                self.settings.save(&self.settings_file);
                self.draw_tab_bar();
                self.layout();
            }
            Command::CheckUpdate => self.check_for_update(),
            Command::InstallUpdate => self.install_update(),
            Command::Quit => return false,
        }
        true
    }
}

fn main() -> wry::Result<()> {
    let data_dir = data_dir();
    let _ = fs::create_dir_all(&data_dir);
    let session_file = data_dir.join("session.txt");
    let settings_file = data_dir.join("settings.txt");
    let settings = Settings::load(&settings_file);

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

    let mut context = WebContext::new(Some(data_dir.join("webview")));
    let proxy = event_loop.create_proxy();
    let tab_bar = build_tab_bar(
        &window,
        &mut context,
        proxy.clone(),
        layout(&window, &settings, false).0,
    )?;

    let mut browser = Browser {
        window,
        context,
        proxy,
        tabs: Vec::new(),
        current: 0,
        closed: Vec::new(),
        next_id: 0,
        session_file,
        settings,
        settings_file,
        update: Status::Idle,
        tab_bar,
        tab_bar_up: false,
        tab_bar_timer: 0,
        fullscreen: false,
    };
    for url in &urls {
        browser.open_tab(url)?;
    }
    if browser.settings.check_updates {
        browser.check_for_update();
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
                browser.layout();
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
            Event::UserEvent(UserEvent::SelectTab(index)) => {
                if index < browser.tabs.len() {
                    browser.select(index);
                }
                true
            }
            Event::UserEvent(UserEvent::TabBarReady) => {
                browser.draw_tab_bar();
                true
            }
            Event::UserEvent(UserEvent::HideTabBar(timer)) => {
                if timer == browser.tab_bar_timer {
                    browser.tab_bar_up = false;
                    browser.layout();
                }
                true
            }
            Event::UserEvent(UserEvent::Update(status)) => {
                browser.set_update(status);
                true
            }
            Event::UserEvent(UserEvent::UpdateDownloaded(installer)) => {
                // The installer replaces zeteme once it has exited, then
                // starts it again; the session is saved on the way out.
                match update::install(&installer) {
                    Ok(()) => false,
                    Err(err) => {
                        browser.set_update(Status::Failed(err));
                        true
                    }
                }
            }
            Event::UserEvent(UserEvent::Title(id, title)) => {
                if let Some(tab) = browser.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.title = title;
                }
                browser.update_title();
                browser.draw_tab_bar();
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
