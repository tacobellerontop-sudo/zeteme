//! Browser extensions, run by WebView2 itself: Chromium extensions load
//! unpacked from `%LOCALAPPDATA%\zeteme\extensions`, and WebView2 keeps them
//! installed in the profile from then on. zeteme only fetches and unpacks
//! them, and opens their popup and options pages in tabs, since it has no
//! toolbar to put their buttons on.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::update::curl;

/// Which Chrome version to tell the Chrome Web Store we are, so it serves
/// current extensions.
const CHROME_VERSION: &str = "140.0.0.0";

#[derive(Clone, Debug, PartialEq)]
pub struct Extension {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Page {
    Popup,
    Options,
}

impl Page {
    pub fn parse(text: &str) -> Option<Page> {
        match text {
            "popup" => Some(Page::Popup),
            "options" => Some(Page::Options),
            _ => None,
        }
    }
}

/// Which extension folder each installed extension was loaded from, kept as
/// `id<TAB>folder` lines in `index.txt`. WebView2 reports only an
/// extension's id and name; the folder has its manifest.
pub struct Index {
    file: PathBuf,
    pub entries: Vec<(String, PathBuf)>,
}

impl Index {
    pub fn load(dir: &Path) -> Index {
        let file = dir.join("index.txt");
        let entries = fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(id, folder)| (id.to_string(), dir.join(folder)))
            .collect();
        Index { file, entries }
    }

    pub fn folder(&self, id: &str) -> Option<&Path> {
        self.entries
            .iter()
            .find(|(entry, _)| entry == id)
            .map(|(_, folder)| folder.as_path())
    }

    pub fn insert(&mut self, id: String, folder: PathBuf) {
        self.entries.retain(|(entry, _)| *entry != id);
        self.entries.push((id, folder));
        self.save();
    }

    /// Forgets `id` and returns the folder it was loaded from.
    pub fn remove(&mut self, id: &str) -> Option<PathBuf> {
        let index = self.entries.iter().position(|(entry, _)| entry == id)?;
        let (_, folder) = self.entries.remove(index);
        self.save();
        Some(folder)
    }

    fn save(&self) {
        let dir = self.file.parent().unwrap_or(Path::new("."));
        let lines: Vec<String> = self
            .entries
            .iter()
            .map(|(id, folder)| {
                let folder = folder.strip_prefix(dir).unwrap_or(folder);
                format!("{id}\t{}", folder.display())
            })
            .collect();
        let _ = fs::write(&self.file, lines.join("\n"));
    }
}

/// Where to download an extension from, for a Chrome Web Store, Edge Add-ons
/// or Firefox Add-ons link (or a bare Chrome Web Store id).
fn download_url(source: &str) -> Option<String> {
    let source = source.trim().trim_end_matches('/');
    let path = source.split(['?', '#']).next().unwrap_or(source);
    let store_id = path.rsplit('/').next().filter(|id| is_store_id(id));
    if !source.contains("://") {
        return store_id.map(chrome_store_url);
    }
    if source.contains("://chromewebstore.google.com/") || source.contains("://chrome.google.com/")
    {
        store_id.map(chrome_store_url)
    } else if source.contains("://microsoftedge.microsoft.com/") {
        store_id.map(|id| {
            format!(
                "https://edge.microsoft.com/extensionwebstorebase/v1/crx?response=redirect\
                 &x=id%3D{id}%26installsource%3Dondemand%26uc"
            )
        })
    } else if source.contains("://addons.mozilla.org/") {
        let slug = path.rsplit_once("/addon/")?.1.split('/').next()?;
        (!slug.is_empty()).then(|| {
            format!("https://addons.mozilla.org/firefox/downloads/latest/{slug}/latest.xpi")
        })
    } else {
        None
    }
}

fn chrome_store_url(id: &str) -> String {
    format!(
        "https://clients2.google.com/service/update2/crx?response=redirect\
         &prodversion={CHROME_VERSION}&acceptformat=crx2,crx3\
         &x=id%3D{id}%26installsource%3Dondemand%26uc"
    )
}

