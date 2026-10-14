//! The tab bar: a small webview of its own beside or over the page, so pages
//! can't restyle or block it.

use wry::{
    Rect,
    dpi::{PhysicalPosition, PhysicalSize},
};

pub const HTML: &str = include_str!("tabbar.html");

/// How long the bar stays up after switching tabs in `Show::Auto` and
/// `Show::Hover`.
pub const AUTO_HIDE_MS: u64 = 1500;

/// Logical pixels: the height of a bar at the top or bottom, and the width
/// of one at the side.
const HEIGHT: f64 = 22.0;
const WIDTH: f64 = 200.0;
/// Logical pixels of the bar left at the window's edge in `Show::Hover`,
/// for the mouse to find.
const EDGE: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Show {
    /// Over the page, briefly, when tabs are opened, closed or switched.
    Auto,
    /// Like `Auto`, and also while the mouse is at the bar's edge of the window.
    Hover,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Position {
    Top,
    Bottom,
    Left,
    Right,
}

/// A button that can be pinned at the end of the tab bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Downloads,
    History,
    NewTab,
    Bitwarden,
    Settings,
}

impl Button {
    pub const ALL: [Button; 5] = [
        Button::Downloads,
        Button::History,
        Button::NewTab,
        Button::Bitwarden,
        Button::Settings,
    ];

    pub fn parse(text: &str) -> Option<Button> {
        Button::ALL.into_iter().find(|b| b.as_str() == text)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Button::Downloads => "downloads",
            Button::History => "history",
            Button::NewTab => "newtab",
            Button::Bitwarden => "bitwarden",
            Button::Settings => "settings",
        }
    }

    /// Reads a comma-separated list, skipping names it doesn't know.
    pub fn parse_list(text: &str) -> Vec<Button> {
        let mut buttons = Vec::new();
        for button in text
            .split(',')
            .filter_map(|name| Button::parse(name.trim()))
        {
            if !buttons.contains(&button) {
                buttons.push(button);
            }
        }
        buttons
    }

    pub fn list_str(buttons: &[Button]) -> String {
        let names: Vec<&str> = buttons.iter().map(|b| b.as_str()).collect();
        names.join(",")
    }
}

impl Show {
    pub fn parse(text: &str) -> Option<Show> {
        match text {
            "auto" => Some(Show::Auto),
            "hover" => Some(Show::Hover),
            "always" => Some(Show::Always),
            "never" => Some(Show::Never),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Show::Auto => "auto",
            Show::Hover => "hover",
            Show::Always => "always",
            Show::Never => "never",
        }
    }
}

impl Position {
    pub fn parse(text: &str) -> Option<Position> {
        match text {
            "top" => Some(Position::Top),
            "bottom" => Some(Position::Bottom),
            "left" => Some(Position::Left),
            "right" => Some(Position::Right),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Position::Top => "top",
            Position::Bottom => "bottom",
            Position::Left => "left",
            Position::Right => "right",
        }
    }

    pub fn vertical(self) -> bool {
        matches!(self, Position::Left | Position::Right)
    }
}

pub fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        position: PhysicalPosition::new(x, y).into(),
        size: PhysicalSize::new(width, height).into(),
    }
}

/// How much of the bar is on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Extent {
    Full,
    /// Just a strip along the window's edge.
    Edge,
    None,
}

/// Where the bar and the pages go in a window of `width` x `height` physical
/// pixels. With `reserve` the pages make room for the bar; otherwise the bar
/// is drawn over them.
pub fn layout(
    position: Position,
    extent: Extent,
    reserve: bool,
    width: u32,
    height: u32,
    scale: f64,
) -> (Rect, Rect) {
    let (full, limit) = if position.vertical() {
        ((WIDTH * scale) as u32, width / 2)
    } else {
        ((HEIGHT * scale) as u32, height / 2)
    };
    let full = full.min(limit);
    let size = match extent {
        Extent::Full => full,
        Extent::Edge => ((EDGE * scale) as u32).clamp(1, full),
        Extent::None => 0,
    };
    let room = if reserve { full } else { 0 };
    match position {
        Position::Top => (rect(0, 0, width, size), rect(0, room, width, height - room)),
        Position::Bottom => (
            rect(0, height - size, width, size),
            rect(0, 0, width, height - room),
        ),
        Position::Left => (
            rect(0, 0, size, height),
            rect(room, 0, width - room, height),
        ),
        Position::Right => (
            rect(width - size, 0, size, height),
            rect(0, 0, width - room, height),
        ),
    }
}

