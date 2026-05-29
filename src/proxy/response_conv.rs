use serde_json::{Value, json};

use crate::settings::LlmProfile;

pub(super) fn openai_to_anthropic_response(
    openai: &Value,
    request: &Value,
    profile: &LlmProfile,
) -> Value {
    let choice = openai
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
    let mut content = Vec::new();

    // Reasoning models like DeepSeek-r1 / QwQ / GLM-Zero surface their chain of
    // thought in `reasoning_content` alongside the final answer in `content`.
    // Map it to an Anthropic `thinking` block so Claude Code shows it as the
    // collapsible thinking section users expect from native Claude.
    let reasoning = message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(reasoning) = reasoning {
        content.push(json!({ "type": "thinking", "thinking": reasoning }));
    }

    let text = openai_message_content_to_text(message.get("content").unwrap_or(&Value::Null));
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }

    if let Some(block) = anthropic_tool_use_from_openai_function_call(&message) {
        content.push(block);
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for tool_call in tool_calls {
            if let Some(block) = anthropic_tool_use_from_openai(tool_call) {
                content.push(block);
            }
        }
    }

    if content.is_empty() {
        content.push(json!({ "type": "text", "text": "" }));
    }

    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop");
    let stop_reason = match finish_reason {
        "tool_calls" | "function_call" => "tool_use",
        "length" => "max_tokens",
        "content_filter" => "stop_sequence",
        _ => "end_turn",
    };

    let prompt_tokens = openai
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = openai
        .get("usage")
        .map(cached_input_tokens)
        .unwrap_or(0)
        .min(prompt_tokens);

    json!({
        "id": openai.get("id").and_then(Value::as_str).unwrap_or("msg_claudie_proxy"),
        "type": "message",
        "role": "assistant",
        "model": response_model(openai, request, profile),
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": {
            // Anthropic reports cache reads separately from input_tokens, while OpenAI
            // folds cached prompt tokens into prompt_tokens. Subtract so Claude Code's
            // /cost accounting matches native behaviour.
            "input_tokens": prompt_tokens - cache_read,
            "output_tokens": openai.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0),
            "cache_read_input_tokens": cache_read,
            "cache_creation_input_tokens": 0
        }
    })
}

/// Extract the number of prompt tokens served from the upstream's prompt cache.
/// OpenAI exposes `usage.prompt_tokens_details.cached_tokens`; DeepSeek uses
/// `usage.prompt_cache_hit_tokens`. Returns 0 when neither is present.
pub(super) fn cached_input_tokens(usage: &Value) -> u64 {
    usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64))
        .unwrap_or(0)
}

fn anthropic_tool_use_from_openai(tool_call: &Value) -> Option<Value> {
    let function = tool_call.get("function")?;
    let name = function.get("name").and_then(Value::as_str)?;
    let input = openai_function_arguments_to_value(function.get("arguments"));
    Some(json!({
        "type": "tool_use",
        "id": tool_call.get("id").and_then(Value::as_str).unwrap_or("tool_call"),
        "name": name,
        "input": input
    }))
}

fn anthropic_tool_use_from_openai_function_call(message: &Value) -> Option<Value> {
    let function = message.get("function_call")?;
    let name = function.get("name").and_then(Value::as_str)?;
    Some(json!({
        "type": "tool_use",
        "id": "function_call",
        "name": name,
        "input": openai_function_arguments_to_value(function.get("arguments"))
    }))
}

pub(super) fn openai_function_arguments_to_value(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::String(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| json!({ "arguments": text }))
        }
        Some(Value::Object(_)) => arguments.cloned().unwrap_or_else(|| json!({})),
        Some(Value::Null) | None => json!({}),
        Some(other) => json!({ "arguments": other }),
    }
}

fn response_model(openai: &Value, request: &Value, profile: &LlmProfile) -> String {
    openai
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| request.get("model").and_then(Value::as_str))
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .or_else(|| {
            let model = profile.model.trim();
            (!model.is_empty()).then_some(model)
        })
        .unwrap_or("claudie-openai-proxy")
        .to_string()
}

