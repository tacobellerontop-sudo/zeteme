// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bitwarden;
mod bookmarks;
mod command;
mod downloads;
mod extensions;
mod geometry;
mod history;
mod json;
mod settings;
mod tabbar;
mod update;

use std::{
    borrow::Cow,
    cell::Cell,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use bitwarden::Login;
use bookmarks::Bookmarks;
use command::{Command, DownloadAction, OpenMode, normalize_url, parse_ipc};
use downloads::{Download, Downloads};
use extensions::{Extension, Index, Page};
use geometry::Geometry;
use history::History;
use settings::{BOOKMARKS_URL, DOWNLOADS_URL, HISTORY_URL, SETTINGS_URL, Settings, js_string};
use tabbar::{Button, Extent, Show};
use tao::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Fullscreen, Window, WindowBuilder},
};
#[cfg(windows)]
use tao::{
    platform::windows::{IconExtWindows, WindowBuilderExtWindows},
    window::Icon,
};
use update::Status;
use wry::{
    NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder,
    http::{Request, Response},
};
#[cfg(windows)]
use wry::{WebViewBuilderExtWindows, WebViewExtWindows};

const ZOOM_STEP: f64 = 0.1;
/// What shows before a page has drawn, instead of white: the tab bar's grey.
const BACKGROUND: (u8, u8, u8, u8) = (30, 30, 30, 255);

/// Injected into every page; handles the vim keybindings, hints and command bar.
const KEYS_JS: &str = include_str!("keys.js");
/// The Bitwarden password prompt and login chooser.
const PROMPT_HTML: &str = include_str!("prompt.html");
/// The Bitwarden vault panel `:bw` opens.
const VAULT_HTML: &str = include_str!("vault.html");
/// The address bar `o` opens.
const OPEN_BAR_HTML: &str = include_str!("openbar.html");
/// Pages from the history the address bar suggests from, at most.
const SUGGESTED: usize = 2000;

enum UserEvent {
    Command(Command),
    Title(u64, String),
    NewTab(String),
    Update(Status),
    UpdateDownloaded(PathBuf),
    SelectTab(usize),
    TabBarReady,
    TabBarHover(bool),
    HideTabBar(u64),
    ExtensionsListed(Result<Vec<Extension>, String>),
    ExtensionUnpacked(Result<PathBuf, String>),
    ExtensionAdded(PathBuf, Result<String, String>),
    ExtensionRemoved(String, Result<(), String>),
    ExtensionEnabled(Result<(), String>),
    /// Logins for the tab and page, and whether they are for the vault panel.
    BitwardenLogins(u64, String, bool, Result<Vec<Login>, bitwarden::Error>),
    BitwardenUnlocked(Result<String, bitwarden::Error>),
    /// The whole vault, re-read in the background.
    VaultRefreshed(Result<Vec<Login>, bitwarden::Error>),
    Flash(u64, String),
    /// A message from the Bitwarden prompt or vault panel.
    Prompt(String),
    /// A page finished loading in the tab.
    Loaded(u64),
    Download(u64, downloads::Event),
}

/// What a Bitwarden prompt is for: the tab and page it fills.
struct Pending {
    tab: u64,
    url: String,
    logins: Vec<Login>,
    /// Whether the vault panel opens once the vault is unlocked.
    vault: bool,
}

struct Tab {
    id: u64,
    webview: WebView,
    title: String,
    zoom: f64,
    /// The time and address of the visit last put in the history.
    visit: Option<(u64, String)>,
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
    /// Where the window was when last neither maximized nor minimized.
    geometry: Option<Geometry>,
    geometry_file: PathBuf,
    settings: Settings,
    settings_file: PathBuf,
    update: Status,
    tab_bar: WebView,
    /// Whether the bar is up in `Show::Auto`, and which timer may hide it.
    tab_bar_up: bool,
    tab_bar_timer: u64,
    /// Whether the mouse is on the bar, for `Show::Hover`.
    tab_bar_hover: bool,
    fullscreen: bool,
    extensions: Vec<Extension>,
    extension_index: Index,
    extensions_dir: PathBuf,
    /// What the settings page says about the last extension change.
    extension_status: String,
    /// The unlocked Bitwarden session key, kept only while zeteme runs.
    bitwarden_session: Option<String>,
    /// Every login, read once after unlocking: running `bw` takes a second
    /// or more, so lookups use this and refresh it in the background.
    vault: Option<Vec<Login>>,
    refreshing: bool,
    pending: Option<Pending>,
    prompt: Option<WebView>,
    history: History,
    bookmarks: Bookmarks,
    downloads: Downloads,
    /// When download progress was last shown, so it is shown a few times a
    /// second rather than for every chunk.
    downloads_shown: Cell<Instant>,
    /// Whether quitting was already put off once for downloads going on.
    quit_warned: bool,
}

/// Where cookies, logins and the saved session live: `%LOCALAPPDATA%\zeteme`.
fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zeteme")
}

/// Left by an update, so the restarted zeteme reopens the tabs.
fn updated_marker(data_dir: &Path) -> PathBuf {
    data_dir.join("updated")
}

/// Gives the window the icon zeteme.exe carries (assets/zeteme.rc).
#[cfg(windows)]
fn with_icon(builder: WindowBuilder) -> WindowBuilder {
    let icon = || Icon::from_resource(1, None).ok();
    builder.with_window_icon(icon()).with_taskbar_icon(icon())
}

#[cfg(not(windows))]
fn with_icon(builder: WindowBuilder) -> WindowBuilder {
    builder
}

