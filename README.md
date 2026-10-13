# zeteme

A keyboard-driven web browser for Windows, written in Rust, in the spirit of
[qutebrowser](https://qutebrowser.org/). Pages are rendered with Microsoft
WebView2 through the [`wry`](https://crates.io/crates/wry) crate.

## Requirements

- Windows 10 or 11 with the WebView2 runtime (included with current Windows versions)
- Rust stable, only to build from source

## Install

Download `zeteme-setup.exe` from the latest
[release](https://github.com/tacobellerontop-sudo/zeteme/releases) and run it.
It installs zeteme for your user only, so it needs no admin rights, and adds
it to the Start menu. The plain `zeteme.exe` is attached too if you'd rather
run it without installing.

## Updates

`:settings` opens the settings page. Its Updates section shows the running
version, checks GitHub for a newer release and installs it: zeteme closes,
the installer runs without its wizard, and zeteme reopens with your tabs.
By default zeteme checks once when it starts and, if there is an update,
says so in the window title. Updating needs `curl.exe`, which comes with
Windows 10 and 11.

## Releasing

Bump `version` in `Cargo.toml`, then push a matching `v*` tag or run the
Release workflow from the Actions tab with that tag. The workflow refuses a
tag that doesn't match `Cargo.toml`, because zeteme compares the two to find
updates.

### Code signing

Windows Smart App Control blocks unsigned programs, so releases should be
signed. The Release workflow signs `zeteme.exe` and the installer with
[Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/)
(formerly Trusted Signing) once these repository secrets are set; without
them it builds unsigned releases as before.

| Secret | Value |
| --- | --- |
| `AZURE_TENANT_ID` | Directory (tenant) ID of the app registration |
| `AZURE_CLIENT_ID` | Application (client) ID of the app registration |
| `AZURE_CLIENT_SECRET` | A client secret of the app registration |
| `AZURE_SIGNING_ENDPOINT` | The signing account's endpoint, e.g. `https://eus.codesigning.azure.net/` |
| `AZURE_SIGNING_ACCOUNT` | The signing account name |
| `AZURE_CERTIFICATE_PROFILE` | The certificate profile name |

The app registration needs the *Artifact Signing Certificate Profile Signer*
role on the signing account.

## Building

```sh
cargo run -- example.com
```

With no argument, zeteme reopens your last session, or DuckDuckGo the first
time. Video fullscreen fills the screen; `Esc` leaves it.

## Keys

zeteme has no toolbar and only a pop-up tab bar; everything is done from the keyboard, with bindings
modeled on qutebrowser. Clicking or tabbing into a text field puts you in
insert mode, where keys go to the page until you press `Esc`. A field the page
focuses by itself (a search box on load, say) doesn't: click it or press `i`.
Keys work the same when an embedded player or other frame has focus.

| Key | Action |
| --- | --- |
| `j` / `k` | Scroll down / up |
| `h` / `l` | Scroll left / right |
| `Ctrl-d` / `Ctrl-u` | Half page down / up |
| `gg` / `G` | Top / bottom of page |
| `H` / `L` | Back / forward |
| `r` | Reload |
| `f` / `F` | Link hints: follow / open in new tab |
| `o` / `O` | Open a URL or search, in this tab / a new tab |
| `go` | Edit the current URL |
| `J` / `K` (or `gt` / `gT`) | Next / previous tab |
| `d` / `u` | Close tab / reopen closed tab |
| `+` / `-` / `=` | Zoom in / out / reset |
| `/` then `n` / `N` | Find text, next / previous match |
| `yy` | Copy the current URL |
| `gi` | Focus the first text field |
| `i` | Type into the field the page focused |
| `:` | Command bar |
| `Esc` | Leave insert mode, close the command bar or cancel hints |

Commands: `:open <url or search>` (`:o`), `:tabopen` (`:t`), `:close`,
`:back`, `:forward`, `:reload` (`:r`), `:settings` (`:set`), `:quit` (`:q`).

Anything typed into `:open` that isn't a URL is searched on DuckDuckGo.

## Tabs

A tab bar pops up over the page for a moment when you open, close or switch
tabs, then hides again; clicking a tab switches to it. In `:settings` you can
also have it come up when the mouse reaches its edge of the window, show it
always (pages make room for it) or never, and put it at the top, bottom, left
or right.

## Data

Cookies, logins, settings and the open tabs are kept in `%LOCALAPPDATA%\zeteme`, so
zeteme reopens where you left off. Downloads go to your Downloads folder.
