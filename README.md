# mditor

Fast, low-resource Markdown viewer and editor (Rust, GTK4, GtkSourceView 5, pulldown-cmark; no WebKit).

- Instant **View / Edit** toggle (`Ctrl+E`) with scroll position kept in sync
- **Tabs** (`Ctrl+T`, `Ctrl+W`, `Ctrl+Tab`), drag & drop, open several files at once
- Obsidian-style native preview: callouts (foldable), `[[wikilinks]]` and `![[embeds]]` (notes, sections, images with `|300` sizes, local video/audio, YouTube cards), `==highlights==`, `#tags`, `%%comments%%`, properties, footnotes, math (as Unicode), syntax-coloured code, bordered full-width tables, inline HTML, remote images
- Follows the system theme; X11 and Wayland

## Install

Download the `.deb`, `.rpm` or `.flatpak` from the latest GitHub Release.

## Build

```
sudo apt install libgtk-4-dev libgtksourceview-5-dev pkg-config
cargo build --release
```
