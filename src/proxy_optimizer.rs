use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::settings::storage::{read_json_or_default, save_pretty_json};
use crate::settings::{LlmProfile, claudie_home};

const OPTIMIZER_VERSION: &str = "v2";
const DEFAULT_SUMMARY_THRESHOLD_TOKENS: usize = 12_000;
const DEFAULT_KEEP_RECENT_MESSAGES: usize = 8;
const DEFAULT_KEEP_RECENT_TOKENS: usize = 6_000;
const DEFAULT_TOOL_RESULT_LIMIT_TOKENS: usize = 2_000;
const DEFAULT_TEXT_LIMIT_TOKENS: usize = 4_000;
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 4_096;
const DEFAULT_LOCAL_SUMMARY_TOKENS: usize = 2_000;
const SUMMARY_MAX_TOKENS: u64 = 800;
const CHARS_PER_TOKEN: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SummaryMode {
    Local,
    Model,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProxyOptimizationConfig {
    pub(crate) enabled: bool,
    pub(crate) summary_threshold_tokens: usize,
    pub(crate) keep_recent_messages: usize,
    pub(crate) keep_recent_tokens: usize,
    pub(crate) tool_result_limit_tokens: usize,
    pub(crate) text_limit_tokens: usize,
    pub(crate) max_output_tokens: u64,
    pub(crate) local_summary_tokens: usize,
    pub(crate) summary_mode: SummaryMode,
}

impl ProxyOptimizationConfig {
    pub(crate) fn from_profile(profile: &LlmProfile) -> Self {
        let mut config = Self::default();
        if let Some(value) = profile.extra_env_value("CLAUDIE_PROXY_OPTIMIZE") {
            config.enabled = !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            );
        }
        config.summary_threshold_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_SUMMARY_THRESHOLD",
            config.summary_threshold_tokens,
        );
        config.keep_recent_messages = env_usize(
            profile,
            "CLAUDIE_PROXY_KEEP_RECENT_MESSAGES",
            config.keep_recent_messages,
        );
        config.keep_recent_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_KEEP_RECENT_TOKENS",
            config.keep_recent_tokens,
        );
        config.tool_result_limit_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_TOOL_RESULT_LIMIT",
            config.tool_result_limit_tokens,
        );
        config.text_limit_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_TEXT_LIMIT",
            config.text_limit_tokens,
        );
        config.max_output_tokens = env_u64_allow_zero(
            profile,
            "CLAUDIE_PROXY_MAX_OUTPUT_TOKENS",
            config.max_output_tokens,
        );
        config.local_summary_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_LOCAL_SUMMARY_TOKENS",
            config.local_summary_tokens,
        );
        if let Some(value) = profile.extra_env_value("CLAUDIE_PROXY_SUMMARY_MODE") {
            config.summary_mode = match value.trim().to_ascii_lowercase().as_str() {
                "model" | "remote" | "llm" => SummaryMode::Model,
                _ => SummaryMode::Local,
            };
        }
        config
    }

    fn signature(&self) -> String {
        format!(
            "{OPTIMIZER_VERSION}:{}:{}:{}:{}:{}:{}:{}:{:?}",
            self.summary_threshold_tokens,
            self.keep_recent_messages,
            self.keep_recent_tokens,
            self.tool_result_limit_tokens,
            self.text_limit_tokens,
            self.max_output_tokens,
            self.local_summary_tokens,
            self.summary_mode
        )
    }
}