/// Lays out the tab bar and pages in `window`, returning (bar, pages).
fn layout(window: &Window, settings: &Settings, extent: Extent, fullscreen: bool) -> (Rect, Rect) {
    let size = window.inner_size();
    let reserve = settings.tabs_show == Show::Always && !fullscreen;
    tabbar::layout(
        settings.tabs_position,
        extent,
        reserve,
        size.width,
        size.height,
        window.scale_factor(),
    )
}

/// Lets a webview run browser extensions. WebView2 needs every webview on a
/// profile to agree on this, so the tab bar and all pages get it.
fn with_extensions(builder: WebViewBuilder) -> WebViewBuilder {
    #[cfg(windows)]
    let builder = builder.with_browser_extensions_enabled(true);
    builder
}

/// Turns WebView2's own password manager on or off for `webview`.
#[cfg_attr(not(windows), allow(unused_variables))]
fn set_password_saving(webview: &WebView, on: bool) {
    #[cfg(windows)]
    unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings4;
        use windows_core::Interface;
        if let Ok(settings) = webview
            .webview()
            .Settings()
            .and_then(|s| s.cast::<ICoreWebView2Settings4>())
        {
            let _ = settings.SetIsPasswordAutosaveEnabled(on);
            let _ = settings.SetIsGeneralAutofillEnabled(true);
        }
    }
}

fn build_tab_bar(
    window: &Window,
    context: &mut WebContext,
    proxy: EventLoopProxy<UserEvent>,
    bounds: Rect,
) -> wry::Result<WebView> {
    with_extensions(WebViewBuilder::new_with_web_context(context))
        .with_html(tabbar::HTML)
        .with_bounds(bounds)
        .with_visible(false)
        .with_background_color(BACKGROUND)
        .with_ipc_handler(move |req: Request<String>| {
            let msg = req.body();
            let event = if msg == "ready" {
                UserEvent::TabBarReady
            } else if let Some(on) = msg.strip_prefix("hover ") {
                UserEvent::TabBarHover(on == "on")
            } else if let Some(index) = msg.strip_prefix("select ").and_then(|i| i.parse().ok()) {
                UserEvent::SelectTab(index)
            } else if let Some(button) = msg.strip_prefix("button ").and_then(Button::parse) {
                UserEvent::Command(match button {
                    Button::Downloads => Command::Downloads,
                    Button::History => Command::History,
                    Button::Bookmarks => Command::Bookmarks,
                    Button::NewTab => Command::NewTab,
                    Button::Bitwarden => Command::BitwardenVault,
                    Button::Settings => Command::Settings,
                })
            } else {
                return;
            };
            let _ = proxy.send_event(event);
        })
        .build_as_child(window)
}

impl Browser {
    fn bounds(&self) -> Rect {
        layout(&self.window, &self.settings, Extent::Full, self.fullscreen).1
    }

    /// Places the bar and pages for the current settings and window size.
    fn layout(&self) {
        let up = self.tab_bar_up;
        let extent = match self.settings.tabs_show {
            Show::Always => Extent::Full,
            Show::Auto if up => Extent::Full,
            Show::Hover if up || self.tab_bar_hover => Extent::Full,
            Show::Hover => Extent::Edge,
            Show::Auto | Show::Never => Extent::None,
        };
        let (bar, pages) = layout(&self.window, &self.settings, extent, self.fullscreen);
        for tab in &self.tabs {
            let _ = tab.webview.set_bounds(pages);
        }
        // A bar that is out of the way is shrunk rather than hidden: a hidden
        // webview flashes white while it starts drawing again.
        let _ = self.tab_bar.set_bounds(bar);
        let _ = self
            .tab_bar
            .set_visible(!self.fullscreen && self.settings.tabs_show != Show::Never);
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
        let buttons: Vec<String> = self
            .settings
            .buttons
            .iter()
            .map(|button| js_string(button.as_str()))
            .collect();
        let _ = self.tab_bar.evaluate_script(&format!(
            "window.zetemeTabs && zetemeTabs([{}], {}, {}, [{}])",
            titles.join(","),
            self.current,
            self.settings.tabs_position.vertical(),
            buttons.join(",")
        ));
        self.show_downloads(true);
    }

