use serde_json::Value;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::app::{OfficialUsageWindow, QuotaStats};
use crate::time_util::{date_value_unix_ms, now_unix_ms, usage_percent_value};
use crate::util::shorten;

/// `capture_official` should be true only while the official profile is
/// active, so other providers' payloads cannot pollute the official usage
/// windows maintained by the official usage poller.
pub(super) fn update_quota_from_value(
    quota: &mut QuotaStats,
    value: &Value,
    capture_official: bool,
) {
    walk_quota_value(quota, value, None, capture_official);

    let latest_total = quota
        .input_tokens
        .saturating_add(quota.output_tokens)
        .saturating_add(quota.cache_creation_tokens)
        .saturating_add(quota.cache_read_tokens);
    if latest_total > 0 {
        quota.total_tokens = quota.total_tokens.max(latest_total);
    }
}

/// First string value under a `model` key anywhere in the payload, used to tag
/// a session with the model (provider) it is using. Mirrors the `model` case in
/// `walk_quota_value` but is per-session rather than global.
pub(super) fn model_from_value(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if normalize_key(key) == "model"
                    && let Some(model) = child.as_str()
                    && !model.trim().is_empty()
                {
                    return Some(model.trim().to_string());
                }
                if let Some(found) = model_from_value(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(model_from_value),
        _ => None,
    }
}

fn walk_quota_value(
    quota: &mut QuotaStats,
    value: &Value,
    parent_key: Option<&str>,
    capture_official: bool,
) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized_key = normalize_key(key);
                match normalized_key.as_str() {
                    "inputtokens" => {
                        quota.input_tokens = number_value(child).unwrap_or(quota.input_tokens)
                    }
                    "outputtokens" => {
                        quota.output_tokens = number_value(child).unwrap_or(quota.output_tokens)
                    }
                    "cachecreationinputtokens" => {
                        quota.cache_creation_tokens =
                            number_value(child).unwrap_or(quota.cache_creation_tokens)
                    }
                    "cachereadinputtokens" => {
                        quota.cache_read_tokens =
                            number_value(child).unwrap_or(quota.cache_read_tokens)
                    }
                    "totaltokens" => {
                        quota.total_tokens = number_value(child).unwrap_or(quota.total_tokens)
                    }
                    "model" => {
                        if let Some(model) = child.as_str() {
                            quota.last_model = model.to_string();
                        }
                    }
                    "provider" | "providername" | "providerid" => {
                        if let Some(provider) = child.as_str() {
                            quota.provider = shorten(provider, 28);
                        }
                    }
                    "ratelimits" | "limits" => {
                        quota.rate_limits = shorten(&child.to_string(), 130);
                        if capture_official {
                            capture_official_rate_limits(quota, child);
                        }
                    }
                    _ => {}
                }
                capture_provider_quota_field(quota, key, &normalized_key, child, parent_key);
                walk_quota_value(quota, child, Some(key), capture_official);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_quota_value(quota, item, parent_key, capture_official);
            }
        }
        _ => {}
    }
}

fn capture_provider_quota_field(
    quota: &mut QuotaStats,
    key: &str,
    normalized_key: &str,
    value: &Value,
    parent_key: Option<&str>,
) {
    let Some(display_value) = display_quota_value(value) else {
        return;
    };

    if is_remaining_key(normalized_key) {
        quota.quota_remaining = display_value;
        if quota.provider.is_empty() {
            quota.provider = quota_source_label(key, parent_key);
        }
    } else if is_limit_key(normalized_key) {
        quota.quota_limit = display_value;
    } else if is_reset_key(normalized_key) {
        quota.quota_reset = display_value;
    }
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|ch| *ch != '_' && *ch != '-' && *ch != ' ')
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_remaining_key(key: &str) -> bool {
    matches!(
        key,
        "remaining"
            | "remainingquota"
            | "quotaremaining"
            | "remainingcredits"
            | "creditsremaining"
            | "remainingcredit"
            | "creditremaining"
            | "creditbalance"
            | "balance"
            | "remainingbalance"
            | "balanceremaining"
            | "remainingrequests"
            | "requestsremaining"
            | "remainingtokens"
            | "tokensremaining"
            | "remainingmessages"
            | "messagesremaining"
    ) || (key.contains("remaining")
        && [
            "quota", "credit", "balance", "request", "token", "message", "usage",
        ]
        .iter()
        .any(|marker| key.contains(marker)))
}

