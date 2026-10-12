# zeteme

A keyboard-driven web browser for Windows, written in Rust, in the spirit of
[qutebrowser](https://qutebrowser.org/). Pages are rendered with Microsoft
WebView2 through the [`wry`](https://crates.io/crates/wry) crate.

## Requirements

- Windows 10 or 11 with the WebView2 runtime (included with current Windows versions)
- Rust stable

## Running

```sh
cargo run -- example.com
```

With no argument, zeteme opens DuckDuckGo.

## Keys

zeteme has no toolbar; everything is done from the keyboard, with bindings
modeled on qutebrowser. Clicking or tabbing into a text field puts you in
insert mode, where keys go to the page until you press `Esc`.

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
| `:` | Command bar |
| `Esc` | Leave insert mode, close the command bar or cancel hints |

Commands: `:open <url or search>` (`:o`), `:tabopen` (`:t`), `:close`,
`:back`, `:forward`, `:reload` (`:r`), `:quit` (`:q`).

Anything typed into `:open` that isn't a URL is searched on DuckDuckGo.

## Data

Cookies, logins and the open tabs are kept in `%LOCALAPPDATA%\zeteme`, so
zeteme reopens where you left off. Downloads go to your Downloads folder.