impl Default for ProxyOptimizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            summary_threshold_tokens: DEFAULT_SUMMARY_THRESHOLD_TOKENS,
            keep_recent_messages: DEFAULT_KEEP_RECENT_MESSAGES,
            keep_recent_tokens: DEFAULT_KEEP_RECENT_TOKENS,
            tool_result_limit_tokens: DEFAULT_TOOL_RESULT_LIMIT_TOKENS,
            text_limit_tokens: DEFAULT_TEXT_LIMIT_TOKENS,
            max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
            local_summary_tokens: DEFAULT_LOCAL_SUMMARY_TOKENS,
            summary_mode: SummaryMode::Local,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OptimizedRequest {
    pub(crate) request: Value,
    pub(crate) pending_summary: Option<PendingSummary>,
    pub(crate) cache_hit: bool,
    pub(crate) compressed: bool,
    pub(crate) local_summary: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingSummary {
    pub(crate) cache_key: String,
    pub(crate) summary_request: Value,
    pub(crate) fallback_request: Value,
    prefix_messages: Vec<Value>,
    recent_messages: Vec<Value>,
}

impl PendingSummary {
    pub(crate) fn request_with_summary(&self, summary: &str) -> Value {
        let mut request = self.fallback_request.clone();
        set_messages(
            &mut request,
            messages_with_summary(
                self.prefix_messages.clone(),
                summary,
                self.recent_messages.clone(),
            ),
        );
        request
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SummaryCache {
    #[serde(default)]
    version: String,
    #[serde(default)]
    entries: BTreeMap<String, CachedSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedSummary {
    summary: String,
    created_at_ms: u128,
}

pub(crate) fn optimize_openai_request(request: Value, profile: &LlmProfile) -> OptimizedRequest {
    let config = ProxyOptimizationConfig::from_profile(profile);
    if !config.enabled {
        return OptimizedRequest {
            request,
            pending_summary: None,
            cache_hit: false,
            compressed: false,
            local_summary: false,
        };
    }

    let mut request = request;
    let Some(original_messages) = request.get("messages").and_then(Value::as_array).cloned() else {
        return OptimizedRequest {
            request,
            pending_summary: None,
            cache_hit: false,
            compressed: false,
            local_summary: false,
        };
    };

    let (messages, compressed) = compress_messages(&original_messages, &config);
    set_messages(&mut request, messages.clone());
    let output_capped = cap_output_tokens(&mut request, config.max_output_tokens);

    if estimate_tokens(&request) <= config.summary_threshold_tokens {
        return OptimizedRequest {
            request,
            pending_summary: None,
            cache_hit: false,
            compressed: compressed || output_capped,
            local_summary: false,
        };
    }

    let prefix_len = leading_system_count(&messages);
    let recent_start = recent_start_index(&messages, prefix_len, &config);
    if recent_start <= prefix_len {
        return OptimizedRequest {
            request,
            pending_summary: None,
            cache_hit: false,
            compressed: compressed || output_capped,
            local_summary: false,
        };
    }

    let prefix_messages = messages[..prefix_len].to_vec();
    let old_messages = messages[prefix_len..recent_start].to_vec();
    let recent_messages = messages[recent_start..].to_vec();
    let cache_key = summary_cache_key(profile, &request, &old_messages, &config);

    if let Some(summary) = load_summary(&cache_key) {
        let mut request_with_summary = request;
        set_messages(
            &mut request_with_summary,
            messages_with_summary(prefix_messages, &summary, recent_messages),
        );
        return OptimizedRequest {
            request: request_with_summary,
            pending_summary: None,
            cache_hit: true,
            compressed: true,
            local_summary: false,
        };
    }

    if config.summary_mode == SummaryMode::Local {
        let summary = local_summary_from_messages(&old_messages, config.local_summary_tokens);
        if let Err(_err) = save_summary(&cache_key, &summary) {}
        let mut request_with_summary = request;
        set_messages(
            &mut request_with_summary,
            messages_with_summary(prefix_messages, &summary, recent_messages),
        );
        return OptimizedRequest {
            request: request_with_summary,
            pending_summary: None,
            cache_hit: false,
            compressed: true,
            local_summary: true,
        };
    }

    let summary_request = build_summary_request(&request, &old_messages);
    OptimizedRequest {
        request: request.clone(),
        pending_summary: Some(PendingSummary {
            cache_key,
            summary_request,
            fallback_request: request,
            prefix_messages,
            recent_messages,
        }),
        cache_hit: false,
        compressed: true,
        local_summary: false,
    }
}

pub(crate) fn summary_text_from_openai_response(response: &Value) -> Option<String> {
    response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn save_summary(cache_key: &str, summary: &str) -> Result<(), String> {
    let mut cache: SummaryCache = read_json_or_default(&summary_cache_path());
    cache.version = OPTIMIZER_VERSION.to_string();
    cache.entries.insert(
        cache_key.to_string(),
        CachedSummary {
            summary: summary.to_string(),
            created_at_ms: now_millis(),
        },
    );
    save_pretty_json(&summary_cache_path(), &cache)
}

fn load_summary(cache_key: &str) -> Option<String> {
    let cache: SummaryCache = read_json_or_default(&summary_cache_path());
    if cache.version != OPTIMIZER_VERSION && !cache.version.is_empty() {
        return None;
    }
    cache
        .entries
        .get(cache_key)
        .map(|entry| entry.summary.clone())
        .filter(|summary| !summary.trim().is_empty())
}

fn summary_cache_path() -> std::path::PathBuf {
    claudie_home().join("proxy_summaries.json")
}

fn env_usize(profile: &LlmProfile, key: &str, default: usize) -> usize {
    profile
        .extra_env_value(key)
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_u64_allow_zero(profile: &LlmProfile, key: &str, default: u64) -> u64 {
    profile
        .extra_env_value(key)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn cap_output_tokens(request: &mut Value, cap: u64) -> bool {
    if cap == 0 {
        return false;
    }
    let mut changed = false;
    if let Some(object) = request.as_object_mut() {
        for key in ["max_tokens", "max_completion_tokens"] {
            if object
                .get(key)
                .and_then(Value::as_u64)
                .is_some_and(|value| value > cap)
            {
                object.insert(key.to_string(), Value::Number(cap.into()));
                changed = true;
            }
        }
    }
    changed
}

fn compress_messages(messages: &[Value], config: &ProxyOptimizationConfig) -> (Vec<Value>, bool) {
    let mut compressed = false;
    let messages = messages
        .iter()
        .map(|message| {
            let mut message = message.clone();
            let is_tool = message_role(&message) == "tool";
            let limit = if is_tool {
                config.tool_result_limit_tokens
            } else {
                config.text_limit_tokens
            };
            if compress_message_content(&mut message, limit, is_tool) {
                compressed = true;
            }
            message
        })
        .collect();
    (messages, compressed)
}

fn compress_message_content(message: &mut Value, limit_tokens: usize, is_tool: bool) -> bool {
    let Some(object) = message.as_object_mut() else {
        return false;
    };
    let Some(content) = object.get_mut("content") else {
        return false;
    };
    compress_content_value(content, limit_tokens, is_tool)
}

fn compress_content_value(content: &mut Value, limit_tokens: usize, is_tool: bool) -> bool {
    match content {
        Value::String(text) => {
            if let Some(compressed) = head_tail_compress(text, limit_tokens, is_tool) {
                *text = compressed;
                true
            } else {
                false
            }
        }
        Value::Array(blocks) => {
            let mut changed = false;
            for block in blocks {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    if let Some(compressed) = head_tail_compress(text, limit_tokens, is_tool) {
                        if let Some(object) = block.as_object_mut() {
                            object.insert("text".to_string(), Value::String(compressed));
                            changed = true;
                        }
                    }
                }
            }
            changed
        }
        _ => false,
    }
}

fn head_tail_compress(text: &str, limit_tokens: usize, is_tool: bool) -> Option<String> {
    let limit_chars = limit_tokens.saturating_mul(CHARS_PER_TOKEN);
    let char_count = text.chars().count();
    if char_count <= limit_chars {
        return None;
    }

    let label = if is_tool { "tool result" } else { "message" };
    let note = format!(
        "\n\n[claudie proxy omitted {} characters from a long {label} to reduce OpenAI proxy input tokens. The beginning and end are preserved.]\n\n",
        char_count.saturating_sub(limit_chars)
    );
    let available = limit_chars.saturating_sub(note.chars().count()).max(200);
    let head_len = available / 2;
    let tail_len = available.saturating_sub(head_len);
    Some(format!(
        "{}{}{}",
        take_chars(text, head_len),
        note,
        take_last_chars(text, tail_len)
    ))
}

fn leading_system_count(messages: &[Value]) -> usize {
    messages
        .iter()
        .take_while(|message| message_role(message) == "system")
        .count()
}

fn recent_start_index(
    messages: &[Value],
    prefix_len: usize,
    config: &ProxyOptimizationConfig,
) -> usize {
    let by_count = messages
        .len()
        .saturating_sub(config.keep_recent_messages)
        .max(prefix_len);
    let mut by_tokens = messages.len();
    let mut tokens = 0_usize;
    for index in (prefix_len..messages.len()).rev() {
        tokens = tokens.saturating_add(estimate_tokens(&messages[index]));
        by_tokens = index;
        if tokens >= config.keep_recent_tokens {
            break;
        }
    }
    let mut start = by_count.min(by_tokens).max(prefix_len);
    start = avoid_leading_tool_messages(messages, prefix_len, start);
    start
}

fn avoid_leading_tool_messages(messages: &[Value], prefix_len: usize, start: usize) -> usize {
    if start >= messages.len() || message_role(&messages[start]) != "tool" {
        return start;
    }
    let mut index = start;
    while index > prefix_len && message_role(&messages[index]) == "tool" {
        index -= 1;
    }
    index
}

fn build_summary_request(request: &Value, old_messages: &[Value]) -> Value {
    let model = request.get("model").cloned().unwrap_or_else(|| json!(""));
    json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "You summarize older Claude Code conversation context so a coding assistant can continue. Preserve user goals, decisions, constraints, file paths, tool results, errors, and pending tasks. Be concise, specific, and do not invent facts."
            },
            {
                "role": "user",
                "content": format!(
                    "Summarize these older messages for continuation. Keep durable facts and omit unimportant repetition.\n\nOlder messages JSON:\n{}",
                    Value::Array(old_messages.to_vec())
                )
            }
        ],
        "temperature": 0,
        "max_tokens": SUMMARY_MAX_TOKENS,
        "stream": false
    })
}

fn local_summary_from_messages(messages: &[Value], budget_tokens: usize) -> String {
    let budget_chars = budget_tokens.saturating_mul(CHARS_PER_TOKEN).max(1_000);
    let mut lines = Vec::new();
    lines.push(format!(
        "Local extractive summary of {} older messages. Full text was compacted to reduce OpenAI proxy cost; recent messages remain verbatim.",
        messages.len()
    ));
    let mut used_chars = lines[0].chars().count();
    for (index, message) in messages.iter().enumerate().rev() {
        if used_chars >= budget_chars {
            break;
        }
        let role = message_role(message);
        let mut detail = message_content_text(message);
        if detail.trim().is_empty() && message.get("tool_calls").is_some() {
            detail = format!("assistant tool calls: {}", message["tool_calls"]);
        }
        if detail.trim().is_empty() {
            continue;
        }
        let compressed = detail
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let remaining_chars = budget_chars.saturating_sub(used_chars);
        let per_message_tokens = (remaining_chars / CHARS_PER_TOKEN).min(350).max(80);
        let excerpt = head_tail_compress(&compressed, per_message_tokens, role == "tool")
            .unwrap_or(compressed);
        let line = format!("{}. {role}: {excerpt}", index + 1);
        used_chars = used_chars.saturating_add(line.chars().count() + 1);
        lines.push(line);
    }
    lines[1..].reverse();
    lines.join("\n")
}

fn message_content_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| block.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn messages_with_summary(
    mut prefix_messages: Vec<Value>,
    summary: &str,
    mut recent_messages: Vec<Value>,
) -> Vec<Value> {
    prefix_messages.push(json!({
        "role": "system",
        "content": format!(
            "Compressed older conversation context generated by claudie OpenAI proxy:\n\n{}",
            summary.trim()
        )
    }));
    prefix_messages.append(&mut recent_messages);
    prefix_messages
}

fn summary_cache_key(
    profile: &LlmProfile,
    request: &Value,
    old_messages: &[Value],
    config: &ProxyOptimizationConfig,
) -> String {
    let tools = request.get("tools").cloned().unwrap_or(Value::Null);
    stable_hash(&[
        OPTIMIZER_VERSION,
        profile.id.as_str(),
        request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        config.signature().as_str(),
        &Value::Array(old_messages.to_vec()).to_string(),
        &tools.to_string(),
    ])
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

fn set_messages(request: &mut Value, messages: Vec<Value>) {
    if let Some(object) = request.as_object_mut() {
        object.insert("messages".to_string(), Value::Array(messages));
    }
}

fn message_role(message: &Value) -> &str {
    message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn estimate_tokens(value: &Value) -> usize {
    value.to_string().chars().count().div_ceil(CHARS_PER_TOKEN)
}

fn take_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn take_last_chars(text: &str, count: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let start = chars.len().saturating_sub(count);
    chars[start..].iter().collect()
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> LlmProfile {
        LlmProfile {
            id: "test-profile".to_string(),
            model: "gpt-test".to_string(),
            ..LlmProfile::default()
        }
    }

    fn request_with_messages(messages: Vec<Value>) -> Value {
        json!({
            "model": "gpt-test",
            "messages": messages,
            "stream": false
        })
    }

    #[test]
    fn below_threshold_request_does_not_summarize() {
        let request = request_with_messages(vec![json!({ "role": "user", "content": "hello" })]);

        let optimized = optimize_openai_request(request.clone(), &profile());

        assert!(optimized.pending_summary.is_none());
        assert!(!optimized.cache_hit);
        assert_eq!(optimized.request, request);
    }

    #[test]
    fn disabled_optimizer_leaves_request_unchanged() {
        let mut profile = profile();
        profile.extra_env = "CLAUDIE_PROXY_OPTIMIZE=0".to_string();
        let request = request_with_messages(vec![json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "x".repeat(DEFAULT_TOOL_RESULT_LIMIT_TOKENS * CHARS_PER_TOKEN + 50)
        })]);

        let optimized = optimize_openai_request(request.clone(), &profile);

        assert_eq!(optimized.request, request);
        assert!(optimized.pending_summary.is_none());
        assert!(!optimized.compressed);
    }

    #[test]
    fn long_tool_result_is_head_tail_compressed() {
        let text = format!(
            "{}{}{}",
            "start-",
            "x".repeat(DEFAULT_TOOL_RESULT_LIMIT_TOKENS * CHARS_PER_TOKEN + 500),
            "-end"
        );
        let request = request_with_messages(vec![json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": text
        })]);

        let optimized = optimize_openai_request(request, &profile());
        let content = optimized.request["messages"][0]["content"]
            .as_str()
            .unwrap();

        assert!(optimized.compressed);
        assert!(content.starts_with("start-"));
        assert!(content.ends_with("-end"));
        assert!(content.contains("claudie proxy omitted"));
    }

    #[test]
    fn over_threshold_uses_local_summary_by_default_and_keeps_recent_messages() {
        let messages = (0..30)
            .map(|index| {
                json!({
                    "role": if index % 2 == 0 { "user" } else { "assistant" },
                    "content": format!("message-{index}-{}", "x".repeat(8_000))
                })
            })
            .collect::<Vec<_>>();
        let request = request_with_messages(messages);

        let optimized = optimize_openai_request(request, &profile());
        let output_messages = optimized.request["messages"].as_array().unwrap();

        assert!(optimized.local_summary);
        assert!(optimized.pending_summary.is_none());
        assert!(output_messages.iter().any(|message| {
            message["content"]
                .as_str()
                .unwrap_or("")
                .contains("Local extractive summary")
        }));
        assert!(output_messages.iter().any(|message| {
            message["content"]
                .as_str()
                .unwrap_or("")
                .contains("message-29")
        }));
    }

    #[test]
    fn model_summary_mode_creates_pending_summary() {
        let mut profile = profile();
        profile.extra_env = "CLAUDIE_PROXY_SUMMARY_MODE=model".to_string();
        let messages = (0..30)
            .map(|index| {
                json!({
                    "role": if index % 2 == 0 { "user" } else { "assistant" },
                    "content": format!("message-{index}-{}", "x".repeat(8_000))
                })
            })
            .collect::<Vec<_>>();
        let request = request_with_messages(messages);

        let optimized = optimize_openai_request(request, &profile);

        assert!(!optimized.local_summary);
        assert!(optimized.pending_summary.is_some());
    }

    #[test]
    fn local_summary_stays_within_budget() {
        let messages = (0..20)
            .map(|index| {
                json!({
                    "role": "tool",
                    "tool_call_id": format!("call_{index}"),
                    "content": format!("tool-result-{index}-{}", "x".repeat(20_000))
                })
            })
            .collect::<Vec<_>>();

        let summary = local_summary_from_messages(&messages, 1_000);

        assert!(estimate_tokens(&Value::String(summary)) <= 1_300);
    }

    #[test]
    fn output_token_budget_is_capped_by_default() {
        let request = json!({
            "model": "gpt-test",
            "messages": [{ "role": "user", "content": "hello" }],
            "max_tokens": 100_000_u64,
            "max_completion_tokens": 100_000_u64
        });

        let optimized = optimize_openai_request(request, &profile());

        assert_eq!(optimized.request["max_tokens"], DEFAULT_MAX_OUTPUT_TOKENS);
        assert_eq!(
            optimized.request["max_completion_tokens"],
            DEFAULT_MAX_OUTPUT_TOKENS
        );
        assert!(optimized.compressed);
    }

    #[test]
    fn output_token_cap_can_be_disabled() {
        let mut profile = profile();
        profile.extra_env = "CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=0".to_string();
        let request = json!({
            "model": "gpt-test",
            "messages": [{ "role": "user", "content": "hello" }],
            "max_tokens": 100_000_u64
        });

        let optimized = optimize_openai_request(request, &profile);

        assert_eq!(optimized.request["max_tokens"], 100_000_u64);
        assert!(!optimized.compressed);
    }

    #[test]
    fn cache_key_is_stable_and_changes_with_tools() {
        let config = ProxyOptimizationConfig::default();
        let old = vec![json!({ "role": "user", "content": "same old history" })];
        let request_a = json!({
            "model": "gpt-test",
            "tools": [{ "type": "function", "function": { "name": "Read" } }]
        });
        let request_b = json!({
            "model": "gpt-test",
            "tools": [{ "type": "function", "function": { "name": "Write" } }]
        });

        let key_a1 = summary_cache_key(&profile(), &request_a, &old, &config);
        let key_a2 = summary_cache_key(&profile(), &request_a, &old, &config);
        let key_b = summary_cache_key(&profile(), &request_b, &old, &config);

        assert_eq!(key_a1, key_a2);
        assert_ne!(key_a1, key_b);
    }

    #[test]
    fn summary_text_extracts_openai_message_content() {
        let response = json!({
            "choices": [{ "message": { "content": " summary text " } }]
        });

        assert_eq!(
            summary_text_from_openai_response(&response).as_deref(),
            Some("summary text")
        );
    }

    #[test]
    fn leading_tool_messages_pull_in_previous_assistant() {
        let messages = vec![
            json!({ "role": "user", "content": "old" }),
            json!({ "role": "assistant", "content": null, "tool_calls": [{ "id": "c1" }] }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "result" }),
            json!({ "role": "user", "content": "next" }),
        ];

        assert_eq!(avoid_leading_tool_messages(&messages, 0, 2), 1);
    }
}