    /// In `Show::Auto` and `Show::Hover`, shows the bar for a moment.
    fn flash_tab_bar(&mut self) {
        if !matches!(self.settings.tabs_show, Show::Auto | Show::Hover) {
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
        let load_proxy = self.proxy.clone();
        let settings_file = self.settings_file.clone();
        let bounds = self.bounds();
        let webview = with_extensions(WebViewBuilder::new_with_web_context(&mut self.context))
            .with_url(url)
            .with_bounds(bounds)
            .with_initialization_script(KEYS_JS)
            .with_background_color(BACKGROUND)
            .with_custom_protocol("zeteme".into(), move |_, req| {
                // wry hands over zeteme://start/ here even where the page's
                // own address is http://zeteme.start/ (Windows).
                let page = match req.uri().host() {
                    Some("start") => settings::start_page().to_string(),
                    Some("history") => history::PAGE.to_string(),
                    Some("downloads") => downloads::PAGE.to_string(),
                    Some("bookmarks") => bookmarks::PAGE.to_string(),
                    _ => settings::page(&Settings::load(&settings_file)),
                };
                Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(Cow::Owned(page.into_bytes()))
                    .unwrap()
            })
            .with_ipc_handler(move |req: Request<String>| {
                let internal = settings::is_internal(&req.uri().to_string());
                match parse_ipc(req.body()) {
                    Some(command) if internal || !command.internal_only() => {
                        let _ = command_proxy.send_event(UserEvent::Command(command));
                    }
                    _ => {}
                }
            })
            .with_document_title_changed_handler(move |title| {
                let _ = title_proxy.send_event(UserEvent::Title(id, title));
            })
            .with_on_page_load_handler(move |event, _| {
                if let PageLoadEvent::Finished = event {
                    let _ = load_proxy.send_event(UserEvent::Loaded(id));
                }
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
        set_password_saving(&webview, self.settings.save_passwords);
        #[cfg(windows)]
        {
            let proxy = self.proxy.clone();
            let _ = downloads::webview2::watch(&webview, move |download, event| {
                let _ = proxy.send_event(UserEvent::Download(download, event));
            });
        }

        self.tabs.push(Tab {
            id,
            webview,
            title: String::new(),
            zoom: 1.0,
            visit: None,
        });
        self.select(self.tabs.len() - 1);
        Ok(())
    }

    fn select(&mut self, index: usize) {
        self.close_prompt();
        self.current = index;
        // Show the new tab before hiding the old one, so the window behind
        // them never shows through.
        let _ = self.tabs[index].webview.set_visible(true);
        for (i, tab) in self.tabs.iter().enumerate() {
            if i != index {
                let _ = tab.webview.set_visible(false);
            }
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
            return !self.may_quit();
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

    /// Notes where the window is, so it opens there next time.
    fn track_geometry(&mut self) {
        if self.fullscreen || self.window.is_minimized() {
            return;
        }
        if self.window.is_maximized() {
            if let Some(geometry) = &mut self.geometry {
                geometry.maximized = true;
            }
            return;
        }
        let Ok(position) = self.window.outer_position() else {
            return;
        };
        let size = self.window.inner_size();
        self.geometry = Some(Geometry {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            maximized: false,
        });
    }

    /// Switches to the tab showing zeteme's own page at `url`, or opens one.
    fn open_page(&mut self, url: &str) {
        let open = self
            .tabs
            .iter()
            .position(|tab| tab.webview.url().is_ok_and(|u| u.starts_with(url)));
        match open {
            Some(index) => self.select(index),
            None => {
                let _ = self.open_tab(url);
            }
        }
    }

    fn open_settings(&mut self) {
        self.open_page(SETTINGS_URL);
    }

    /// Runs `script` on every open tab showing zeteme's own page at `url`.
    fn eval_page(&self, url: &str, script: &str) {
        for tab in &self.tabs {
            if tab.webview.url().is_ok_and(|u| u.starts_with(url)) {
                let _ = tab.webview.evaluate_script(script);
            }
        }
    }

    /// Runs `script` on every open settings page.
    fn eval_settings(&self, script: &str) {
        self.eval_page(SETTINGS_URL, script);
    }

    /// Puts the tab's page in the history, or gives the visit its title.
    /// `titled` says the tab's title just changed, so it is this page's.
    fn note_visit(&mut self, id: u64, titled: bool) {
        if !self.settings.history {
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        let url = tab.webview.url().unwrap_or_default();
        if !history::worth_keeping(&url) {
            return;
        }
        match &tab.visit {
            Some((time, visited)) if *visited == url => {
                self.history.retitle(*time, &url, &tab.title);
            }
            _ => {
                // Otherwise the title may still be the page before's.
                let title = if titled { tab.title.as_str() } else { "" };
                let time = self.history.add(&url, title);
                tab.visit = Some((time, url));
            }
        }
    }

    /// Lists the visits matching `query` on any open history page.
    fn show_history(&self, query: &str) {
        let rows: Vec<String> = self
            .history
            .search(query)
            .iter()
            .map(|visit| {
                format!(
                    "[{}, {}, {}]",
                    visit.time,
                    js_string(&visit.url),
                    js_string(&visit.title)
                )
            })
            .collect();
        self.eval_page(
            HISTORY_URL,
            &format!(
                "window.zetemeHistory && zetemeHistory([{}], {})",
                rows.join(","),
                js_string(query)
            ),
        );
    }

    /// Shows the downloads on the tab bar's button and any open downloads
    /// page; progress alone only a few times a second.
    fn show_downloads(&self, now: bool) {
        if !now && self.downloads_shown.get().elapsed() < Duration::from_millis(250) {
            return;
        }
        self.downloads_shown.set(Instant::now());
        let (going, done) = self.downloads.summary();
        let _ = self.tab_bar.evaluate_script(&format!(
            "window.zetemeDownloadButton && zetemeDownloadButton({going}, {done}, {})",
            self.downloads.unseen
        ));
        self.eval_page(DOWNLOADS_URL, &self.downloads.script());
    }

    fn download_event(&mut self, id: u64, event: downloads::Event) {
        let tab = self.tabs[self.current].id;
        match event {
            downloads::Event::Started { url, path, total } => {
                let download = Download {
                    id,
                    time: history::now(),
                    url,
                    path,
                    received: 0,
                    total,
                    state: downloads::State::Running,
                };
                self.flash(
                    tab,
                    &format!("Downloading {} (:downloads)", download.name()),
                );
                self.downloads.start(download);
                self.quit_warned = false;
                self.show_downloads(true);
                self.flash_tab_bar();
            }
            downloads::Event::Progress { received, total } => {
                self.downloads.progress(id, received, total);
                self.show_downloads(false);
            }
            downloads::Event::State { state, path } => {
                if let Some(download) = self.downloads.set_state(id, state, Some(path)) {
                    let name = download.name();
                    let note = match &download.state {
                        downloads::State::Done => format!("Downloaded {name}"),
                        downloads::State::Failed(reason, _) => {
                            format!("Download failed: {name} ({reason})")
                        }
                        _ => format!("Canceled {name}"),
                    };
                    self.flash(tab, &note);
                    self.flash_tab_bar();
                }
                self.show_downloads(true);
            }
        }
    }

    /// Quitting cancels downloads, so the first try while any are going on
    /// only says so.
    fn may_quit(&mut self) -> bool {
        let (going, _) = self.downloads.summary();
        if going == 0 || self.quit_warned {
            return true;
        }
        self.quit_warned = true;
        let tab = self.tabs[self.current].id;
        let note = if going == 1 {
            "A download is going on: quit again to cancel it".to_string()
        } else {
            format!("{going} downloads are going on: quit again to cancel them")
        };
        self.flash(tab, &note);
        self.flash_tab_bar();
        false
    }

    fn download_action(&mut self, action: DownloadAction, id: u64) {
        let Some(download) = self.downloads.get(id) else {
            return;
        };
        match action {
            DownloadAction::Open => downloads::open(&download.path),
            DownloadAction::Reveal => downloads::reveal(&download.path),
            DownloadAction::Remove => {
                self.downloads.remove(id);
                self.show_downloads(true);
            }
            #[cfg(windows)]
            DownloadAction::Pause => downloads::webview2::pause(id),
            #[cfg(windows)]
            DownloadAction::Resume => downloads::webview2::resume(id),
            #[cfg(windows)]
            DownloadAction::Cancel => downloads::webview2::cancel(id),
            #[cfg(not(windows))]
            _ => {}
        }
    }

    /// Records the update status and shows it on any open settings page.
    fn set_update(&mut self, status: Status) {
        self.update = status;
        self.eval_settings(&settings::status_script(&self.update));
        self.update_title();
    }

    /// Shows the installed extensions on any open settings page.
    fn show_extensions(&self) {
        let list: Vec<settings::ExtensionRow> = self
            .extensions
            .iter()
            .map(|ext| {
                let folder = self.extension_index.folder(&ext.id);
                let has = |page| {
                    folder
                        .and_then(|f| extensions::page_path(f, page))
                        .is_some()
                };
                settings::ExtensionRow {
                    extension: ext,
                    popup: has(Page::Popup),
                    options: has(Page::Options),
                }
            })
            .collect();
        self.eval_settings(&settings::extensions_script(&list, &self.extension_status));
    }

    fn set_extension_status(&mut self, status: impl Into<String>) {
        self.extension_status = status.into();
        self.show_extensions();
    }

    /// Asks WebView2 for the installed extensions.
    fn refresh_extensions(&self) {
        #[cfg(windows)]
        {
            let proxy = self.proxy.clone();
            extensions::webview2::list(
                &self.tab_bar,
                Box::new(move |result| {
                    let _ = proxy.send_event(UserEvent::ExtensionsListed(result));
                }),
            );
        }
    }

    fn install_extension(&mut self, source: String) {
        if cfg!(not(windows)) {
            self.set_extension_status("Extensions need WebView2, on Windows.");
            return;
        }
        self.set_extension_status("Installing...");
        let dir = self.extensions_dir.clone();
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let result = extensions::unpack(&source, &dir);
            let _ = proxy.send_event(UserEvent::ExtensionUnpacked(result));
        });
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    fn add_extension(&mut self, folder: PathBuf) {
        #[cfg(windows)]
        {
            let proxy = self.proxy.clone();
            let added = folder.clone();
            extensions::webview2::add(
                &self.tab_bar,
                &folder,
                Box::new(move |result| {
                    let _ = proxy.send_event(UserEvent::ExtensionAdded(added, result));
                }),
            );
        }
    }

    fn remove_extension(&mut self, id: String) {
        #[cfg(windows)]
        {
            let proxy = self.proxy.clone();
            let removed = id.clone();
            extensions::webview2::remove(
                &self.tab_bar,
                id,
                Box::new(move |result| {
                    let _ = proxy.send_event(UserEvent::ExtensionRemoved(removed, result));
                }),
            );
        }
        #[cfg(not(windows))]
        let _ = id;
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    fn enable_extension(&mut self, id: String, on: bool) {
        #[cfg(windows)]
        {
            let proxy = self.proxy.clone();
            extensions::webview2::enable(
                &self.tab_bar,
                id,
                on,
                Box::new(move |result| {
                    let _ = proxy.send_event(UserEvent::ExtensionEnabled(result));
                }),
            );
        }
    }

    /// Opens an extension's popup or options page in a new tab, since
    /// zeteme has no toolbar for it to drop down from.
    fn open_extension_page(&mut self, id: &str, page: Page) -> bool {
        let path = self
            .extension_index
            .folder(id)
            .and_then(|folder| extensions::page_path(folder, page));
        match path {
            Some(path) => self.open_tab(&extensions::page_url(id, &path)).is_ok(),
            None => false,
        }
    }

    fn flash(&self, tab: u64, text: &str) {
        if let Some(tab) = self.tabs.iter().find(|t| t.id == tab) {
            let _ = tab.webview.evaluate_script(&format!(
                "window.zetemeFlash && zetemeFlash({})",
                js_string(text)
            ));
        }
    }

    /// Asks Bitwarden for the logins of the current page, to fill one
    /// (Ctrl-Shift-L), or for every login, for the vault panel (`:bw`).
    fn bitwarden(&mut self, vault: bool) {
        let tab = &self.tabs[self.current];
        let (id, url) = (tab.id, tab.webview.url().unwrap_or_default());
        if !vault && !url.starts_with("http") {
            self.flash(id, "Bitwarden fills web pages only");
            return;
        }
        match self.vault.clone() {
            Some(logins) => {
                self.bitwarden_logins(id, url, vault, Ok(logins));
                self.refresh_vault();
            }
            None => self.lookup(id, url, vault),
        }
    }

    /// Reads the whole vault, then carries on with the tab and page.
    fn lookup(&self, tab: u64, url: String, vault: bool) {
        let session = self.bitwarden_session.clone();
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let result = bitwarden::logins(None, session.as_deref());
            let _ = proxy.send_event(UserEvent::BitwardenLogins(tab, url, vault, result));
        });
    }

    /// Re-reads the vault in the background, so logins added elsewhere show
    /// up on the next lookup.
    fn refresh_vault(&mut self) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        let session = self.bitwarden_session.clone();
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let result = bitwarden::logins(None, session.as_deref());
            let _ = proxy.send_event(UserEvent::VaultRefreshed(result));
        });
    }

    fn bitwarden_logins(
        &mut self,
        tab: u64,
        url: String,
        vault: bool,
        result: Result<Vec<Login>, bitwarden::Error>,
    ) {
        if let Ok(logins) = &result {
            self.vault = Some(logins.clone());
        }
        let result = result.map(|logins| {
            if vault {
                logins
            } else {
                let mut here: Vec<Login> = logins
                    .into_iter()
                    .filter(|login| bitwarden::matches(login, &url))
                    .collect();
                here.sort_by_cached_key(|login| login.name.to_lowercase());
                here
            }
        });
        match result {
            Ok(logins) if vault => self.open_vault(tab, url, logins),
            Ok(logins) if logins.is_empty() => self.flash(tab, "No Bitwarden login for this site"),
            Ok(mut logins) if logins.len() == 1 => self.fill(tab, &url, logins.remove(0)),
            Ok(logins) => {
                let items: Vec<String> = logins
                    .iter()
                    .take(9)
                    .map(|login| match login.username.as_str() {
                        "" => login.name.clone(),
                        user => format!("{}  {user}", login.name),
                    })
                    .collect();
                self.pending = Some(Pending {
                    tab,
                    url,
                    logins,
                    vault,
                });
                self.open_prompt("choose", "Bitwarden: which login?", &items);
            }
            Err(bitwarden::Error::Locked) => {
                self.bitwarden_session = None;
                self.vault = None;
                self.pending = Some(Pending {
                    tab,
                    url,
                    logins: Vec::new(),
                    vault,
                });
                self.open_prompt("password", "Bitwarden master password: ", &[]);
            }
            Err(bitwarden::Error::Failed(err)) => self.flash(tab, &format!("Bitwarden: {err}")),
        }
    }

    /// Fills `login` into the tab `tab`, if it still shows the page it was
    /// looked up for.
    fn fill(&mut self, tab: u64, url: &str, login: Login) {
        let Some(webview) = self.tabs.iter().find(|t| t.id == tab).map(|t| &t.webview) else {
            return;
        };
        let now = webview.url().unwrap_or_default();
        if bitwarden::origin(&now) != bitwarden::origin(url) {
            return;
        }
        let _ = webview.evaluate_script(&format!(
            "window.zetemeFill && zetemeFill({}, {})",
            js_string(&login.username),
            js_string(&login.password)
        ));
        if login.has_totp {
            self.copy_totp(tab, login.id);
        }
    }

    fn copy_totp(&self, tab: u64, id: String) {
        let session = self.bitwarden_session.clone();
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let text = match bitwarden::totp(&id, session.as_deref()) {
                Ok(code) if bitwarden::copy(&code) => "Copied the one-time code",
                Ok(_) => "Could not copy the one-time code",
                Err(_) => "Could not get the one-time code",
            };
            let _ = proxy.send_event(UserEvent::Flash(tab, text.to_string()));
        });
    }

    /// Copies `text` and says so on the tab `tab`.
    fn copy(&self, tab: u64, text: String, what: &'static str) {
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let note = if bitwarden::copy(&text) {
                format!("Copied the {what}")
            } else {
                format!("Could not copy the {what}")
            };
            let _ = proxy.send_event(UserEvent::Flash(tab, note));
        });
    }

    /// Shows a webview of zeteme's own over the page, so the page can't read
    /// what is typed or listed in it. Its messages arrive as
    /// `UserEvent::Prompt`.
    fn open_overlay(&mut self, html: String, bounds: Rect) {
        self.close_prompt();
        let proxy = self.proxy.clone();
        let prompt = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_html(html)
            .with_bounds(bounds)
            .with_background_color(BACKGROUND)
            .with_focused(true)
            .with_ipc_handler(move |req: Request<String>| {
                let _ = proxy.send_event(UserEvent::Prompt(req.body().clone()));
            });
        if let Ok(prompt) = with_extensions(prompt).build_as_child(&self.window) {
            let _ = prompt.focus();
            self.prompt = Some(prompt);
        }
    }

    /// The master password prompt or the login chooser, along the bottom of
    /// the window.
    fn open_prompt(&mut self, kind: &str, label: &str, items: &[String]) {
        let items: Vec<String> = items.iter().map(|item| js_string(item)).collect();
        let data = format!(
            "{{\"kind\": {}, \"label\": {}, \"items\": [{}]}}",
            js_string(kind),
            js_string(label),
            items.join(", ")
        );
        let lines = 1 + items.len() as u32;
        let size = self.window.inner_size();
        let height = (((lines * 20 + 4) as f64) * self.window.scale_factor()) as u32;
        let height = height.min(size.height);
        self.open_overlay(
            PROMPT_HTML.replace("{{prompt}}", &data),
            tabbar::rect(0, size.height - height, size.width, height),
        );
    }

    /// The vault panel: the bottom bar grown to list every login, those for
    /// this page first.
    fn open_vault(&mut self, tab: u64, url: String, mut logins: Vec<Login>) {
        logins.sort_by_cached_key(|login| {
            (!bitwarden::matches(login, &url), login.name.to_lowercase())
        });
        let items: Vec<String> = logins
            .iter()
            .map(|login| {
                format!(
                    "[{}, {}, {}, {}, {}]",
                    js_string(&login.name),
                    js_string(&login.username),
                    js_string(
                        &login
                            .uris
                            .first()
                            .map(|u| bitwarden::host(u))
                            .unwrap_or_default()
                    ),
                    bitwarden::matches(login, &url),
                    login.has_totp
                )
            })
            .collect();
        let size = self.window.inner_size();
        let height = ((360.0 * self.window.scale_factor()) as u32).min(size.height / 2);
        let bounds = tabbar::rect(0, size.height - height, size.width, height);
        self.pending = Some(Pending {
            tab,
            url,
            logins,
            vault: true,
        });
        self.open_overlay(
            VAULT_HTML.replace("{{items}}", &format!("[{}]", items.join(", "))),
            bounds,
        );
    }

    /// The address bar, along the bottom of the window like the command bar
    /// it stands in for, with bookmarks and the most visited pages to pick.
    fn open_bar(&mut self, mode: OpenMode) {
        let mut items: Vec<String> = self
            .bookmarks
            .list
            .iter()
            .rev()
            .map(|b| format!("[{}, {}, true]", js_string(&b.url), js_string(&b.title)))
            .collect();
        items.extend(
            self.history
                .ranked(SUGGESTED)
                .into_iter()
                .filter(|(url, _)| !self.bookmarks.contains(url))
                .map(|(url, title)| format!("[{}, {}, false]", js_string(url), js_string(title))),
        );
        let text = match mode {
            OpenMode::Edit => self.tabs[self.current].webview.url().unwrap_or_default(),
            _ => String::new(),
        };
        let data = format!(
            "{{\"mode\": {}, \"text\": {}, \"items\": [{}]}}",
            js_string(if mode == OpenMode::NewTab {
                "tabopen"
            } else {
                "open"
            }),
            js_string(&text),
            items.join(", ")
        );
        let lines = 1 + items.len().min(10) as u32;
        let size = self.window.inner_size();
        let height = (((lines * 20 + 4) as f64) * self.window.scale_factor()) as u32;
        let height = height.min(size.height);
        self.open_overlay(
            OPEN_BAR_HTML.replace("{{data}}", &data),
            tabbar::rect(0, size.height - height, size.width, height),
        );
    }

    /// `M`: bookmarks the current page, or removes its bookmark.
    fn toggle_bookmark(&mut self) {
        let tab = &self.tabs[self.current];
        let (id, url) = (tab.id, tab.webview.url().unwrap_or_default());
        if !history::worth_keeping(&url) {
            self.flash(id, "Only web pages can be bookmarked");
            return;
        }
        let title = tab.title.clone();
        let note = if self.bookmarks.toggle(&url, &title) {
            "Bookmarked (:bookmarks)"
        } else {
            "Removed the bookmark"
        };
        self.flash(id, note);
        self.show_bookmarks();
    }

    /// Lists the bookmarks on any open bookmarks page, newest first.
    fn show_bookmarks(&self) {
        let rows: Vec<String> = self
            .bookmarks
            .list
            .iter()
            .rev()
            .map(|b| format!("[{}, {}]", js_string(&b.url), js_string(&b.title)))
            .collect();
        self.eval_page(
            BOOKMARKS_URL,
            &format!(
                "window.zetemeBookmarks && zetemeBookmarks([{}])",
                rows.join(",")
            ),
        );
    }

    fn close_prompt(&mut self) {
        if self.prompt.take().is_some()
            && let Some(tab) = self.tabs.get(self.current)
        {
            let _ = tab.webview.focus();
        }
    }

    /// Handles what the prompt or vault panel sent.
    fn prompt_message(&mut self, msg: &str) {
        let (action, arg) = msg.split_once(' ').unwrap_or((msg, ""));
        self.close_prompt();
        match action {
            "unlock" => {
                self.prompt_unlock(arg.to_string());
                return;
            }
            "go-open" => {
                self.run(Command::Open(normalize_url(arg)));
                return;
            }
            "go-tabopen" => {
                self.run(Command::TabOpen(normalize_url(arg)));
                return;
            }
            _ => {}
        }
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        let Some(index) = arg.parse().ok().filter(|&i| i < pending.logins.len()) else {
            return;
        };
        let login = pending.logins.swap_remove(index);
        match action {
            "pick" | "fill" => self.fill(pending.tab, &pending.url, login),
            "copy-user" => self.copy(pending.tab, login.username, "username"),
            "copy-pass" => self.copy(pending.tab, login.password, "password"),
            "copy-totp" if login.has_totp => self.copy_totp(pending.tab, login.id),
            "copy-totp" => self.flash(pending.tab, "That login has no one-time code"),
            _ => {}
        }
    }

    fn prompt_unlock(&mut self, password: String) {
        let Some(pending) = &self.pending else {
            return;
        };
        self.flash(pending.tab, "Unlocking Bitwarden...");
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let _ = proxy.send_event(UserEvent::BitwardenUnlocked(bitwarden::unlock(&password)));
        });
    }

    fn bitwarden_unlocked(&mut self, result: Result<String, bitwarden::Error>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        match result {
            Ok(session) => {
                self.bitwarden_session = Some(session);
                // Look the logins up again, now that the vault is open.
                self.lookup(pending.tab, pending.url, pending.vault);
            }
            Err(bitwarden::Error::Locked) => self.flash(pending.tab, "Bitwarden is still locked"),
            Err(bitwarden::Error::Failed(err)) => {
                self.flash(pending.tab, &format!("Bitwarden: {err}"))
            }
        }
    }

    fn bitwarden_lock(&mut self) {
        self.bitwarden_session = None;
        self.vault = None;
        let tab = self.tabs[self.current].id;
        self.flash(tab, "Bitwarden locked");
        thread::spawn(bitwarden::lock);
    }

    /// `:ext <name>`: the popup (or else options) of the first extension
    /// whose name contains `name`; the extension list without one.
    fn open_extension(&mut self, name: &str) {
        let name = name.to_lowercase();
        let found = self
            .extensions
            .iter()
            .find(|ext| !name.is_empty() && ext.name.to_lowercase().contains(&name))
            .map(|ext| ext.id.clone());
        let opened = found.is_some_and(|id| {
            self.open_extension_page(&id, Page::Popup)
                || self.open_extension_page(&id, Page::Options)
        });
        if !opened {
            self.open_settings();
        }
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
            Command::NewTab => {
                let _ = self.open_tab(self.settings.start_page.url());
            }
            Command::Close => return self.close_current(),
            Command::Undo => {
                if let Some(url) = self.closed.pop() {
                    let _ = self.open_tab(&url);
                }
            }
            Command::NextTab => self.select((self.current + 1) % self.tabs.len()),
            Command::PrevTab => self.select((self.current + self.tabs.len() - 1) % self.tabs.len()),
            Command::SelectTab(n) => {
                let last = self.tabs.len() - 1;
                self.select(if n == 9 { last } else { (n - 1).min(last) });
            }
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
            Command::SettingsReady => {
                self.set_update(self.update.clone());
                self.show_extensions();
                self.refresh_extensions();
            }
            Command::SetCheckUpdates(on) => {
                self.settings.check_updates = on;
                self.settings.save(&self.settings_file);
            }
            Command::SetSavePasswords(on) => {
                self.settings.save_passwords = on;
                self.settings.save(&self.settings_file);
                for tab in &self.tabs {
                    set_password_saving(&tab.webview, on);
                }
            }
            Command::Bitwarden => self.bitwarden(false),
            Command::BitwardenVault => self.bitwarden(true),
            Command::BitwardenLock => self.bitwarden_lock(),
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
            Command::SetStartPage(page) => {
                self.settings.start_page = page;
                self.settings.save(&self.settings_file);
            }
            Command::SetReopenTabs(on) => {
                self.settings.reopen_tabs = on;
                self.settings.save(&self.settings_file);
            }
            Command::CheckUpdate => self.check_for_update(),
            Command::InstallUpdate => self.install_update(),
            Command::Extension(name) => self.open_extension(&name),
            Command::ExtensionInstall(source) => self.install_extension(source),
            Command::ExtensionRemove(id) => self.remove_extension(id),
            Command::ExtensionEnable(id, on) => self.enable_extension(id, on),
            Command::ExtensionPage(id, page) => {
                self.open_extension_page(&id, page);
            }
            Command::History => self.open_page(HISTORY_URL),
            Command::HistorySearch(query) => self.show_history(&query),
            Command::HistoryDelete(time, url) => self.history.remove(time, &url),
            Command::HistoryClear => {
                self.history.clear();
                for tab in &mut self.tabs {
                    tab.visit = None;
                }
                self.show_history("");
            }
            Command::SetHistory(on) => {
                self.settings.history = on;
                self.settings.save(&self.settings_file);
            }
            Command::SetButtons(buttons) => {
                self.settings.buttons = buttons;
                self.settings.save(&self.settings_file);
                self.draw_tab_bar();
            }
            Command::OpenBar(mode) => self.open_bar(mode),
            Command::Bookmark => self.toggle_bookmark(),
            Command::Bookmarks => self.open_page(BOOKMARKS_URL),
            Command::BookmarksReady => self.show_bookmarks(),
            Command::BookmarkDelete(url) => self.bookmarks.remove(&url),
            Command::Downloads => {
                self.open_page(DOWNLOADS_URL);
                self.downloads.unseen = false;
                self.show_downloads(true);
            }
            Command::DownloadsReady => {
                self.downloads.unseen = false;
                self.show_downloads(true);
            }
            Command::Download(action, id) => self.download_action(action, id),
            Command::DownloadsClear => {
                self.downloads.clear_finished();
                self.show_downloads(true);
            }
            Command::DownloadsFolder => downloads::reveal_folder(),
            Command::Quit => return !self.may_quit(),
        }
        true
    }
}

