use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::settings::LlmProfile;

mod cache;
mod compress;
mod config;
mod summary;

#[cfg(test)]
mod tests;

pub(crate) use cache::{proxy_cache_dir, prune_cache_dir, save_summary};
pub(crate) use config::{ProxyOptimizationConfig, SummaryMode};

pub(super) const OPTIMIZER_VERSION: &str = "v5";
pub(super) const CHARS_PER_TOKEN: usize = 4;
const MESSAGE_ENVELOPE_CHARS: usize = 24;

#[derive(Clone, Debug)]
pub(crate) struct OptimizedRequest {
    /// Request ready to send upstream. `Value::Null` placeholder when
    /// `pending_summary` is `Some`; callers then use the pending summary's
    /// `request_with_summary`/`fallback_request` instead.
    pub(crate) request: Value,
    pub(crate) pending_summary: Option<PendingSummary>,
    // Optimizer outcome flags: asserted by the optimizer tests, not read in production.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) cache_hit: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) compressed: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) local_summary: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingSummary {
    pub(crate) cache_key: String,
    pub(crate) config: ProxyOptimizationConfig,
    pub(crate) summary_request: Value,
    pub(crate) fallback_request: Value,
    prefix_messages: Vec<Value>,
    recent_messages: Vec<Value>,
}

impl PendingSummary {
    pub(crate) fn request_with_summary(self, summary: &str) -> Value {
        let mut request = self.fallback_request;
        set_messages(
            &mut request,
            messages_with_summary(self.prefix_messages, summary, self.recent_messages),
        );
        request
    }
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
    let Some(mut messages) = take_messages(&mut request) else {
        return OptimizedRequest {
            request,
            pending_summary: None,
            cache_hit: false,
            compressed: false,
            local_summary: false,
        };
    };

    let compressed = compress::compress_messages_in_place(&mut messages, &config);
    let output_capped = cap_output_tokens(&mut request, config.max_output_tokens);

    if estimate_messages_tokens(&messages) <= config.summary_threshold_tokens {
        set_messages(&mut request, messages);
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
        set_messages(&mut request, messages);
        return OptimizedRequest {
            request,
            pending_summary: None,
            cache_hit: false,
            compressed: compressed || output_capped,
            local_summary: false,
        };
    }

    let cache_key = cache::summary_cache_key(
        profile,
        &request,
        &messages[prefix_len..recent_start],
        &config,
    );

    if let Some(summary) = cache::load_summary(&cache_key, &config) {
        let (prefix_messages, _old_messages, recent_messages) =
            split_into_segments(messages, prefix_len, recent_start);
        set_messages(
            &mut request,
            messages_with_summary(prefix_messages, &summary, recent_messages),
        );
        return OptimizedRequest {
            request,
            pending_summary: None,
            cache_hit: true,
            compressed: true,
            local_summary: false,
        };
    }

    if config.summary_mode == SummaryMode::Local {
        let summary = {
            let old_messages = &messages[prefix_len..recent_start];
            summary::local_summary_for_request(profile, &request, old_messages, &config)
        };
        let _ = save_summary(&cache_key, &summary, &config);
        let (prefix_messages, _old_messages, recent_messages) =
            split_into_segments(messages, prefix_len, recent_start);
        set_messages(
            &mut request,
            messages_with_summary(prefix_messages, &summary, recent_messages),
        );
        return OptimizedRequest {
            request,
            pending_summary: None,
            cache_hit: false,
            compressed: true,
            local_summary: true,
        };
    }

    let summary_request =
        summary::build_summary_request(&request, &messages[prefix_len..recent_start]);
    let (prefix_messages, old_messages, recent_messages) =
        split_into_segments(messages, prefix_len, recent_start);
    let mut combined =
        Vec::with_capacity(prefix_messages.len() + old_messages.len() + recent_messages.len());
    combined.extend_from_slice(&prefix_messages);
    combined.extend(old_messages);
    combined.extend_from_slice(&recent_messages);
    set_messages(&mut request, combined);
    OptimizedRequest {
        // The caller only consumes the pending summary's requests, so move the
        // full request into fallback_request instead of cloning it.
        request: Value::Null,
        pending_summary: Some(PendingSummary {
            cache_key,
            config,
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

fn take_messages(request: &mut Value) -> Option<Vec<Value>> {
    let array = request.get_mut("messages")?.as_array_mut()?;
    Some(std::mem::take(array))
}

fn split_into_segments(
    mut messages: Vec<Value>,
    prefix_len: usize,
    recent_start: usize,
) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let recent_messages = messages.split_off(recent_start);
    let old_messages = messages.split_off(prefix_len);
    (messages, old_messages, recent_messages)
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
        tokens = tokens.saturating_add(estimate_message_tokens(&messages[index]));
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

pub(super) fn messages_with_summary(
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

pub(super) fn set_messages(request: &mut Value, messages: Vec<Value>) {
    if let Some(object) = request.as_object_mut() {
        object.insert("messages".to_string(), Value::Array(messages));
    }
}

pub(super) fn message_role(message: &Value) -> &str {
    message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn estimate_message_chars(message: &Value) -> usize {
    let content_chars = match message.get("content") {
        Some(Value::String(text)) => text.chars().count(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|block| {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| block.as_str())
                    .map(|text| text.chars().count())
                    .unwrap_or(0)
            })
            .sum(),
        _ => 0,
    };
    let tool_calls_chars = message
        .get("tool_calls")
        .map(|value| value.to_string().chars().count())
        .unwrap_or(0);
    content_chars + tool_calls_chars + MESSAGE_ENVELOPE_CHARS
}

fn estimate_message_tokens(message: &Value) -> usize {
    estimate_message_chars(message).div_ceil(CHARS_PER_TOKEN)
}

fn estimate_messages_tokens(messages: &[Value]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

pub(super) fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
