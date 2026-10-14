//! The pages visited, kept as `time<TAB>url<TAB>title` lines in
//! `history.txt`. The file is only ever appended to while zeteme runs: a
//! page whose title arrives later gets a second line with the same time and
//! address, which replaces the first when the file is read back.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// The history page.
pub const PAGE: &str = include_str!("history.html");

/// Visits kept; older ones are dropped when zeteme starts.
const MAX: usize = 20_000;
/// Visits the history page shows for one search.
pub const SHOWN: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct Visit {
    /// Seconds since 1970.
    pub time: u64,
    pub url: String,
    pub title: String,
}

pub struct History {
    file: PathBuf,
    /// Oldest first.
    pub visits: Vec<Visit>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether visiting `url` goes into the history: web pages, not zeteme's
/// own pages or blank ones.
pub fn worth_keeping(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && !crate::settings::is_internal(url)
}

/// Tabs and line breaks would split a line of the file.
fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

fn line(visit: &Visit) -> String {
    format!(
        "{}\t{}\t{}\n",
        visit.time,
        clean(&visit.url),
        clean(&visit.title)
    )
}

fn parse(text: &str) -> Vec<Visit> {
    let mut visits: Vec<Visit> = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(time), Some(url)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(time) = time.parse() else {
            continue;
        };
        let title = parts.next().unwrap_or("").to_string();
        // A later title for the same visit is usually right behind it.
        match visits
            .iter_mut()
            .rev()
            .take(32)
            .find(|v| v.time == time && v.url == url)
        {
            Some(visit) => visit.title = title,
            None => visits.push(Visit {
                time,
                url: url.to_string(),
                title,
            }),
        }
    }
    visits
}

impl History {
    pub fn load(file: &Path) -> History {
        let text = fs::read_to_string(file).unwrap_or_default();
        let mut visits = parse(&text);
        let lines = text.lines().count();
        if visits.len() > MAX {
            visits.drain(..visits.len() - MAX);
        }
        let history = History {
            file: file.to_path_buf(),
            visits,
        };
        // Rewrite the file once it carries many replaced titles or old visits.
        if lines > history.visits.len() + history.visits.len() / 2 + 100 {
            history.save();
        }
        history
    }

    fn save(&self) {
        let text: String = self.visits.iter().map(line).collect();
        let _ = fs::write(&self.file, text);
    }

    fn append(&self, visit: &Visit) {
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
        {
            let _ = file.write_all(line(visit).as_bytes());
        }
    }

    /// Records a visit to `url` and returns its time, which `retitle` takes.
    pub fn add(&mut self, url: &str, title: &str) -> u64 {
        let visit = Visit {
            time: now(),
            url: url.to_string(),
            title: title.to_string(),
        };
        self.append(&visit);
        let time = visit.time;
        self.visits.push(visit);
        time
    }

    /// Gives the visit at `time` to `url` a new title.
    pub fn retitle(&mut self, time: u64, url: &str, title: &str) {
        let Some(visit) = self
            .visits
            .iter_mut()
            .rev()
            .find(|v| v.time == time && v.url == url)
        else {
            return;
        };
        if visit.title != title {
            visit.title = title.to_string();
            let visit = visit.clone();
            self.append(&visit);
        }
    }

    /// The newest visits whose address or title contains every word of
    /// `query`, ignoring case.
    pub fn search(&self, query: &str) -> Vec<&Visit> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.visits
            .iter()
            .rev()
            .filter(|visit| {
                let text = format!("{} {}", visit.url, visit.title).to_lowercase();
                words.iter().all(|word| text.contains(word))
            })
            .take(SHOWN)
            .collect()
    }

    pub fn remove(&mut self, time: u64, url: &str) {
        let before = self.visits.len();
        self.visits.retain(|v| !(v.time == time && v.url == url));
        if self.visits.len() != before {
            self.save();
        }
    }

    pub fn clear(&mut self) {
        self.visits.clear();
        let _ = fs::remove_file(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("zeteme-{name}-{}.txt", std::process::id()))
    }

    #[test]
    fn keeps_web_pages_only() {
        assert!(worth_keeping("https://example.com/"));
        assert!(!worth_keeping("about:blank"));
        assert!(!worth_keeping(crate::settings::SETTINGS_URL));
        assert!(!worth_keeping("chrome-extension://abc/popup.html"));
    }

    #[test]
    fn later_titles_replace_earlier_ones() {
        let visits = parse("5\thttps://a/\t\n5\thttps://a/\tA page\n6\thttps://b/\tB\njunk\n");
        assert_eq!(visits.len(), 2);
        assert_eq!(visits[0].title, "A page");
        assert_eq!(visits[1].url, "https://b/");
    }

    #[test]
    fn records_searches_and_removes() {
        let file = temp("history");
        let _ = fs::remove_file(&file);
        let mut history = History::load(&file);
        let time = history.add("https://rust-lang.org/", "");
        history.retitle(time, "https://rust-lang.org/", "Rust\tProgramming Language");
        history.add("https://example.com/", "Example");

        let loaded = History::load(&file);
        assert_eq!(loaded.visits.len(), 2);
        assert_eq!(loaded.visits[0].title, "Rust Programming Language");
        let found = loaded.search("RUST language");
        assert_eq!(found.len(), 1);
        assert_eq!(loaded.search("").len(), 2);
        assert_eq!(loaded.search("")[0].url, "https://example.com/");

        history.remove(time, "https://rust-lang.org/");
        assert_eq!(History::load(&file).visits.len(), 1);
        history.clear();
        assert!(History::load(&file).visits.is_empty());
        let _ = fs::remove_file(&file);
    }
}