/// Puts a child window above its siblings, so the bar stays over pages
/// opened after it.
#[cfg(windows)]
pub fn raise(hwnd: *mut std::ffi::c_void) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetWindowPos(
            hwnd: *mut std::ffi::c_void,
            after: *mut std::ffi::c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }
    const SWP_NOSIZE: u32 = 0x1;
    const SWP_NOMOVE: u32 = 0x2;
    const SWP_NOACTIVATE: u32 = 0x10;
    // Queued like wry's own raise of a new page, so it lands after it.
    const SWP_ASYNCWINDOWPOS: u32 = 0x4000;
    // HWND_TOP is null.
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn physical(rect: Rect) -> (i32, i32, u32, u32) {
        let p = rect.position.to_physical::<i32>(1.0);
        let s = rect.size.to_physical::<u32>(1.0);
        (p.x, p.y, s.width, s.height)
    }

    #[test]
    fn lays_out_each_side() {
        let (bar, page) = layout(Position::Top, Extent::Full, true, 1000, 800, 1.0);
        assert_eq!(physical(bar), (0, 0, 1000, 22));
        assert_eq!(physical(page), (0, 22, 1000, 778));

        let (bar, page) = layout(Position::Bottom, Extent::Full, true, 1000, 800, 2.0);
        assert_eq!(physical(bar), (0, 756, 1000, 44));
        assert_eq!(physical(page), (0, 0, 1000, 756));

        let (bar, page) = layout(Position::Left, Extent::Full, true, 1000, 800, 1.0);
        assert_eq!(physical(bar), (0, 0, 200, 800));
        assert_eq!(physical(page), (200, 0, 800, 800));

        let (bar, page) = layout(Position::Right, Extent::Full, false, 1000, 800, 1.0);
        assert_eq!(physical(bar), (800, 0, 200, 800));
        assert_eq!(physical(page), (0, 0, 1000, 800));
    }

    #[test]
    fn shrinks_to_the_edge() {
        let (bar, page) = layout(Position::Right, Extent::Edge, false, 1000, 800, 1.5);
        assert_eq!(physical(bar), (994, 0, 6, 800));
        assert_eq!(physical(page), (0, 0, 1000, 800));

        let (bar, _) = layout(Position::Bottom, Extent::None, false, 1000, 800, 1.0);
        assert_eq!(physical(bar), (0, 800, 1000, 0));
    }

    #[test]
    fn fits_tiny_windows() {
        let (bar, page) = layout(Position::Left, Extent::Full, true, 100, 10, 1.0);
        assert_eq!(physical(bar), (0, 0, 50, 10));
        assert_eq!(physical(page), (50, 0, 50, 10));
    }

    #[test]
    fn parses_names() {
        for show in [Show::Auto, Show::Hover, Show::Always, Show::Never] {
            assert_eq!(Show::parse(show.as_str()), Some(show));
        }
        for position in [
            Position::Top,
            Position::Bottom,
            Position::Left,
            Position::Right,
        ] {
            assert_eq!(Position::parse(position.as_str()), Some(position));
        }
        assert_eq!(Position::parse("middle"), None);
        for button in Button::ALL {
            assert_eq!(Button::parse(button.as_str()), Some(button));
        }
        assert_eq!(
            Button::parse_list(" history, newtab,history,x"),
            vec![Button::History, Button::NewTab]
        );
        assert_eq!(
            Button::list_str(&Button::parse_list("settings,downloads")),
            "settings,downloads"
        );
    }
}
