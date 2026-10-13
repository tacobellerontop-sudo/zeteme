//! Checking GitHub Releases for a newer zeteme and installing it.
//!
//! The work is done by `curl.exe`, which ships with Windows 10 and 11, so
//! zeteme carries no HTTP or TLS code of its own.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const RELEASES_URL: &str = "https://github.com/tacobellerontop-sudo/zeteme/releases";
/// Name of the installer attached to every release by the Release workflow.
const INSTALLER_ASSET: &str = "zeteme-setup.exe";

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(String),
    Downloading(String),
    Failed(String),
}

/// Runs curl without flashing a console window.
pub(crate) fn curl(args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("curl");
    command
        .args(["--silent", "--show-error", "--proto", "=https"])
        .args(args)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|err| format!("could not run curl: {err}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Asks GitHub for the tag of the latest release. `/releases/latest`
/// redirects to `/releases/tag/<tag>`, which avoids the rate-limited API.
pub fn latest_tag() -> Result<String, String> {
    let url = format!("{RELEASES_URL}/latest");
    let redirect = curl(&[
        "--head",
        "--max-time",
        "15",
        "--output",
        NULL_DEVICE,
        "--write-out",
        "%{redirect_url}",
        &url,
    ])?;
    tag_from_redirect(&redirect).ok_or_else(|| "no release found".to_string())
}

fn tag_from_redirect(url: &str) -> Option<String> {
    let tag = url.trim().rsplit_once("/releases/tag/")?.1;
    (!tag.is_empty()).then(|| tag.to_string())
}

/// Downloads the installer for `tag` into the temp folder.
pub fn download(tag: &str) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(INSTALLER_ASSET);
    let url = format!("{RELEASES_URL}/download/{tag}/{INSTALLER_ASSET}");
    let dest = path.to_string_lossy();
    curl(&[
        "--fail",
        "--location",
        "--max-time",
        "600",
        "--output",
        &dest,
        &url,
    ])?;
    Ok(path)
}

/// Starts the installer without its wizard; it waits for zeteme to close,
/// replaces it and starts it again.
pub fn install(installer: &Path) -> Result<(), String> {
    Command::new(installer)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("could not start the installer: {err}"))
}

fn parse_version(version: &str) -> Option<Vec<u64>> {
    let version = version.trim().trim_start_matches('v');
    let core = version.split(['-', '+']).next()?;
    core.split('.').map(|part| part.parse().ok()).collect()
}

/// Whether the release `tag` is newer than the running zeteme.
pub fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(tag), Some(current)) => tag > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tag_from_redirect() {
        assert_eq!(
            tag_from_redirect(
                "https://github.com/tacobellerontop-sudo/zeteme/releases/tag/v0.2.0\n"
            ),
            Some("v0.2.0".into())
        );
        // With no releases GitHub redirects to the release list.
        assert_eq!(
            tag_from_redirect("https://github.com/tacobellerontop-sudo/zeteme/releases"),
            None
        );
        assert_eq!(tag_from_redirect(""), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.3"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }
}
