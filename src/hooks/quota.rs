use serde_json::Value;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::app::QuotaStats;
use crate::util::shorten;

pub(super) fn update_quota_from_value(quota: &mut QuotaStats, value: &Value) {
    walk_quota_value(quota, value, None);

    let latest_total = quota
        .input_tokens
        .saturating_add(quota.output_tokens)
        .saturating_add(quota.cache_creation_tokens)
        .saturating_add(quota.cache_read_tokens);
    if latest_total > 0 {
        quota.total_tokens = quota.total_tokens.max(latest_total);
    }
}

fn walk_quota_value(quota: &mut QuotaStats, value: &Value, parent_key: Option<&str>) {
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
                    "ratelimits" | "limits" => quota.rate_limits = shorten(&child.to_string(), 130),
                    _ => {}
                }
                capture_provider_quota_field(quota, key, &normalized_key, child, parent_key);
                walk_quota_value(quota, child, Some(key));
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_quota_value(quota, item, parent_key);
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
        update_quota_from_value(&mut quota, &value);
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