fn is_limit_key(key: &str) -> bool {
    matches!(
        key,
        "limit"
            | "quota"
            | "quotalimit"
            | "creditlimit"
            | "creditslimit"
            | "requestlimit"
            | "requestsperminute"
            | "tokensperminute"
            | "messagelimit"
    ) || (key.contains("limit")
        && ["quota", "credit", "request", "token", "message", "usage"]
            .iter()
            .any(|marker| key.contains(marker)))
}

fn is_reset_key(key: &str) -> bool {
    matches!(
        key,
        "reset"
            | "resetat"
            | "resetsat"
            | "resetin"
            | "resetsin"
            | "retryafter"
            | "retryafterseconds"
    )
}

fn display_quota_value(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) if !text.trim().is_empty() => Some(shorten(text.trim(), 42)),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn quota_source_label(key: &str, parent_key: Option<&str>) -> String {
    parent_key
        .filter(|parent| {
            let parent = normalize_key(parent);
            !matches!(parent.as_str(), "usage" | "metadata")
        })
        .unwrap_or(key)
        .chars()
        .map(|ch| if ch == '_' || ch == '-' { ' ' } else { ch })
        .collect::<String>()
        .trim()
        .to_string()
}

fn number_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
}

fn capture_official_rate_limits(quota: &mut QuotaStats, value: &Value) {
    let Some(object) = value.as_object() else {
        return;
    };

    for (key, child) in object {
        match normalize_key(key).as_str() {
            "fivehour" => merge_official_window(&mut quota.official_five_hour, child),
            "sevenday" => merge_official_window(&mut quota.official_seven_day, child),
            _ => {}
        }
    }

    if quota.official_five_hour.used_percentage.is_some()
        || quota.official_seven_day.used_percentage.is_some()
    {
        quota.official_usage_error.clear();
        quota.official_usage_updated_at_unix_ms = Some(now_unix_ms());
    }
}

fn merge_official_window(window: &mut OfficialUsageWindow, value: &Value) {
    let Some(object) = value.as_object() else {
        return;
    };

    for (key, child) in object {
        match normalize_key(key).as_str() {
            "usedpercentage" | "utilization" => {
                if let Some(percent) = usage_percent_value(child) {
                    window.used_percentage = Some(percent);
                }
            }
            "resetsat" | "resetat" => {
                if let Some(reset_at) = date_value_unix_ms(child) {
                    window.reset_at_unix_ms = Some(reset_at);
                    window.reset_label.clear();
                } else if let Some(label) = child
                    .as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    // A label replaces any previously captured timestamp;
                    // otherwise the stale timestamp would keep rendering.
                    window.reset_at_unix_ms = None;
                    window.reset_label = shorten(label, 32);
                }
            }
            _ => {}
        }
    }
}

pub(super) fn scan_transcript_usage(path: &Path) -> Option<QuotaStats> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() == 0 {
        return None;
    }

    let mut file = fs::File::open(path).ok()?;
    let start = metadata.len().saturating_sub(1_500_000);
    if start > 0 {
        file.seek(SeekFrom::Start(start)).ok()?;
    }

    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    let mut quota = QuotaStats::default();
    let mut observed = 0_u64;

    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let before = quota
            .input_tokens
            .saturating_add(quota.output_tokens)
            .saturating_add(quota.cache_creation_tokens)
            .saturating_add(quota.cache_read_tokens);
        // Official fields are dropped when the snapshot is merged back into
        // state.quota, so transcript scans never capture them.
        update_quota_from_value(&mut quota, &value, false);
        let after = quota
            .input_tokens
            .saturating_add(quota.output_tokens)
            .saturating_add(quota.cache_creation_tokens)
            .saturating_add(quota.cache_read_tokens);
        if after > before {
            observed = observed.saturating_add(after - before);
        }
    }

    quota.observed_total_tokens = observed.max(quota.total_tokens);
    Some(quota)
}
