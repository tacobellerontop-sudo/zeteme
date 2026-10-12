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

zeteme has no toolbar; everything is done from the keyboard. Clicking or
tabbing into a text field puts you in insert mode, where keys go to the page
until you press `Esc`.

| Key | Action |
| --- | --- |
| `j` / `k` | Scroll down / up |
| `h` / `l` | Scroll left / right |
| `d` / `u` | Half page down / up |
| `gg` / `G` | Top / bottom of page |
| `H` / `L` | Back / forward |
| `r` | Reload |
| `o` | Open a URL |
| `go` | Edit the current URL |
| `/` then `n` / `N` | Find text, next / previous match |
| `yy` | Copy the current URL |
| `gi` | Focus the first text field |
| `:` | Command bar |
| `Esc` | Leave insert mode or close the command bar |

Commands: `:open <url>` (`:o`), `:back`, `:forward`, `:reload` (`:r`), `:quit` (`:q`). Each CI run also uploads a release
build of `zeteme.exe` as a downloadable artifact.
