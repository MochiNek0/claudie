use serde_json::Value;

use super::config::ProxyOptimizationConfig;
use super::{CHARS_PER_TOKEN, message_role};

pub(super) fn compress_messages_in_place(
    messages: &mut [Value],
    config: &ProxyOptimizationConfig,
) -> bool {
    let mut compressed = false;
    for message in messages.iter_mut() {
        let is_tool = message_role(message) == "tool";
        let limit = if is_tool {
            config.tool_result_limit_tokens
        } else {
            config.text_limit_tokens
        };
        if compress_message_content(message, limit, is_tool) {
            compressed = true;
        }
    }
    compressed
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

pub(super) fn head_tail_compress(text: &str, limit_tokens: usize, is_tool: bool) -> Option<String> {
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

fn take_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn take_last_chars(text: &str, count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    match text.char_indices().rev().nth(count - 1) {
        Some((byte_idx, _)) => text[byte_idx..].to_string(),
        None => text.to_string(),
    }
}
