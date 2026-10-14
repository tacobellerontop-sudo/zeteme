//! Bookmarks, kept as `url<TAB>title` lines in `bookmarks.txt`, newest last.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// The bookmarks page.
pub const PAGE: &str = include_str!("bookmarks.html");

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
}

pub struct Bookmarks {
    file: PathBuf,
    pub list: Vec<Bookmark>,
}

fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

impl Bookmarks {
    pub fn load(file: &Path) -> Bookmarks {
        let list = fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (url, title) = line.split_once('\t').unwrap_or((line, ""));
                Bookmark {
                    url: url.to_string(),
                    title: title.to_string(),
                }
            })
            .collect();
        Bookmarks {
            file: file.to_path_buf(),
            list,
        }
    }

    fn save(&self) {
        let text: String = self
            .list
            .iter()
            .map(|b| format!("{}\t{}\n", clean(&b.url), clean(&b.title)))
            .collect();
        let _ = fs::write(&self.file, text);
    }

    pub fn contains(&self, url: &str) -> bool {
        self.list.iter().any(|b| b.url == url)
    }

    /// Bookmarks `url`, or removes its bookmark; returns whether it is
    /// bookmarked now.
    pub fn toggle(&mut self, url: &str, title: &str) -> bool {
        if self.contains(url) {
            self.remove(url);
            false
        } else {
            self.list.push(Bookmark {
                url: url.to_string(),
                title: title.to_string(),
            });
            self.save();
            true
        }
    }

    pub fn remove(&mut self, url: &str) {
        self.list.retain(|b| b.url != url);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_and_keeps_bookmarks() {
        let file =
            std::env::temp_dir().join(format!("zeteme-bookmarks-{}.txt", std::process::id()));
        let _ = fs::remove_file(&file);
        let mut bookmarks = Bookmarks::load(&file);
        assert!(bookmarks.toggle("https://a.com/", "A\tsite"));
        assert!(bookmarks.toggle("https://b.com/", ""));
        assert!(!bookmarks.toggle("https://a.com/", "A"));
        bookmarks.toggle("https://c.com/", "C");

        let loaded = Bookmarks::load(&file);
        let urls: Vec<&str> = loaded.list.iter().map(|b| b.url.as_str()).collect();
        assert_eq!(urls, ["https://b.com/", "https://c.com/"]);
        assert_eq!(loaded.list[1].title, "C");
        bookmarks.remove("https://b.com/");
        assert_eq!(Bookmarks::load(&file).list.len(), 1);
        let _ = fs::remove_file(&file);
    }
}
