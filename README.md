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

With no argument, zeteme opens DuckDuckGo. Each CI run also uploads a release
build of `zeteme.exe` as a downloadable artifact.
