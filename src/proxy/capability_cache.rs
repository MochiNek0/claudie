use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::proxy_optimizer;
use crate::settings::LlmProfile;
use crate::settings::storage::{read_json_or_default, save_pretty_json};

use super::tool_history::request_has_tool_history;

const CAPABILITY_CACHE_VERSION: &str = "v1";
const DEFAULT_CAPABILITY_CACHE_TTL_HOURS: u64 = 720;
const DEFAULT_CAPABILITY_CACHE_MAX_ENTRIES: usize = 200;
const DEFAULT_PROXY_CACHE_MAX_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CapabilityCacheFile {
    #[serde(default)]
    version: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    model: String,
    supports_native_tool_history: Option<bool>,
    #[serde(default)]
    created_at_ms: u128,
    #[serde(default)]
    last_used_at_ms: u128,
}

pub(super) fn cached_tool_history_needs_transcript(profile: &LlmProfile, request: &Value) -> bool {
    if !request_has_tool_history(request) {
        return false;
    }
    let path = capability_cache_file_path(profile, request);
    cached_tool_history_needs_transcript_at(profile, &path)
}

/// Return whether this upstream/model previously needed text transcript mode.
fn cached_tool_history_needs_transcript_at(profile: &LlmProfile, path: &Path) -> bool {
    let mut cache: CapabilityCacheFile = read_json_or_default(path);
    if cache.version != CAPABILITY_CACHE_VERSION {
        return false;
    }
    if capability_cache_expired(&cache, profile) {
        return false;
    }
    cache.last_used_at_ms = now_millis();
    let _ = save_pretty_json(path, &cache);
    cache.supports_native_tool_history == Some(false)
}

pub(super) fn save_tool_history_capability(
    profile: &LlmProfile,
    request: &Value,
    supports_native_tool_history: bool,
) -> Result<(), String> {
    let path = capability_cache_file_path(profile, request);
    save_tool_history_capability_at(profile, request, supports_native_tool_history, &path)?;
    proxy_optimizer::prune_cache_dir(
        &capability_cache_dir(),
        capability_cache_ttl_hours(profile),
        capability_cache_max_entries(profile),
        proxy_cache_max_bytes(profile),
    )
}

fn save_tool_history_capability_at(
    profile: &LlmProfile,
    request: &Value,
    supports_native_tool_history: bool,
    path: &Path,
) -> Result<(), String> {
    let now = now_millis();
    let cache = CapabilityCacheFile {
        version: CAPABILITY_CACHE_VERSION.to_string(),
        kind: "capability".to_string(),
        base_url: profile.openai_chat_completions_url(),
        model: request_model_for_capability(profile, request),
        supports_native_tool_history: Some(supports_native_tool_history),
        created_at_ms: now,
        last_used_at_ms: now,
    };
    save_pretty_json(path, &cache)
}

fn capability_cache_expired(cache: &CapabilityCacheFile, profile: &LlmProfile) -> bool {
    let ttl_ms = u128::from(capability_cache_ttl_hours(profile)).saturating_mul(60 * 60 * 1000);
    ttl_ms > 0
        && cache.last_used_at_ms > 0
        && now_millis().saturating_sub(cache.last_used_at_ms) > ttl_ms
}

fn capability_cache_file_path(profile: &LlmProfile, request: &Value) -> PathBuf {
    capability_cache_dir().join(format!("{}.json", capability_cache_key(profile, request)))
}

fn capability_cache_dir() -> PathBuf {
    proxy_optimizer::proxy_cache_dir().join("capabilities")
}

fn capability_cache_key(profile: &LlmProfile, request: &Value) -> String {
    stable_hash(&[
        CAPABILITY_CACHE_VERSION,
        &profile.openai_chat_completions_url(),
        &request_model_for_capability(profile, request),
        "tool-history",
    ])
}

fn request_model_for_capability(profile: &LlmProfile, request: &Value) -> String {
    request
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| profile.model.trim())
        .to_string()
}

fn capability_cache_ttl_hours(profile: &LlmProfile) -> u64 {
    env_u64(
        profile,
        "CLAUDIE_PROXY_CAPABILITY_CACHE_TTL_HOURS",
        DEFAULT_CAPABILITY_CACHE_TTL_HOURS,
    )
}

fn capability_cache_max_entries(profile: &LlmProfile) -> usize {
    env_usize(
        profile,
        "CLAUDIE_PROXY_CAPABILITY_CACHE_MAX_ENTRIES",
        DEFAULT_CAPABILITY_CACHE_MAX_ENTRIES,
    )
}

fn proxy_cache_max_bytes(profile: &LlmProfile) -> u64 {
    env_u64(
        profile,
        "CLAUDIE_PROXY_CACHE_MAX_MB",
        DEFAULT_PROXY_CACHE_MAX_BYTES / (1024 * 1024),
    )
    .saturating_mul(1024 * 1024)
}

fn env_usize(profile: &LlmProfile, key: &str, default: usize) -> usize {
    profile
        .extra_env_value(key)
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_u64(profile: &LlmProfile, key: &str, default: u64) -> u64 {
    profile
        .extra_env_value(key)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn stable_hash(parts: &[&str]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_history_capability_cache_forces_transcript_mode() {
        let profile = LlmProfile {
            base_url: format!(
                "https://example.invalid/{}/v1/chat/completions",
                now_millis()
            ),
            model: "gpt-test".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "model": "gpt-test",
            "messages": [{
                "role": "tool",
                "tool_call_id": "call_1",
                "content": "tool result"
            }]
        });
        let dir = std::env::temp_dir().join(format!("claudie-capability-cache-{}", now_millis()));
        let path = dir.join("capability.json");
        let _ = std::fs::remove_file(&path);

        assert!(!cached_tool_history_needs_transcript_at(&profile, &path));
        save_tool_history_capability_at(&profile, &request, false, &path).unwrap();
        assert!(cached_tool_history_needs_transcript_at(&profile, &path));

        let _ = std::fs::remove_dir_all(dir);
    }
}
