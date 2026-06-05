use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::app::{AppState, OfficialUsageWindow};
use crate::time_util::{date_value_unix_ms, now_unix_ms, usage_percent_value};
use crate::util::shorten;

const USAGE_API_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const USAGE_API_USER_AGENT: &str = "claude-code/2.1";
const USAGE_API_BETA: &str = "oauth-2025-04-20";
const FETCH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const FAILURE_INTERVAL: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
struct OAuthCredentials {
    access_token: String,
    subscription_type: String,
}

enum UsageError {
    /// Missing or expired credentials cannot self-heal quickly; retry slowly.
    Credentials(String),
    /// Network or response issues may clear up; retry sooner.
    Transient(String),
}

impl UsageError {
    fn retry_delay(&self) -> Duration {
        match self {
            Self::Credentials(_) => FETCH_INTERVAL,
            Self::Transient(_) => FAILURE_INTERVAL,
        }
    }

    fn into_message(self) -> String {
        match self {
            Self::Credentials(message) | Self::Transient(message) => message,
        }
    }
}

#[derive(Clone, Debug)]
struct OfficialUsageSnapshot {
    plan: String,
    five_hour: OfficialUsageWindow,
    seven_day: OfficialUsageWindow,
    updated_at_unix_ms: u64,
}

pub(crate) fn start_official_usage_poller(state: Arc<Mutex<AppState>>) {
    let _ = thread::Builder::new()
        .name("claudie-official-usage".to_string())
        .spawn(move || run_poller(state));
}

fn run_poller(state: Arc<Mutex<AppState>>) {
    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    loop {
        if !official_profile_is_active(&state) {
            thread::sleep(FAILURE_INTERVAL);
            continue;
        }

        let delay = match refresh_once(&agent) {
            Ok(snapshot) => {
                apply_snapshot(&state, snapshot);
                FETCH_INTERVAL
            }
            Err(err) => {
                let delay = err.retry_delay();
                record_error(&state, err.into_message());
                delay
            }
        };
        thread::sleep(delay);
    }
}

fn official_profile_is_active(state: &Arc<Mutex<AppState>>) -> bool {
    state
        .lock()
        .expect("state poisoned")
        .llm_profiles
        .official_profile_active()
}

fn refresh_once(agent: &ureq::Agent) -> Result<OfficialUsageSnapshot, UsageError> {
    let credentials = read_oauth_credentials().map_err(UsageError::Credentials)?;
    let body = fetch_usage(agent, &credentials.access_token).map_err(UsageError::Transient)?;
    let snapshot = OfficialUsageSnapshot {
        plan: plan_name(&credentials.subscription_type),
        five_hour: parse_usage_window(body.get("five_hour")),
        seven_day: parse_usage_window(body.get("seven_day")),
        updated_at_unix_ms: now_unix_ms(),
    };

    if snapshot.five_hour.used_percentage.is_none() && snapshot.seven_day.used_percentage.is_none()
    {
        return Err(UsageError::Transient(
            "Official usage response did not include rate limit windows.".to_string(),
        ));
    }

    Ok(snapshot)
}

fn fetch_usage(agent: &ureq::Agent, access_token: &str) -> Result<Value, String> {
    let auth = format!("Bearer {access_token}");
    let response = agent
        .get(USAGE_API_URL)
        .set("Authorization", &auth)
        .set("anthropic-beta", USAGE_API_BETA)
        .set("User-Agent", USAGE_API_USER_AGENT)
        .call();

    let text = match response {
        Ok(response) => response
            .into_string()
            .map_err(|err| format!("Official usage response failed: {err}"))?,
        Err(ureq::Error::Status(status, response)) => {
            let retry = response
                .header("Retry-After")
                .map(|value| format!(" retry after {value}s"))
                .unwrap_or_default();
            return Err(format!("Official usage API returned HTTP {status}.{retry}"));
        }
        Err(err) => return Err(format!("Official usage request failed: {err}")),
    };

    serde_json::from_str(&text).map_err(|err| format!("Official usage JSON was invalid: {err}"))
}

fn read_oauth_credentials() -> Result<OAuthCredentials, String> {
    if let Ok(token) = env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            return Ok(OAuthCredentials {
                access_token: token.to_string(),
                subscription_type: String::new(),
            });
        }
    }

    let path = credentials_path()?;
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("Official usage could not read {}: {err}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|err| format!("Official usage credentials JSON was invalid: {err}"))?;
    parse_oauth_credentials(&value, now_unix_ms())
}