fn main() -> wry::Result<()> {
    let data_dir = data_dir();
    let _ = fs::create_dir_all(&data_dir);
    let session_file = data_dir.join("session.txt");
    let settings_file = data_dir.join("settings.txt");
    let geometry_file = data_dir.join("window.txt");
    let extensions_dir = data_dir.join("extensions");
    let _ = fs::create_dir_all(&extensions_dir);
    let history = History::load(&data_dir.join("history.txt"));
    let downloads = Downloads::load(&data_dir.join("downloads.txt"));
    let bookmarks = Bookmarks::load(&data_dir.join("bookmarks.txt"));
    let settings = Settings::load(&settings_file);

    // Restore the last session, then open any URL given on the command line.
    // After an update the tabs always come back, as if zeteme never closed.
    let updated = fs::remove_file(updated_marker(&data_dir)).is_ok();
    let mut urls: Vec<String> = if settings.reopen_tabs || updated {
        fs::read_to_string(&session_file)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
    if let Some(arg) = std::env::args().nth(1) {
        urls.push(normalize_url(&arg));
    }
    if urls.is_empty() {
        urls.push(settings.start_page.url().to_string());
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let mut builder = with_icon(WindowBuilder::new())
        .with_title("zeteme")
        .with_background_color(BACKGROUND);
    // Open as big as, and where, the window was last time.
    let geometry = Geometry::load(&geometry_file);
    if let Some(geometry) = geometry {
        builder = builder
            .with_inner_size(PhysicalSize::new(geometry.width, geometry.height))
            .with_maximized(geometry.maximized);
        let monitors: Vec<_> = event_loop
            .available_monitors()
            .map(|monitor| {
                let (position, size) = (monitor.position(), monitor.size());
                (position.x, position.y, size.width, size.height)
            })
            .collect();
        if geometry.reachable(&monitors) {
            builder = builder.with_position(PhysicalPosition::new(geometry.x, geometry.y));
        }
    }
    let window = builder.build(&event_loop).expect("failed to create window");

    let mut context = WebContext::new(Some(data_dir.join("webview")));
    let proxy = event_loop.create_proxy();
    let tab_bar = build_tab_bar(
        &window,
        &mut context,
        proxy.clone(),
        layout(&window, &settings, Extent::None, false).0,
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
        geometry,
        geometry_file,
        settings,
        settings_file,
        update: Status::Idle,
        tab_bar,
        tab_bar_up: false,
        tab_bar_timer: 0,
        tab_bar_hover: false,
        fullscreen: false,
        extensions: Vec::new(),
        extension_index: Index::load(&extensions_dir),
        extensions_dir,
        extension_status: String::new(),
        bitwarden_session: None,
        vault: None,
        refreshing: false,
        pending: None,
        prompt: None,
        history,
        bookmarks,
        downloads,
        downloads_shown: Cell::new(Instant::now()),
        quit_warned: false,
    };
    browser.track_geometry();
    for url in &urls {
        browser.open_tab(url)?;
    }
    browser.refresh_extensions();
    if browser.settings.check_updates {
        browser.check_for_update();
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        let keep_running = match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => !browser.may_quit(),
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                browser.layout();
                browser.track_geometry();
                true
            }
            Event::WindowEvent {
                event: WindowEvent::Moved(_),
                ..
            } => {
                browser.track_geometry();
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
            Event::UserEvent(UserEvent::TabBarHover(on)) => {
                browser.tab_bar_hover = on;
                browser.layout();
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
                    Ok(()) => {
                        let _ = fs::write(updated_marker(&data_dir), "");
                        false
                    }
                    Err(err) => {
                        browser.set_update(Status::Failed(err));
                        true
                    }
                }
            }
            Event::UserEvent(UserEvent::ExtensionsListed(result)) => {
                match result {
                    Ok(list) => {
                        browser.extensions = list;
                        browser.show_extensions();
                    }
                    Err(err) => {
                        browser.set_extension_status(format!("Could not list extensions: {err}"))
                    }
                }
                true
            }
            Event::UserEvent(UserEvent::ExtensionUnpacked(result)) => {
                match result {
                    Ok(folder) => browser.add_extension(folder),
                    Err(err) => browser.set_extension_status(format!("Install failed: {err}")),
                }
                true
            }
            Event::UserEvent(UserEvent::ExtensionAdded(folder, result)) => {
                match result {
                    Ok(id) => {
                        browser.extension_index.insert(id, folder);
                        browser.set_extension_status("Installed.");
                    }
                    Err(err) => {
                        extensions::discard(&browser.extensions_dir, &folder);
                        browser.set_extension_status(format!("WebView2 would not load it: {err}"));
                    }
                }
                browser.refresh_extensions();
                true
            }
            Event::UserEvent(UserEvent::ExtensionRemoved(id, result)) => {
                match result {
                    Ok(()) => {
                        if let Some(folder) = browser.extension_index.remove(&id) {
                            extensions::discard(&browser.extensions_dir, &folder);
                        }
                        browser.set_extension_status("Removed.");
                    }
                    Err(err) => browser.set_extension_status(format!("Remove failed: {err}")),
                }
                browser.refresh_extensions();
                true
            }
            Event::UserEvent(UserEvent::ExtensionEnabled(result)) => {
                if let Err(err) = result {
                    browser.set_extension_status(format!("Could not switch it: {err}"));
                }
                browser.refresh_extensions();
                true
            }
            Event::UserEvent(UserEvent::BitwardenLogins(tab, url, vault, result)) => {
                browser.bitwarden_logins(tab, url, vault, result);
                true
            }
            Event::UserEvent(UserEvent::BitwardenUnlocked(result)) => {
                browser.bitwarden_unlocked(result);
                true
            }
            Event::UserEvent(UserEvent::Flash(tab, text)) => {
                browser.flash(tab, &text);
                true
            }
            Event::UserEvent(UserEvent::VaultRefreshed(result)) => {
                browser.refreshing = false;
                match result {
                    // Unless it was locked meanwhile.
                    Ok(logins) if browser.vault.is_some() => browser.vault = Some(logins),
                    Ok(_) => {}
                    Err(bitwarden::Error::Locked) => browser.vault = None,
                    Err(_) => {}
                }
                true
            }
            Event::UserEvent(UserEvent::Prompt(msg)) => {
                browser.prompt_message(&msg);
                true
            }
            Event::UserEvent(UserEvent::Loaded(id)) => {
                browser.note_visit(id, false);
                true
            }
            Event::UserEvent(UserEvent::Download(id, event)) => {
                browser.download_event(id, event);
                true
            }
            Event::UserEvent(UserEvent::Title(id, title)) => {
                if let Some(tab) = browser.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.title = title;
                }
                browser.note_visit(id, true);
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
            if let Some(geometry) = &browser.geometry {
                geometry.save(&browser.geometry_file);
            }
            *control_flow = ControlFlow::Exit;
        }
    });
}
