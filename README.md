# mditor

Fast, low-resource Markdown viewer and editor (Rust, GTK4, GtkSourceView 5, pulldown-cmark; no WebKit).

- Instant **View / Edit** toggle (`Ctrl+E`) with scroll position kept in sync
- **Tabs** (`Ctrl+T`, `Ctrl+W`, `Ctrl+Tab`), drag & drop, open several files at once
- Native preview: headings, lists, task lists, quotes, code, tables, images, links
- Follows the system theme; X11 and Wayland

## Install

Download the `.deb`, `.rpm` or `.flatpak` from the latest GitHub Release.

## Build

```
sudo apt install libgtk-4-dev libgtksourceview-5-dev pkg-config
cargo build --release
```
