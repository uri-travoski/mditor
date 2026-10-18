//! Update checks against the latest GitHub release, plus the persisted
//! "check on startup" preference.

use std::path::PathBuf;
use std::time::Duration;

const API: &str = "https://api.github.com/repos/uri-travoski/mditor/releases/latest";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub enum Outcome {
    UpToDate,
    Available { version: String, url: String },
    Failed(String),
}

fn settings_path() -> PathBuf {
    gtk::glib::user_config_dir().join("mditor").join("settings")
}

/// Defaults to on when nothing has been saved yet.
pub fn auto_check_enabled() -> bool {
    std::fs::read_to_string(settings_path())
        .map(|s| !s.lines().any(|l| l.trim() == "auto_update=false"))
        .unwrap_or(true)
}

pub fn set_auto_check(on: bool) {
    let p = settings_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, format!("auto_update={on}\n"));
}

/// Blocking; run it off the main thread.
pub fn check() -> Outcome {
    let resp = ureq::get(API)
        .set("User-Agent", concat!("mditor/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(8))
        .call();
    let body = match resp {
        Ok(r) => match r.into_string() {
            Ok(b) => b,
            Err(e) => return Outcome::Failed(e.to_string()),
        },
        Err(ureq::Error::Status(404, _)) => return Outcome::UpToDate,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    let Some(tag) = json_str(&body, "tag_name") else {
        return Outcome::Failed("unexpected response from GitHub".into());
    };
    if newer(&tag, CURRENT) {
        let url = json_str(&body, "html_url").unwrap_or_else(|| "https://github.com/uri-travoski/mditor/releases".into());
        Outcome::Available { version: tag.trim_start_matches('v').to_string(), url }
    } else {
        Outcome::UpToDate
    }
}

fn json_str(body: &str, key: &str) -> Option<String> {
    let at = body.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

fn parts(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split('.').map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)).collect()
}

pub fn newer(latest: &str, current: &str) -> bool {
    let (a, b) = (parts(latest), parts(current));
    (0..a.len().max(b.len())).map(|i| (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0))).find(|(x, y)| x != y).is_some_and(|(x, y)| x > y)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_versions() {
        assert!(newer("v1.0.1", "1.0.0"));
        assert!(newer("1.10.0", "1.9.9"));
        assert!(!newer("v1.0.0", "1.0.0"));
        assert!(!newer("0.9", "1.0.0"));
    }
    #[test]
    fn parses_json() {
        let b = r#"{"tag_name": "v2.0.0","html_url":"https://x/y"}"#;
        assert_eq!(json_str(b, "tag_name").as_deref(), Some("v2.0.0"));
        assert_eq!(json_str(b, "html_url").as_deref(), Some("https://x/y"));
    }
}