pub(super) fn openai_message_content_to_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.clone()),
                Value::Object(object) => object
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        object
                            .get("content")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    }),
                _ => None,
            })
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_openai_tool_call_response() {
        let profile = LlmProfile {
            model: "gpt-test".to_string(),
            ..LlmProfile::default()
        };
        let response = json!({
            "id": "chatcmpl-1",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "Read", "arguments": "{\"file_path\":\"a.txt\"}" }
                    }]
                }
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 3 }
        });

        let converted = openai_to_anthropic_response(&response, &json!({}), &profile);
        assert_eq!(converted["stop_reason"], "tool_use");
        assert_eq!(converted["content"][0]["type"], "tool_use");
        assert_eq!(converted["content"][0]["input"]["file_path"], "a.txt");
    }

    #[test]
    fn converts_legacy_openai_function_call_response() {
        let profile = LlmProfile {
            model: "gpt-test".to_string(),
            ..LlmProfile::default()
        };
        let response = json!({
            "id": "chatcmpl-1",
            "choices": [{
                "finish_reason": "function_call",
                "message": {
                    "function_call": {
                        "name": "Read",
                        "arguments": "{\"file_path\":\"README.md\"}"
                    }
                }
            }]
        });

        let converted = openai_to_anthropic_response(&response, &json!({}), &profile);
        assert_eq!(converted["stop_reason"], "tool_use");
        assert_eq!(converted["content"][0]["type"], "tool_use");
        assert_eq!(converted["content"][0]["id"], "function_call");
        assert_eq!(converted["content"][0]["input"]["file_path"], "README.md");
    }

    #[test]
    fn accepts_object_tool_call_arguments() {
        let block = anthropic_tool_use_from_openai(&json!({
            "id": "call_1",
            "type": "function",
            "function": {
                "name": "Read",
                "arguments": { "file_path": "AGENTS.md" }
            }
        }))
        .unwrap();

        assert_eq!(block["input"]["file_path"], "AGENTS.md");
    }

    #[test]
    fn non_streaming_response_extracts_reasoning_content_as_thinking_block() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-reasoner".to_string(),
            ..LlmProfile::default()
        };
        let response = json!({
            "id": "chatcmpl-r1",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "reasoning_content": "Let me think step by step...",
                    "content": "The answer is 42."
                }
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 8 }
        });
        let converted = openai_to_anthropic_response(&response, &json!({}), &profile);
        let content = converted["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["thinking"], "Let me think step by step...");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "The answer is 42.");
    }

    #[test]
    fn non_streaming_response_without_reasoning_keeps_legacy_shape() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let response = json!({
            "id": "c",
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "hello" }
            }]
        });
        let converted = openai_to_anthropic_response(&response, &json!({}), &profile);
        let content = converted["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "hello");
    }

    #[test]
    fn empty_or_whitespace_reasoning_content_is_skipped() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-reasoner".to_string(),
            ..LlmProfile::default()
        };
        let response = json!({
            "id": "c",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "reasoning_content": "   \n  ",
                    "content": "answer"
                }
            }]
        });
        let converted = openai_to_anthropic_response(&response, &json!({}), &profile);
        let content = converted["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn maps_cached_tokens_to_cache_read_and_subtracts_from_input() {
        let profile = LlmProfile {
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let response = json!({
            "id": "c",
            "choices": [{ "finish_reason": "stop", "message": { "content": "ok" } }],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 20,
                "prompt_tokens_details": { "cached_tokens": 800 }
            }
        });
        let converted = openai_to_anthropic_response(&response, &json!({}), &profile);
        assert_eq!(converted["usage"]["input_tokens"], 200);
        assert_eq!(converted["usage"]["cache_read_input_tokens"], 800);
        assert_eq!(converted["usage"]["cache_creation_input_tokens"], 0);
    }

    #[test]
    fn maps_deepseek_prompt_cache_hit_tokens() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let response = json!({
            "id": "c",
            "choices": [{ "finish_reason": "stop", "message": { "content": "ok" } }],
            "usage": {
                "prompt_tokens": 500,
                "completion_tokens": 10,
                "prompt_cache_hit_tokens": 500
            }
        });
        let converted = openai_to_anthropic_response(&response, &json!({}), &profile);
        assert_eq!(converted["usage"]["input_tokens"], 0);
        assert_eq!(converted["usage"]["cache_read_input_tokens"], 500);
    }
}
