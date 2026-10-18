//! Tiny persisted key=value settings in ~/.config/mditor/settings.

use std::path::PathBuf;

fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("mditor").join("settings")
}

pub fn get(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path()).ok()?;
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=').map(|v| v.trim().to_string()))
}

pub fn set(key: &str, value: &str) {
    let p = path();
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    let mut out: String = old
        .lines()
        .filter(|l| !l.strip_prefix(key).is_some_and(|r| r.starts_with('=')))
        .map(|l| format!("{l}\n"))
        .collect();
    out.push_str(&format!("{key}={value}\n"));
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, out);
}
