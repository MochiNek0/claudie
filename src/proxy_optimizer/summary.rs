use serde_json::{Value, json};
use std::path::Path;

use crate::settings::LlmProfile;

use super::cache::{
    chunk_cache_dir, chunk_summary_cache_key, load_chunk_summary, prune_chunk_cache,
    save_chunk_summary,
};
use super::compress::head_tail_compress;
use super::config::ProxyOptimizationConfig;
use super::{CHARS_PER_TOKEN, message_role};

const SUMMARY_MAX_TOKENS: u64 = 800;

pub(super) fn build_summary_request(request: &Value, old_messages: &[Value]) -> Value {
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

pub(super) fn local_summary_for_request(
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

pub(super) fn local_summary_with_chunk_cache_at(
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

pub(super) fn local_summary_from_messages(messages: &[Value], budget_tokens: usize) -> String {
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