fn parse_oauth_credentials(value: &Value, now_unix_ms: u64) -> Result<OAuthCredentials, String> {
    let source = value.get("claudeAiOauth").unwrap_or(value);
    let access_token = source
        .get("accessToken")
        .or_else(|| source.get("access_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "Official usage credentials did not include an access token.".to_string())?;

    if let Some(expires_at) = source
        .get("expiresAt")
        .or_else(|| source.get("expires_at"))
        .and_then(Value::as_u64)
        && expires_at <= now_unix_ms
    {
        return Err("Official usage token is expired; restart or /login Claude Code.".to_string());
    }

    let subscription_type = source
        .get("subscriptionType")
        .or_else(|| source.get("subscription_type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();

    Ok(OAuthCredentials {
        access_token: access_token.to_string(),
        subscription_type,
    })
}

fn credentials_path() -> Result<PathBuf, String> {
    if let Ok(config_dir) = env::var("CLAUDE_CONFIG_DIR") {
        let config_dir = config_dir.trim();
        if !config_dir.is_empty() {
            return Ok(PathBuf::from(config_dir).join(".credentials.json"));
        }
    }
    let home = env::var_os("USERPROFILE").ok_or_else(|| "USERPROFILE is not set.".to_string())?;
    Ok(PathBuf::from(home)
        .join(".claude")
        .join(".credentials.json"))
}

fn parse_usage_window(value: Option<&Value>) -> OfficialUsageWindow {
    let Some(object) = value.and_then(Value::as_object) else {
        return OfficialUsageWindow::default();
    };

    let used_percentage = object
        .get("used_percentage")
        .or_else(|| object.get("utilization"))
        .and_then(usage_percent_value);

    let reset_value = object.get("resets_at").or_else(|| object.get("reset_at"));
    let reset_at_unix_ms = reset_value.and_then(date_value_unix_ms);
    let reset_label = if reset_at_unix_ms.is_none() {
        reset_value
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| shorten(text, 32))
            .unwrap_or_default()
    } else {
        String::new()
    };

    OfficialUsageWindow {
        used_percentage,
        reset_at_unix_ms,
        reset_label,
    }
}

fn plan_name(subscription_type: &str) -> String {
    let lower = subscription_type.to_ascii_lowercase();
    if lower.contains("max") {
        "Max".to_string()
    } else if lower.contains("pro") {
        "Pro".to_string()
    } else if lower.contains("team") {
        "Team".to_string()
    } else if subscription_type.trim().is_empty() || lower.contains("api") {
        "Official".to_string()
    } else {
        subscription_type.trim().to_string()
    }
}

fn apply_snapshot(state: &Arc<Mutex<AppState>>, snapshot: OfficialUsageSnapshot) {
    let mut state = state.lock().expect("state poisoned");
    // Re-check under the lock: the user may have switched profiles while the
    // request was in flight.
    if !state.llm_profiles.official_profile_active() {
        return;
    }
    state.quota.provider = "Claude Code".to_string();
    state.quota.official_plan = snapshot.plan;
    state.quota.official_five_hour = snapshot.five_hour;
    state.quota.official_seven_day = snapshot.seven_day;
    state.quota.official_usage_updated_at_unix_ms = Some(snapshot.updated_at_unix_ms);
    state.quota.official_usage_error.clear();
}

fn record_error(state: &Arc<Mutex<AppState>>, err: String) {
    let mut state = state.lock().expect("state poisoned");
    state.quota.official_usage_error = err;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_usage_window_with_iso_reset() {
        let window = parse_usage_window(Some(&json!({
            "utilization": 42.4,
            "resets_at": "2026-04-20T15:00:00.000Z"
        })));

        assert_eq!(window.used_percentage, Some(42));
        assert_eq!(window.reset_at_unix_ms, Some(1_776_697_200_000));
        assert!(window.reset_label.is_empty());
    }

    #[test]
    fn parses_usage_window_with_offset_iso_reset() {
        let window = parse_usage_window(Some(&json!({
            "utilization": 42.4,
            "resets_at": "2026-04-20T15:00:00+00:00"
        })));

        assert_eq!(window.used_percentage, Some(42));
        assert_eq!(window.reset_at_unix_ms, Some(1_776_697_200_000));
        assert!(window.reset_label.is_empty());
    }

    #[test]
    fn parses_nested_credentials() {
        let value = json!({
            "claudeAiOauth": {
                "accessToken": " token ",
                "subscriptionType": "max",
                "expiresAt": 2_000
            }
        });

        let credentials = parse_oauth_credentials(&value, 1_000).unwrap();

        assert_eq!(credentials.access_token, "token");
        assert_eq!(credentials.subscription_type, "max");
    }

    #[test]
    fn parses_legacy_flat_credentials() {
        let value = json!({
            "access_token": "token",
            "subscription_type": "pro"
        });

        let credentials = parse_oauth_credentials(&value, 1_000).unwrap();

        assert_eq!(credentials.access_token, "token");
        assert_eq!(credentials.subscription_type, "pro");
    }

    #[test]
    fn rejects_expired_token() {
        let value = json!({
            "claudeAiOauth": {
                "accessToken": "token",
                "expiresAt": 1_000
            }
        });

        let err = parse_oauth_credentials(&value, 2_000).unwrap_err();

        assert!(err.contains("expired"));
    }

    #[test]
    fn rejects_missing_access_token() {
        assert!(parse_oauth_credentials(&json!({}), 0).is_err());
        assert!(parse_oauth_credentials(&json!({"accessToken": "  "}), 0).is_err());
    }
}
