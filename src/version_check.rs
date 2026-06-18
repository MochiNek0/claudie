use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::app::AppState;

const RELEASES_API_URL: &str = "https://api.github.com/repos/MochiNek0/claudie/releases/latest";
const USER_AGENT: &str = concat!("claudie/", env!("CARGO_PKG_VERSION"));
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Give the UI and other startup work a head start before the first check.
const STARTUP_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

struct LatestRelease {
    version: String,
    url: String,
}

pub(crate) fn start_version_checker(state: Arc<Mutex<AppState>>) {
    let _ = thread::Builder::new()
        .name("claudie-version-check".to_string())
        .spawn(move || run_checker(state));
}

fn run_checker(state: Arc<Mutex<AppState>>) {
    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    thread::sleep(STARTUP_DELAY);
    loop {
        let delay = match fetch_latest_release(&agent) {
            Ok(release) => {
                if is_newer(CURRENT_VERSION, &release.version) {
                    apply_update(&state, release);
                }
                CHECK_INTERVAL
            }
            // A failed check is silent: stale or absent update info just means the
            // menu item stays hidden. Retry sooner than the success cadence.
            Err(_) => RETRY_INTERVAL,
        };
        thread::sleep(delay);
    }
}

fn fetch_latest_release(agent: &ureq::Agent) -> Result<LatestRelease, String> {
    let response = agent
        .get(RELEASES_API_URL)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .call();

    let text = match response {
        Ok(response) => response
            .into_string()
            .map_err(|err| format!("release response read failed: {err}"))?,
        Err(ureq::Error::Status(status, _)) => {
            return Err(format!("release API returned HTTP {status}"));
        }
        Err(err) => return Err(format!("release request failed: {err}")),
    };

    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("release JSON was invalid: {err}"))?;
    parse_release(&value)
}

fn parse_release(value: &Value) -> Result<LatestRelease, String> {
    let version = value
        .get("tag_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|tag| tag.trim_start_matches('v').to_string())
        .filter(|version| !version.is_empty())
        .ok_or_else(|| "release response did not include a tag_name.".to_string())?;
    let url = value
        .get("html_url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    Ok(LatestRelease { version, url })
}

fn apply_update(state: &Arc<Mutex<AppState>>, release: LatestRelease) {
    let mut state = state.lock().expect("state poisoned");
    state.update.latest_version = release.version;
    state.update.release_url = release.url;
}

/// Whether `latest` is a strictly newer version than `current`. Both may carry a
/// leading `v`. Components are compared numerically left to right; missing
/// components count as 0 (so `0.2` == `0.2.0`). Unparseable input is treated as 0.
fn is_newer(current: &str, latest: &str) -> bool {
    version_parts(latest) > version_parts(current)
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u64>()
                .unwrap_or(0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn newer_patch_and_minor_versions_detected() {
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.1.9", "0.2.0"));
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(is_newer("0.9.0", "1.0.0"));
    }

    #[test]
    fn equal_or_older_is_not_newer() {
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "v0.1.0"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1"));
    }

    #[test]
    fn invalid_input_is_safe() {
        assert!(!is_newer("0.1.0", ""));
        assert!(!is_newer("0.1.0", "garbage"));
        assert!(!is_newer("0.1.0", "v..."));
    }

    #[test]
    fn parse_release_strips_v_prefix() {
        let release = parse_release(&json!({
            "tag_name": "v1.2.3",
            "html_url": "https://example.com/r/1.2.3"
        }))
        .unwrap();
        assert_eq!(release.version, "1.2.3");
        assert_eq!(release.url, "https://example.com/r/1.2.3");
    }

    #[test]
    fn parse_release_requires_tag() {
        assert!(parse_release(&json!({})).is_err());
        assert!(parse_release(&json!({"tag_name": "  "})).is_err());
    }
}
