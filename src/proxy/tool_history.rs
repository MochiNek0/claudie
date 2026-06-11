use serde_json::{Value, json};

use super::response_conv::{openai_function_arguments_to_value, openai_message_content_to_text};
use super::upstream::UpstreamError;

pub(super) fn should_retry_with_tool_transcript(err: &UpstreamError, request: &Value) -> bool {
    if !request_has_tool_history(request) {
        return false;
    }
    if !matches!(err.status, Some(400) | Some(422)) {
        return false;
    }
    let message = err.message.to_ascii_lowercase();
    ["tool", "tool_call", "messages", "role"]
        .iter()
        .any(|needle| message.contains(needle))
}

pub(super) fn request_has_tool_history(request: &Value) -> bool {
    request
        .get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message.get("role").and_then(Value::as_str) == Some("tool")
                    || message.get("tool_calls").is_some()
                    || message.get("function_call").is_some()
            })
        })
}

pub(super) fn tool_history_as_text_transcript(request: &Value) -> Value {
    let mut request = request.clone();
    let Some(object) = request.as_object_mut() else {
        return request;
    };
    let Some(messages) = object.get("messages").and_then(Value::as_array) else {
        return request;
    };
    let converted = messages
        .iter()
        .map(tool_message_as_text_transcript)
        .collect::<Vec<_>>();
    object.insert("messages".to_string(), Value::Array(converted));
    request
}

fn tool_message_as_text_transcript(message: &Value) -> Value {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user");
    if role == "tool" {
        let tool_call_id = message
            .get("tool_call_id")
            .and_then(Value::as_str)
            .unwrap_or("tool_call");
        return json!({
            "role": "user",
            "content": format!(
                "[Claude Code tool result for {tool_call_id}]\n{}",
                openai_message_content_to_text(message.get("content").unwrap_or(&Value::Null))
            )
        });
    }

    let mut converted = message.clone();
    let Some(object) = converted.as_object_mut() else {
        return converted;
    };
    if role == "assistant"
        && (object.get("tool_calls").is_some() || object.get("function_call").is_some())
    {
        let mut parts = Vec::new();
        let existing =
            openai_message_content_to_text(object.get("content").unwrap_or(&Value::Null));
        if !existing.trim().is_empty() {
            parts.push(existing);
        }
        if let Some(function_call) = object.get("function_call") {
            parts.push(format_openai_function_call("function_call", function_call));
        }
        if let Some(tool_calls) = object.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                let id = tool_call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_call");
                if let Some(function_call) = tool_call.get("function") {
                    parts.push(format_openai_function_call(id, function_call));
                }
            }
        }
        object.insert("content".to_string(), Value::String(parts.join("\n")));
        object.remove("tool_calls");
        object.remove("function_call");
    }
    converted
}

fn format_openai_function_call(id: &str, function_call: &Value) -> String {
    let name = function_call
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let arguments = openai_function_arguments_to_value(function_call.get("arguments")).to_string();
    format!("[Claude Code tool call {id}: {name}]\n{arguments}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_tool_history_as_text_transcript_for_compat_retry() {
        let request = json!({
            "model": "gpt-test",
            "messages": [
                {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "Read",
                            "arguments": "{\"file_path\":\"README.md\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "content": "README contents"
                }
            ]
        });

        assert!(should_retry_with_tool_transcript(
            &UpstreamError {
                status: Some(400),
                message: "OpenAI proxy upstream returned HTTP 400: invalid role tool".to_string(),
                retry_after: None,
            },
            &request
        ));
        let converted = tool_history_as_text_transcript(&request);
        let messages = converted["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "assistant");
        assert!(messages[0].get("tool_calls").is_none());
        assert!(messages[0]["content"].as_str().unwrap().contains("Read"));
        assert_eq!(messages[1]["role"], "user");
        assert!(
            messages[1]["content"]
                .as_str()
                .unwrap()
                .contains("README contents")
        );
    }

    #[test]
    fn transcript_retry_requires_structured_4xx_status() {
        let request = json!({
            "model": "gpt-test",
            "messages": [{
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "Bash", "arguments": "{}" }
                }]
            }]
        });

        // Network-level failure whose text merely mentions "http 400 tool"
        // must not trigger a transcript retry: real 400/422 responses always
        // carry a structured status.
        assert!(!should_retry_with_tool_transcript(
            &UpstreamError {
                status: None,
                message: "connection reset while sending http 400 tool history".to_string(),
                retry_after: None,
            },
            &request
        ));
        assert!(!should_retry_with_tool_transcript(
            &UpstreamError {
                status: Some(500),
                message: "OpenAI proxy upstream returned HTTP 500: tool messages rejected"
                    .to_string(),
                retry_after: None,
            },
            &request
        ));
        assert!(should_retry_with_tool_transcript(
            &UpstreamError {
                status: Some(422),
                message: "OpenAI proxy upstream returned HTTP 422: unknown role".to_string(),
                retry_after: None,
            },
            &request
        ));
    }

    #[test]
    fn openai_missing_tool_response_error_triggers_transcript_retry() {
        let request = json!({
            "model": "gpt-test",
            "messages": [{
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "Bash", "arguments": "{}" }
                }]
            }]
        });

        assert!(should_retry_with_tool_transcript(
            &UpstreamError {
                status: Some(400),
                message: "OpenAI proxy upstream returned HTTP 400: An assistant message with 'tool_calls' must be followed by tool messages responding to each 'tool_call_id'."
                    .to_string(),
                retry_after: None,
            },
            &request
        ));
    }
}