/// Chromium extension ids are 32 letters from a to p.
fn is_store_id(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// The zip archive inside a `.crx` file, or the whole file if it is a plain
/// zip (`.zip`, `.xpi`).
fn zip_part(bytes: &[u8]) -> Result<&[u8], String> {
    if !bytes.starts_with(b"Cr24") {
        return Ok(bytes);
    }
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let start = match word(4) {
        // Magic, version, header length, header.
        Some(3) => word(8).map(|header| 12 + header),
        // Magic, version, key length, signature length, key, signature.
        Some(2) => word(8).zip(word(12)).map(|(key, sig)| 16 + key + sig),
        _ => None,
    };
    start
        .and_then(|start| bytes.get(start..))
        .ok_or_else(|| "not a valid .crx file".to_string())
}

/// The folder holding `manifest.json`: the top of the archive, or its only
/// folder.
fn manifest_root(dir: &Path) -> Option<PathBuf> {
    if dir.join("manifest.json").is_file() {
        return Some(dir.to_path_buf());
    }
    let mut entries = fs::read_dir(dir).ok()?.filter_map(Result::ok);
    let only = entries.next()?.path();
    (entries.next().is_none() && only.join("manifest.json").is_file()).then_some(only)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn extract(archive: &Path, to: &Path) -> Result<(), String> {
    // Windows' own bsdtar reads zip files; a GNU tar earlier on PATH (from
    // Git, say) would not.
    let tar = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("tar.exe"))
        .unwrap_or_else(|| PathBuf::from("tar"));
    let mut command = Command::new(tar);
    command.arg("-xf").arg(archive).arg("-C").arg(to);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|err| format!("could not run tar: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "could not unpack the extension: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Fetches `source` (a store link, an extension folder, or a `.crx`, `.zip`
/// or `.xpi` file) into a new folder under `dir`, and returns the folder
/// with its `manifest.json`. Blocks, so it runs off the UI thread.
pub fn unpack(source: &str, dir: &Path) -> Result<PathBuf, String> {
    let source = source.trim().trim_matches('"');
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_millis());
    let folder = dir.join(format!("ext-{stamp}"));
    let result = unpack_into(source, &folder);
    if result.is_err() {
        let _ = fs::remove_dir_all(&folder);
    }
    result
}

fn unpack_into(source: &str, folder: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(folder).map_err(|err| err.to_string())?;
    let local = Path::new(source);
    if local.is_dir() {
        // Copied, because WebView2 drops an extension whose files change.
        let root = manifest_root(local).ok_or("that folder has no manifest.json")?;
        copy_dir(&root, folder).map_err(|err| format!("could not copy the extension: {err}"))?;
        return Ok(folder.to_path_buf());
    }
    let bytes = if local.is_file() {
        fs::read(local).map_err(|err| err.to_string())?
    } else {
        let url = download_url(source).ok_or(
            "give a Chrome Web Store, Edge Add-ons or Firefox Add-ons link, \
             or an extension folder or .crx, .zip or .xpi file",
        )?;
        let download = folder.with_extension("download");
        let result = curl(&[
            "--fail",
            "--location",
            "--max-time",
            "300",
            "--output",
            &download.to_string_lossy(),
            &url,
        ])
        .and_then(|_| fs::read(&download).map_err(|err| err.to_string()));
        let _ = fs::remove_file(&download);
        result.map_err(|err| format!("download failed: {err}"))?
    };
    let archive = folder.with_extension("zip");
    fs::write(&archive, zip_part(&bytes)?).map_err(|err| err.to_string())?;
    let result = extract(&archive, folder);
    let _ = fs::remove_file(&archive);
    result?;
    let root = manifest_root(folder).ok_or("the archive has no manifest.json")?;
    // Store signatures; Chromium refuses unpacked extensions with names
    // starting with "_" other than its own "_locales".
    let _ = fs::remove_dir_all(root.join("_metadata"));
    Ok(root)
}

/// Deletes the folder under `dir` that `folder` was unpacked into.
pub fn discard(dir: &Path, folder: &Path) {
    let top = folder
        .strip_prefix(dir)
        .ok()
        .and_then(|rest| rest.components().next());
    if let Some(top) = top {
        let _ = fs::remove_dir_all(dir.join(top));
    }
}

/// The path of an extension's popup or options page, from its manifest.
pub fn page_path(folder: &Path, page: Page) -> Option<String> {
    let manifest = fs::read_to_string(folder.join("manifest.json")).ok()?;
    let path = match page {
        Page::Popup => json_string(&manifest, "default_popup"),
        Page::Options => json_string(&manifest, "options_page").or_else(|| {
            let ui = &manifest[manifest.find("\"options_ui\"")?..];
            json_string(ui, "page")
        }),
    }?;
    Some(path.trim_start_matches('/').to_string())
}

/// The string value of the first `"key": "value"` in `json`. Enough for the
/// few manifest keys zeteme reads, without a JSON parser.
fn json_string(json: &str, key: &str) -> Option<String> {
    let quoted = format!("\"{key}\"");
    let mut rest = json;
    while let Some(at) = rest.find(&quoted) {
        rest = &rest[at + quoted.len()..];
        let Some(value) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let value = value.trim_start().strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => out.push(chars.next()?),
                c => out.push(c),
            }
        }
        return None;
    }
    None
}

/// The extension's own URL for `path`.
pub fn page_url(id: &str, path: &str) -> String {
    format!("chrome-extension://{id}/{path}")
}

/// The WebView2 side: installing, listing and switching extensions through
/// the profile every zeteme webview shares. Each call reports back through
/// `done` exactly once, on the UI thread.
#[cfg(windows)]
pub mod webview2 {
    use std::{cell::Cell, path::Path, rc::Rc};

    use webview2_com::{
        BrowserExtensionEnableCompletedHandler, BrowserExtensionRemoveCompletedHandler,
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_13, ICoreWebView2BrowserExtension, ICoreWebView2Profile7,
        },
        ProfileAddBrowserExtensionCompletedHandler, ProfileGetBrowserExtensionsCompletedHandler,
        take_pwstr,
    };
    use windows_core::{BOOL, HSTRING, Interface, PWSTR};
    use wry::{WebView, WebViewExtWindows};

    use super::Extension;

    pub type Done<T> = Box<dyn FnOnce(Result<T, String>)>;

    /// `done`, shared between a completion handler and the error path for a
    /// call WebView2 refuses outright, whichever comes first.
    struct Once<T>(Rc<Cell<Option<Done<T>>>>);

    impl<T> Clone for Once<T> {
        fn clone(&self) -> Self {
            Once(self.0.clone())
        }
    }

    impl<T> Once<T> {
        fn new(done: Done<T>) -> Self {
            Once(Rc::new(Cell::new(Some(done))))
        }

        fn call(&self, result: windows_core::Result<T>) {
            if let Some(done) = self.0.take() {
                done(result.map_err(|err| err.message()));
            }
        }
    }

    fn profile(webview: &WebView) -> windows_core::Result<ICoreWebView2Profile7> {
        unsafe {
            webview
                .webview()
                .cast::<ICoreWebView2_13>()?
                .Profile()?
                .cast::<ICoreWebView2Profile7>()
        }
    }

    fn describe(extension: &ICoreWebView2BrowserExtension) -> windows_core::Result<Extension> {
        unsafe {
            let mut id = PWSTR::null();
            extension.Id(&mut id)?;
            let mut name = PWSTR::null();
            extension.Name(&mut name)?;
            let mut enabled = BOOL::default();
            extension.IsEnabled(&mut enabled)?;
            Ok(Extension {
                id: take_pwstr(id),
                name: take_pwstr(name),
                enabled: enabled.as_bool(),
            })
        }
    }

    type Installed = Vec<(Extension, ICoreWebView2BrowserExtension)>;

    fn installed(webview: &WebView, done: Done<Installed>) {
        let done = Once::new(done);
        let handler = ProfileGetBrowserExtensionsCompletedHandler::create(Box::new({
            let done = done.clone();
            move |result, list| {
                let read = || -> windows_core::Result<Installed> {
                    result?;
                    let mut out = Vec::new();
                    if let Some(list) = list {
                        let mut count = 0;
                        unsafe { list.Count(&mut count)? };
                        for i in 0..count {
                            let extension = unsafe { list.GetValueAtIndex(i)? };
                            out.push((describe(&extension)?, extension));
                        }
                    }
                    Ok(out)
                };
                done.call(read());
                Ok(())
            }
        }));
        if let Err(err) = profile(webview).and_then(|p| unsafe { p.GetBrowserExtensions(&handler) })
        {
            done.call(Err(err));
        }
    }

    pub fn list(webview: &WebView, done: Done<Vec<Extension>>) {
        installed(
            webview,
            Box::new(move |result| {
                done(result.map(|all| all.into_iter().map(|(ext, _)| ext).collect()))
            }),
        );
    }

    /// Installs the unpacked extension in `folder`, reporting its id.
    pub fn add(webview: &WebView, folder: &Path, done: Done<String>) {
        let done = Once::new(done);
        let handler = ProfileAddBrowserExtensionCompletedHandler::create(Box::new({
            let done = done.clone();
            move |result, extension| {
                done.call(result.and_then(|()| match extension {
                    Some(extension) => describe(&extension).map(|ext| ext.id),
                    None => Err(windows_core::Error::empty()),
                }));
                Ok(())
            }
        }));
        let path = HSTRING::from(folder.as_os_str());
        if let Err(err) =
            profile(webview).and_then(|p| unsafe { p.AddBrowserExtension(&path, &handler) })
        {
            done.call(Err(err));
        }
    }

    /// Runs `action` on the installed extension `id`.
    fn with_extension(
        webview: &WebView,
        id: String,
        done: Done<()>,
        action: impl FnOnce(&ICoreWebView2BrowserExtension, Once<()>) -> windows_core::Result<()>
        + 'static,
    ) {
        installed(
            webview,
            Box::new(move |result| {
                let done = Once::new(done);
                let found = result.and_then(|all| {
                    all.into_iter()
                        .find(|(ext, _)| ext.id == id)
                        .ok_or_else(|| "that extension is not installed".to_string())
                });
                match found {
                    Ok((_, extension)) => {
                        if let Err(err) = action(&extension, done.clone()) {
                            done.call(Err(err));
                        }
                    }
                    Err(err) => {
                        if let Some(done) = done.0.take() {
                            done(Err(err));
                        }
                    }
                }
            }),
        );
    }

    pub fn remove(webview: &WebView, id: String, done: Done<()>) {
        with_extension(webview, id, done, |extension, done| {
            let handler = BrowserExtensionRemoveCompletedHandler::create(Box::new(move |result| {
                done.call(result);
                Ok(())
            }));
            unsafe { extension.Remove(&handler) }
        });
    }

    pub fn enable(webview: &WebView, id: String, on: bool, done: Done<()>) {
        with_extension(webview, id, done, move |extension, done| {
            let handler = BrowserExtensionEnableCompletedHandler::create(Box::new(move |result| {
                done.call(result);
                Ok(())
            }));
            unsafe { extension.Enable(on, &handler) }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "ddkjiahejlhfcafbddmgiahcphecmpfh";

    #[test]
    fn finds_store_downloads() {
        let chrome = download_url(&format!(
            "https://chromewebstore.google.com/detail/ublock-origin-lite/{ID}?hl=en"
        ))
        .unwrap();
        assert!(chrome.starts_with("https://clients2.google.com/"));
        assert!(chrome.contains(&format!("id%3D{ID}%26")));
        assert_eq!(download_url(ID), Some(chrome));
        assert!(
            download_url(&format!(
                "https://microsoftedge.microsoft.com/addons/detail/ublock-origin-lite/{ID}"
            ))
            .unwrap()
            .starts_with("https://edge.microsoft.com/")
        );
        assert_eq!(
            download_url("https://addons.mozilla.org/en-US/firefox/addon/ublock-origin/"),
            Some(
                "https://addons.mozilla.org/firefox/downloads/latest/ublock-origin/latest.xpi"
                    .into()
            )
        );
        assert_eq!(download_url("https://example.com/extension"), None);
        assert_eq!(download_url("not-an-id"), None);
    }

    #[test]
    fn strips_crx_headers() {
        let zip = b"PK\x03\x04rest";
        assert_eq!(zip_part(zip).unwrap(), zip);
        let mut crx3 = b"Cr24\x03\0\0\0\x02\0\0\0hh".to_vec();
        crx3.extend_from_slice(zip);
        assert_eq!(zip_part(&crx3).unwrap(), zip);
        let mut crx2 = b"Cr24\x02\0\0\0\x01\0\0\0\x02\0\0\0kss".to_vec();
        crx2.extend_from_slice(zip);
        assert_eq!(zip_part(&crx2).unwrap(), zip);
        assert!(zip_part(b"Cr24\x03\0\0\0\xff\0\0\0").is_err());
    }

    #[test]
    fn reads_manifest_pages() {
        let dir = std::env::temp_dir().join(format!("zeteme-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("manifest.json"),
            r#"{ "name": "x", "action": { "default_title": "a \"b\"",
                 "default_popup": "/popup/index.html" },
                 "options_ui": { "open_in_tab": true, "page": "options.html" } }"#,
        )
        .unwrap();
        assert_eq!(
            page_path(&dir, Page::Popup),
            Some("popup/index.html".into())
        );
        assert_eq!(page_path(&dir, Page::Options), Some("options.html".into()));
        fs::write(dir.join("manifest.json"), r#"{"options_page":"o.html"}"#).unwrap();
        assert_eq!(page_path(&dir, Page::Popup), None);
        assert_eq!(page_path(&dir, Page::Options), Some("o.html".into()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_json_strings() {
        assert_eq!(json_string(r#"{"a" : "x\"y"}"#, "a"), Some("x\"y".into()));
        assert_eq!(
            json_string(r#"{"b": "a", "a": "z"}"#, "a"),
            Some("z".into())
        );
        assert_eq!(json_string(r#"{"a": 1}"#, "a"), None);
    }

    #[test]
    fn keeps_an_index() {
        let dir = std::env::temp_dir().join(format!("zeteme-index-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut index = Index::load(&dir);
        index.insert("abc".into(), dir.join("ext-1"));
        index.insert("def".into(), dir.join("ext-2").join("inner"));
        let index = Index::load(&dir);
        assert_eq!(
            index.folder("def"),
            Some(dir.join("ext-2").join("inner").as_path())
        );
        let mut index = index;
        assert_eq!(index.remove("abc"), Some(dir.join("ext-1")));
        assert_eq!(Index::load(&dir).entries.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Packs a store-style extension with Windows' tar, wraps it as a .crx
    /// and installs it from the file.
    #[cfg(windows)]
    #[test]
    fn unpacks_crx_files() {
        let dir = std::env::temp_dir().join(format!("zeteme-crx-{}", std::process::id()));
        let src = dir.join("src");
        fs::create_dir_all(src.join("_metadata")).unwrap();
        fs::write(src.join("manifest.json"), r#"{"manifest_version": 3}"#).unwrap();
        fs::write(src.join("_metadata").join("verified_contents.json"), "[]").unwrap();
        let zip = dir.join("ext.zip");
        let tar = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("tar.exe");
        let status = Command::new(tar)
            .arg("-a")
            .arg("-cf")
            .arg(&zip)
            .arg("-C")
            .arg(&src)
            .args(["manifest.json", "_metadata"])
            .status()
            .unwrap();
        assert!(status.success());
        let mut crx = b"Cr24\x03\0\0\0\x02\0\0\0hh".to_vec();
        crx.extend(fs::read(&zip).unwrap());
        let file = dir.join("ext.crx");
        fs::write(&file, crx).unwrap();

        let installed = dir.join("installed");
        let root = unpack(&file.to_string_lossy(), &installed).unwrap();
        assert!(root.join("manifest.json").is_file());
        assert!(!root.join("_metadata").exists());
        assert!(unpack(&dir.join("missing.crx").to_string_lossy(), &installed).is_err());
        assert_eq!(fs::read_dir(&installed).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
