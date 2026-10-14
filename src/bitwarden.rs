//! Filling logins from Bitwarden, like qutebrowser's `qute-bitwarden`
//! userscript: zeteme asks the Bitwarden CLI (`bw`) for the logins that match
//! the page, and keeps the unlocked session key in memory until it closes.
//! Every call blocks, so they run off the UI thread.

use std::process::{Command, Stdio};

use crate::json::{self, Value};

/// Where `bw unlock` reads the master password from, so it never shows up
/// on a command line.
const PASSWORD_VAR: &str = "ZETEME_BW_PASSWORD";

#[derive(Clone, Debug, PartialEq)]
pub struct Login {
    pub id: String,
    pub name: String,
    pub username: String,
    pub password: String,
    pub has_totp: bool,
    /// The sites the login is saved for.
    pub uris: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    /// The vault needs the master password.
    Locked,
    Failed(String),
}

fn run(args: &[&str], session: Option<&str>, password: Option<&str>) -> Result<String, Error> {
    // `bw` is either a standalone bw.exe or npm's bw.cmd wrapper, which
    // Command only finds by its full name.
    let names: &[&str] = if cfg!(windows) {
        &["bw", "bw.cmd"]
    } else {
        &["bw"]
    };
    for name in names {
        let mut command = Command::new(name);
        command
            .args(args)
            .arg("--nointeraction")
            .stdin(Stdio::null())
            .env_remove(PASSWORD_VAR);
        if let Some(session) = session {
            command.args(["--session", session]);
        }
        if let Some(password) = password {
            command.env(PASSWORD_VAR, password);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let output = match command.output() {
            Ok(output) => output,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(Error::Failed(format!("could not run bw: {err}"))),
        };
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() {
            return Ok(stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() { stdout } else { stderr };
        let lower = message.to_lowercase();
        return Err(if lower.contains("locked") {
            Error::Locked
        } else if lower.contains("not logged in") {
            Error::Failed("log in once with `bw login` in a terminal".into())
        } else {
            Error::Failed(message)
        });
    }
    Err(Error::Failed(
        "the Bitwarden CLI (bw) isn't installed or isn't on PATH".into(),
    ))
}

/// Unlocks the vault and returns the session key.
pub fn unlock(password: &str) -> Result<String, Error> {
    let key = run(
        &["unlock", "--raw", "--passwordenv", PASSWORD_VAR],
        None,
        Some(password),
    )
    .map_err(|err| match err {
        Error::Locked => Error::Failed("wrong master password".into()),
        err => err,
    })?;
    if key.is_empty() {
        return Err(Error::Failed("wrong master password".into()));
    }
    // Pick up logins added elsewhere since bw last synced; a failed sync
    // still leaves the cached vault usable.
    let _ = run(&["sync"], Some(&key), None);
    Ok(key)
}

pub fn lock() {
    let _ = run(&["lock"], None, None);
}

/// The logins saved for `url`, or every login without one.
pub fn logins(url: Option<&str>, session: Option<&str>) -> Result<Vec<Login>, Error> {
    // Without a key of our own, a BW_SESSION from the environment may still
    // unlock the vault; with neither, bw reports it is locked.
    let mut args = vec!["list", "items"];
    if let Some(url) = url {
        args.extend(["--url", url]);
    }
    let output = run(&args, session, None)?;
    parse_logins(&output).ok_or_else(|| Error::Failed("could not read bw's output".into()))
}

fn parse_logins(output: &str) -> Option<Vec<Login>> {
    let items = json::parse(output)?;
    Some(
        items
            .as_array()
            .iter()
            .filter_map(|item| {
                let login = item.get("login");
                if matches!(login, Value::Null) {
                    return None;
                }
                let text = |v: &Value| v.as_str().unwrap_or("").to_string();
                Some(Login {
                    id: text(item.get("id")),
                    name: text(item.get("name")),
                    username: text(login.get("username")),
                    password: text(login.get("password")),
                    has_totp: login.get("totp").as_str().is_some_and(|t| !t.is_empty()),
                    uris: login
                        .get("uris")
                        .as_array()
                        .iter()
                        .filter_map(|uri| uri.get("uri").as_str())
                        .map(str::to_string)
                        .collect(),
                })
            })
            .collect(),
    )
}

/// The current one-time code of the login `id`.
pub fn totp(id: &str, session: Option<&str>) -> Result<String, Error> {
    run(&["get", "totp", id], session, None)
}

/// Puts `text` on the clipboard with Windows' `clip.exe`.
pub fn copy(text: &str) -> bool {
    use std::io::Write;
    let mut command = Command::new("clip");
    command.stdin(Stdio::piped()).stdout(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    child.wait().is_ok_and(|status| status.success()) && wrote
}

/// The site part of `url`, to check the page didn't change while bw ran.
pub fn origin(url: &str) -> &str {
    let after_scheme = url.find("://").map_or(0, |i| i + 3);
    let end = url[after_scheme..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |i| after_scheme + i);
    &url[..end]
}

/// The host of `url` without "www.", or of a bare "example.com".
pub fn host(url: &str) -> String {
    let rest = url.find("://").map_or(url, |i| &url[i + 3..]);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next().unwrap_or("").to_lowercase();
    host.strip_prefix("www.")
        .map(str::to_string)
        .unwrap_or(host)
}

/// Whether `login` is saved for the site at `url`, or a site above or below
/// it (accounts.example.com and example.com).
pub fn matches(login: &Login, url: &str) -> bool {
    let page = host(url);
    !page.is_empty()
        && login.uris.iter().any(|uri| {
            let saved = host(uri);
            !saved.is_empty()
                && (saved == page
                    || page.ends_with(&format!(".{saved}"))
                    || saved.ends_with(&format!(".{page}")))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_logins() {
        let output = r#"[
          {"id": "1", "type": 1, "name": "GitHub",
           "login": {"username": "lx", "password": "p\"w", "totp": "otpauth://x",
                     "uris": [{"match": null, "uri": "https://github.com"}]}},
          {"id": "2", "type": 2, "name": "A note", "login": null, "secureNote": {"type": 0}},
          {"id": "3", "type": 1, "name": "Other", "login": {"username": null, "password": "x", "totp": null}}
        ]"#;
        let logins = parse_logins(output).unwrap();
        assert_eq!(logins.len(), 2);
        assert_eq!(logins[0].username, "lx");
        assert_eq!(logins[0].password, "p\"w");
        assert!(logins[0].has_totp);
        assert_eq!(logins[1].username, "");
        assert!(!logins[1].has_totp);
        assert_eq!(parse_logins("[]"), Some(vec![]));
        assert_eq!(parse_logins("? Master password"), None);
    }

    #[test]
    fn matches_sites() {
        let login = Login {
            id: String::new(),
            name: String::new(),
            username: String::new(),
            password: String::new(),
            has_totp: false,
            uris: vec![
                "https://www.example.com/login".into(),
                "androidapp://x".into(),
            ],
        };
        assert!(matches(&login, "https://example.com/"));
        assert!(matches(&login, "https://accounts.example.com/signin"));
        assert!(!matches(&login, "https://notexample.com/"));
        assert!(!matches(&login, "https://example.org/"));
        assert_eq!(host("example.com"), "example.com");
        assert_eq!(host("https://u@WWW.Example.com:8080/x"), "example.com");
    }

    #[test]
    fn finds_origins() {
        assert_eq!(
            origin("https://a.example.com/login?x"),
            "https://a.example.com"
        );
        assert_eq!(origin("https://a.example.com"), "https://a.example.com");
        assert_eq!(origin("http://localhost:8080#x"), "http://localhost:8080");
    }
}
