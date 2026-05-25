use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::settings::storage::{read_json_or_default, save_pretty_json};
use crate::settings::{LlmProfile, claudie_home};

const OPTIMIZER_VERSION: &str = "v4";
const DEFAULT_SUMMARY_THRESHOLD_TOKENS: usize = 24_000;
const DEFAULT_KEEP_RECENT_MESSAGES: usize = 12;
const DEFAULT_KEEP_RECENT_TOKENS: usize = 10_000;
const DEFAULT_TOOL_RESULT_LIMIT_TOKENS: usize = 3_000;
const DEFAULT_TEXT_LIMIT_TOKENS: usize = 6_000;
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 4_096;
const DEFAULT_LOCAL_SUMMARY_TOKENS: usize = 2_000;
const DEFAULT_CACHE_MAX_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_SUMMARY_CACHE_TTL_HOURS: u64 = 168;
const DEFAULT_SUMMARY_CACHE_MAX_ENTRIES: usize = 200;
const DEFAULT_CHUNK_SIZE_MESSAGES: usize = 8;
const DEFAULT_CHUNK_CACHE_TTL_HOURS: u64 = 168;
const DEFAULT_CHUNK_CACHE_MAX_ENTRIES: usize = 200;
const SUMMARY_MAX_TOKENS: u64 = 800;
const CHARS_PER_TOKEN: usize = 4;
const MILLIS_PER_HOUR: u128 = 60 * 60 * 1000;

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
    pub(crate) cache_max_bytes: u64,
    pub(crate) summary_cache_ttl_hours: u64,
    pub(crate) summary_cache_max_entries: usize,
    pub(crate) chunk_summary_enabled: bool,
    pub(crate) chunk_size_messages: usize,
    pub(crate) chunk_cache_ttl_hours: u64,
    pub(crate) chunk_cache_max_entries: usize,
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
        config.cache_max_bytes = env_u64(
            profile,
            "CLAUDIE_PROXY_CACHE_MAX_MB",
            config.cache_max_bytes / (1024 * 1024),
        )
        .saturating_mul(1024 * 1024);
        config.summary_cache_ttl_hours = env_u64(
            profile,
            "CLAUDIE_PROXY_SUMMARY_CACHE_TTL_HOURS",
            config.summary_cache_ttl_hours,
        );
        config.summary_cache_max_entries = env_usize(
            profile,
            "CLAUDIE_PROXY_SUMMARY_CACHE_MAX_ENTRIES",
            config.summary_cache_max_entries,
        );
        config.chunk_summary_enabled = env_bool(
            profile,
            "CLAUDIE_PROXY_CHUNK_SUMMARY",
            config.chunk_summary_enabled,
        );
        config.chunk_size_messages = env_usize(
            profile,
            "CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES",
            config.chunk_size_messages,
        );
        config.chunk_cache_ttl_hours = env_u64(
            profile,
            "CLAUDIE_PROXY_CHUNK_CACHE_TTL_HOURS",
            config.chunk_cache_ttl_hours,
        );
        config.chunk_cache_max_entries = env_usize(
            profile,
            "CLAUDIE_PROXY_CHUNK_CACHE_MAX_ENTRIES",
            config.chunk_cache_max_entries,
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
            "{OPTIMIZER_VERSION}:{}:{}:{}:{}:{}:{}:{}:{:?}:{}:{}",
            self.summary_threshold_tokens,
            self.keep_recent_messages,
            self.keep_recent_tokens,
            self.tool_result_limit_tokens,
            self.text_limit_tokens,
            self.max_output_tokens,
            self.local_summary_tokens,
            self.summary_mode,
            self.chunk_summary_enabled,
            self.chunk_size_messages
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
            cache_max_bytes: DEFAULT_CACHE_MAX_BYTES,
            summary_cache_ttl_hours: DEFAULT_SUMMARY_CACHE_TTL_HOURS,
            summary_cache_max_entries: DEFAULT_SUMMARY_CACHE_MAX_ENTRIES,
            chunk_summary_enabled: true,
            chunk_size_messages: DEFAULT_CHUNK_SIZE_MESSAGES,
            chunk_cache_ttl_hours: DEFAULT_CHUNK_CACHE_TTL_HOURS,
            chunk_cache_max_entries: DEFAULT_CHUNK_CACHE_MAX_ENTRIES,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SummaryCacheFile {
    #[serde(default)]
    version: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    created_at_ms: u128,
    #[serde(default)]
    last_used_at_ms: u128,
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

    if let Some(summary) = load_summary(&cache_key, &config) {
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
        let summary = local_summary_for_request(profile, &request, &old_messages, &config);
        if let Err(_err) = save_summary(&cache_key, &summary, profile) {}
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

pub(crate) fn save_summary(
    cache_key: &str,
    summary: &str,
    profile: &LlmProfile,
) -> Result<(), String> {
    let config = ProxyOptimizationConfig::from_profile(profile);
    save_summary_with_config(cache_key, summary, &config)
}

fn save_summary_with_config(
    cache_key: &str,
    summary: &str,
    config: &ProxyOptimizationConfig,
) -> Result<(), String> {
    let now = now_millis();
    let entry = SummaryCacheFile {
        version: OPTIMIZER_VERSION.to_string(),
        kind: "summary".to_string(),
        summary: summary.to_string(),
        created_at_ms: now,
        last_used_at_ms: now,
    };
    let path = summary_cache_file_path(cache_key);
    save_pretty_json(&path, &entry)?;
    prune_summary_cache(config)
}

fn load_summary(cache_key: &str, config: &ProxyOptimizationConfig) -> Option<String> {
    let path = summary_cache_file_path(cache_key);
    let mut entry: SummaryCacheFile = read_json_or_default(&path);
    if entry.version == OPTIMIZER_VERSION && !entry.summary.trim().is_empty() {
        entry.last_used_at_ms = now_millis();
        let _ = save_pretty_json(&path, &entry);
        return Some(entry.summary);
    }

    let summary = load_legacy_summary(cache_key)?;
    let _ = save_summary_with_config(cache_key, &summary, config);
    Some(summary)
}

fn summary_cache_path() -> std::path::PathBuf {
    claudie_home().join("proxy_summaries.json")
}

fn load_legacy_summary(cache_key: &str) -> Option<String> {
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

pub(crate) fn proxy_cache_dir() -> PathBuf {
    claudie_home().join("proxy_cache")
}

fn summary_cache_dir() -> PathBuf {
    proxy_cache_dir().join("summaries")
}

fn summary_cache_file_path(cache_key: &str) -> PathBuf {
    summary_cache_dir().join(format!("{cache_key}.json"))
}

fn chunk_cache_dir() -> PathBuf {
    proxy_cache_dir().join("chunks")
}

fn prune_summary_cache(config: &ProxyOptimizationConfig) -> Result<(), String> {
    prune_cache_dir(
        &summary_cache_dir(),
        config.summary_cache_ttl_hours,
        config.summary_cache_max_entries,
        config.cache_max_bytes,
    )
}

fn prune_chunk_cache(config: &ProxyOptimizationConfig) -> Result<(), String> {
    prune_cache_dir(
        &chunk_cache_dir(),
        config.chunk_cache_ttl_hours,
        config.chunk_cache_max_entries,
        config.cache_max_bytes,
    )
}

pub(crate) fn prune_cache_dir(
    dir: &Path,
    ttl_hours: u64,
    max_entries: usize,
    max_bytes: u64,
) -> Result<(), String> {
    let now = now_millis();
    let ttl_ms = u128::from(ttl_hours).saturating_mul(MILLIS_PER_HOUR);
    let mut entries = cache_dir_entries(dir)?;

    for entry in entries.iter().filter(|entry| {
        ttl_ms > 0
            && entry.last_used_at_ms > 0
            && now.saturating_sub(entry.last_used_at_ms) > ttl_ms
    }) {
        let _ = fs::remove_file(&entry.path);
    }

    entries = cache_dir_entries(dir)?;
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.last_used_at_ms));
    let mut kept_bytes = 0_u64;
    for (index, entry) in entries.iter().enumerate() {
        kept_bytes = kept_bytes.saturating_add(entry.size_bytes);
        let too_many = max_entries > 0 && index >= max_entries;
        let too_large = max_bytes > 0 && kept_bytes > max_bytes;
        if too_many || too_large {
            let _ = fs::remove_file(&entry.path);
        }
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct CacheDirEntry {
    path: PathBuf,
    last_used_at_ms: u128,
    size_bytes: u64,
}

fn cache_dir_entries(dir: &Path) -> Result<Vec<CacheDirEntry>, String> {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for item in read_dir {
        let item = item.map_err(|err| err.to_string())?;
        let path = item.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let metadata = item.metadata().map_err(|err| err.to_string())?;
        if !metadata.is_file() {
            continue;
        }
        let entry: SummaryCacheFile = read_json_or_default(&path);
        entries.push(CacheDirEntry {
            path,
            last_used_at_ms: entry.last_used_at_ms.max(entry.created_at_ms),
            size_bytes: metadata.len(),
        });
    }
    Ok(entries)
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

fn env_bool(profile: &LlmProfile, key: &str, default: bool) -> bool {
    profile
        .extra_env_value(key)
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
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

fn local_summary_for_request(
    profile: &LlmProfile,
    request: &Value,
    messages: &[Value],
    config: &ProxyOptimizationConfig,
) -> String {
    if !config.chunk_summary_enabled || messages.len() <= config.chunk_size_messages {
        return local_summary_from_messages(messages, config.local_summary_tokens);
    }
    local_summary_with_chunk_cache_at(profile, request, messages, config, &chunk_cache_dir())
}

fn local_summary_with_chunk_cache_at(
    profile: &LlmProfile,
    request: &Value,
    messages: &[Value],
    config: &ProxyOptimizationConfig,
    chunk_dir: &Path,
) -> String {
    let chunk_size = config.chunk_size_messages.max(1);
    let chunk_count = messages.len().div_ceil(chunk_size);
    if chunk_count <= 1 {
        return local_summary_from_messages(messages, config.local_summary_tokens);
    }

    let per_chunk_tokens = (config.local_summary_tokens / chunk_count)
        .clamp(120, 500)
        .min(config.local_summary_tokens.max(120));
    let mut chunk_summaries = Vec::new();
    for (chunk_index, chunk) in messages.chunks(chunk_size).enumerate() {
        let start = chunk_index * chunk_size;
        let end = start + chunk.len();
        let cache_key = chunk_summary_cache_key(profile, request, chunk, config);
        let path = chunk_dir.join(format!("{cache_key}.json"));
        let summary = load_chunk_summary(&path).unwrap_or_else(|| {
            let summary = local_summary_from_messages(chunk, per_chunk_tokens);
            let _ = save_chunk_summary(&path, &summary, config);
            summary
        });
        chunk_summaries.push(format!(
            "Chunk {} (older messages {}-{}):\n{}",
            chunk_index + 1,
            start + 1,
            end,
            summary.trim()
        ));
    }

    let mut summary = format!(
        "Chunked local summary of {} older messages across {} cached chunks. Full text was compacted to reduce OpenAI proxy cost; recent messages remain verbatim.\n{}",
        messages.len(),
        chunk_count,
        chunk_summaries.join("\n\n")
    );
    let budget_chars = config.local_summary_tokens.saturating_mul(CHARS_PER_TOKEN);
    if summary.chars().count() > budget_chars {
        summary =
            head_tail_compress(&summary, config.local_summary_tokens, false).unwrap_or(summary);
    }
    let _ = prune_chunk_cache(config);
    summary
}

fn load_chunk_summary(path: &Path) -> Option<String> {
    let mut entry: SummaryCacheFile = read_json_or_default(path);
    if entry.version != OPTIMIZER_VERSION
        || entry.kind != "chunk_summary"
        || entry.summary.trim().is_empty()
    {
        return None;
    }
    entry.last_used_at_ms = now_millis();
    let _ = save_pretty_json(path, &entry);
    Some(entry.summary)
}

fn save_chunk_summary(
    path: &Path,
    summary: &str,
    config: &ProxyOptimizationConfig,
) -> Result<(), String> {
    let now = now_millis();
    let entry = SummaryCacheFile {
        version: OPTIMIZER_VERSION.to_string(),
        kind: "chunk_summary".to_string(),
        summary: summary.to_string(),
        created_at_ms: now,
        last_used_at_ms: now,
    };
    save_pretty_json(path, &entry)?;
    prune_chunk_cache(config)
}

fn local_summary_from_messages(messages: &[Value], budget_tokens: usize) -> String {
    let budget_chars = budget_tokens.saturating_mul(CHARS_PER_TOKEN).max(1_000);
    let mut lines = Vec::new();
    lines.push(format!(
        "Local extractive summary of {} older messages. Full text was compacted to reduce OpenAI proxy cost; recent messages remain verbatim.",
        messages.len()
    ));
    let mut used_chars = lines[0].chars().count();
    let original_user_index = messages.iter().position(|message| {
        message_role(message) == "user" && !message_summary_detail(message).trim().is_empty()
    });

    if let Some(index) = original_user_index {
        if let Some(line) = summary_line_for_message(
            index,
            &messages[index],
            budget_chars.saturating_sub(used_chars),
            Some("original user request"),
        ) {
            used_chars = used_chars.saturating_add(line.chars().count() + 1);
            lines.push(line);
        }
    }

    let mut recent_lines = Vec::new();
    for (index, message) in messages.iter().enumerate().rev() {
        if Some(index) == original_user_index {
            continue;
        }
        if used_chars >= budget_chars {
            break;
        }
        let remaining_chars = budget_chars.saturating_sub(used_chars);
        let Some(line) = summary_line_for_message(index, message, remaining_chars, None) else {
            continue;
        };
        used_chars = used_chars.saturating_add(line.chars().count() + 1);
        recent_lines.push(line);
    }
    recent_lines.reverse();
    lines.extend(recent_lines);
    lines.join("\n")
}

fn summary_line_for_message(
    index: usize,
    message: &Value,
    remaining_chars: usize,
    label_override: Option<&str>,
) -> Option<String> {
    let detail = message_summary_detail(message);
    if detail.trim().is_empty() {
        return None;
    }
    let role = label_override.unwrap_or_else(|| message_role(message));
    let per_message_tokens = (remaining_chars / CHARS_PER_TOKEN).min(350).max(80);
    let excerpt = head_tail_compress(&detail, per_message_tokens, message_role(message) == "tool")
        .unwrap_or(detail);
    Some(format!("{}. {role}: {excerpt}", index + 1))
}

fn message_summary_detail(message: &Value) -> String {
    let mut detail = message_content_text(message);
    if detail.trim().is_empty() && message.get("tool_calls").is_some() {
        detail = format!("assistant tool calls: {}", message["tool_calls"]);
    }
    detail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
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

fn chunk_summary_cache_key(
    profile: &LlmProfile,
    request: &Value,
    chunk_messages: &[Value],
    config: &ProxyOptimizationConfig,
) -> String {
    let tools = request.get("tools").cloned().unwrap_or(Value::Null);
    let config_signature = config.signature();
    let messages_json = Value::Array(chunk_messages.to_vec()).to_string();
    let tools_json = tools.to_string();
    stable_hash(&[
        OPTIMIZER_VERSION,
        "chunk",
        profile.id.as_str(),
        request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        config_signature.as_str(),
        messages_json.as_str(),
        tools_json.as_str(),
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
        profile_with_id("test-profile")
    }

    fn profile_with_id(id: &str) -> LlmProfile {
        LlmProfile {
            id: id.to_string(),
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
        let profile = profile_with_id(&format!("local-summary-test-{}", now_millis()));
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
    fn local_summary_preserves_original_user_goal() {
        let mut messages = vec![json!({
            "role": "user",
            "content": "Please optimize README and AGENTS, fill missing parts, and fix inaccurate parts."
        })];
        messages.extend((0..30).map(|index| {
            json!({
                "role": "tool",
                "tool_call_id": format!("call_{index}"),
                "content": format!("tool-result-{index}-{}", "x".repeat(20_000))
            })
        }));

        let summary = local_summary_from_messages(&messages, 250);

        assert!(summary.contains("original user request"));
        assert!(summary.contains("optimize README and AGENTS"));
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
    fn cache_dir_prune_removes_expired_and_over_limit_files() {
        let dir = std::env::temp_dir().join(format!("claudie-cache-prune-{}", now_millis()));
        let old = SummaryCacheFile {
            version: OPTIMIZER_VERSION.to_string(),
            kind: "summary".to_string(),
            summary: "old".to_string(),
            created_at_ms: 1,
            last_used_at_ms: 1,
        };
        let fresh_a = SummaryCacheFile {
            version: OPTIMIZER_VERSION.to_string(),
            kind: "summary".to_string(),
            summary: "fresh a".to_string(),
            created_at_ms: now_millis(),
            last_used_at_ms: now_millis(),
        };
        let fresh_b = SummaryCacheFile {
            version: OPTIMIZER_VERSION.to_string(),
            kind: "summary".to_string(),
            summary: "fresh b".to_string(),
            created_at_ms: now_millis() + 1,
            last_used_at_ms: now_millis() + 1,
        };
        let old_path = dir.join("old.json");
        let fresh_a_path = dir.join("fresh-a.json");
        let fresh_b_path = dir.join("fresh-b.json");
        save_pretty_json(&old_path, &old).unwrap();
        save_pretty_json(&fresh_a_path, &fresh_a).unwrap();
        save_pretty_json(&fresh_b_path, &fresh_b).unwrap();

        prune_cache_dir(&dir, 1, 1, 1024 * 1024).unwrap();

        assert!(!old_path.exists());
        assert!(!fresh_a_path.exists());
        assert!(fresh_b_path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn chunk_summary_cache_reuses_existing_chunk_file() {
        let dir = std::env::temp_dir().join(format!("claudie-chunk-cache-{}", now_millis()));
        let config = ProxyOptimizationConfig {
            chunk_size_messages: 2,
            local_summary_tokens: 2_000,
            ..ProxyOptimizationConfig::default()
        };
        let request = json!({
            "model": "gpt-test",
            "tools": [{ "type": "function", "function": { "name": "Read" } }]
        });
        let messages = vec![
            json!({ "role": "user", "content": "original user request: optimize proxy cache" }),
            json!({ "role": "assistant", "content": "I will inspect the cache" }),
            json!({ "role": "tool", "content": "cache file contents" }),
            json!({ "role": "assistant", "content": "I found a large JSON file" }),
            json!({ "role": "user", "content": "continue" }),
        ];
        let first_chunk_key =
            chunk_summary_cache_key(&profile(), &request, &messages[..2], &config);
        let first_chunk_path = dir.join(format!("{first_chunk_key}.json"));
        save_chunk_summary(&first_chunk_path, "cached first chunk marker", &config).unwrap();

        let summary =
            local_summary_with_chunk_cache_at(&profile(), &request, &messages, &config, &dir);

        assert!(summary.contains("Chunked local summary"));
        assert!(summary.contains("cached first chunk marker"));
        assert!(dir.read_dir().unwrap().count() >= 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn chunk_summary_can_be_disabled_from_profile_env() {
        let mut profile = profile();
        profile.extra_env = "CLAUDIE_PROXY_CHUNK_SUMMARY=0".to_string();

        let config = ProxyOptimizationConfig::from_profile(&profile);

        assert!(!config.chunk_summary_enabled);
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
