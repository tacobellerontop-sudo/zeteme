//! The window's size and place, kept in `window.txt` so zeteme opens where
//! it was last closed.

use std::{fs, path::Path};

/// Smaller than this was a mistake, or a window squeezed by a monitor that's
/// gone; zeteme opens at its default size instead.
const MIN_WIDTH: u32 = 200;
const MIN_HEIGHT: u32 = 150;

/// In physical pixels. `x`, `y`, `width` and `height` are the window when it
/// is neither maximized nor minimized, so un-maximizing returns to them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Geometry {
    pub fn load(file: &Path) -> Option<Geometry> {
        Geometry::parse(&fs::read_to_string(file).ok()?)
    }

    fn parse(text: &str) -> Option<Geometry> {
        let (mut x, mut y, mut width, mut height, mut maximized) = (None, None, None, None, false);
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "x" => x = value.parse().ok(),
                "y" => y = value.parse().ok(),
                "width" => width = value.parse().ok(),
                "height" => height = value.parse().ok(),
                "maximized" => maximized = value == "true",
                _ => {}
            }
        }
        let geometry = Geometry {
            x: x?,
            y: y?,
            width: width?,
            height: height?,
            maximized,
        };
        (geometry.width >= MIN_WIDTH && geometry.height >= MIN_HEIGHT).then_some(geometry)
    }

    pub fn save(&self, file: &Path) {
        let _ = fs::write(
            file,
            format!(
                "x={}\ny={}\nwidth={}\nheight={}\nmaximized={}\n",
                self.x, self.y, self.width, self.height, self.maximized
            ),
        );
    }

    /// Whether enough of the title bar would be on one of `monitors` (each
    /// x, y, width, height) to grab it; if not, Windows should place it.
    pub fn reachable(&self, monitors: &[(i32, i32, u32, u32)]) -> bool {
        let (left, top) = (self.x as i64, self.y as i64);
        let (right, bottom) = (left + self.width as i64, top + 32);
        monitors.iter().any(|&(x, y, width, height)| {
            let (x, y) = (x as i64, y as i64);
            let overlap_x = right.min(x + width as i64) - left.max(x);
            let overlap_y = bottom.min(y + height as i64) - top.max(y);
            overlap_x >= 100 && overlap_y >= 16
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(Geometry::parse(""), None);
        assert_eq!(
            Geometry::parse("x=-8\ny=10\nwidth=1280\nheight=800\nmaximized=true\n"),
            Some(Geometry {
                x: -8,
                y: 10,
                width: 1280,
                height: 800,
                maximized: true,
            })
        );
        assert_eq!(Geometry::parse("x=0\ny=0\nwidth=1280"), None);
        assert_eq!(Geometry::parse("x=0\ny=0\nwidth=0\nheight=0"), None);
    }

    #[test]
    fn saves_and_loads() {
        let geometry = Geometry {
            x: 100,
            y: 50,
            width: 1000,
            height: 700,
            maximized: false,
        };
        let file = std::env::temp_dir().join(format!("zeteme-window-{}.txt", std::process::id()));
        geometry.save(&file);
        let loaded = Geometry::load(&file);
        let _ = fs::remove_file(&file);
        assert_eq!(loaded, Some(geometry));
    }

    #[test]
    fn finds_reachable_windows() {
        let monitors = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
        let at = |x, y| Geometry {
            x,
            y,
            width: 800,
            height: 600,
            maximized: false,
        };
        assert!(at(100, 100).reachable(&monitors));
        assert!(at(3000, 200).reachable(&monitors));
        // On a monitor that has been unplugged.
        assert!(!at(-2000, 100).reachable(&monitors));
        assert!(!at(100, 1400).reachable(&monitors));
        // Mostly off the left edge, but the title bar's end can be grabbed.
        assert!(at(-600, 0).reachable(&monitors));
    }
}
