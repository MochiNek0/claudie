use serde_json::{Map, Value, json};

use crate::settings::LlmProfile;

use super::provider::{
    OPENAI_PROXY_COMPAT_PROMPT, Provider, TOOL_RESULT_IMAGE_PLACEHOLDER, VISION_PLACEHOLDER_TEXT,
    images_enabled_for, model_is_reasoning, model_requires_max_completion_tokens,
    model_supports_tools,
};

pub(super) fn anthropic_to_openai_request(
    request: &Value,
    profile: &LlmProfile,
) -> Result<Value, String> {
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| profile.model.trim())
        .to_string();
    let vision_enabled = images_enabled_for(profile, &model);

    let mut messages = Vec::new();
    let mut system_text = request
        .get("system")
        .map(content_to_text)
        .unwrap_or_default();
    if openai_proxy_compat_prompt_enabled(profile) {
        if !system_text.trim().is_empty() {
            system_text.push_str("\n\n");
        }
        system_text.push_str(OPENAI_PROXY_COMPAT_PROMPT);
    }
    if !system_text.trim().is_empty() {
        messages.push(json!({ "role": "system", "content": system_text }));
    }

    let Some(input_messages) = request.get("messages").and_then(Value::as_array) else {
        return Err("messages must be an array".to_string());
    };
    for message in input_messages {
        append_openai_messages(&mut messages, message, vision_enabled);
    }

    let mut out = Map::new();
    out.insert("model".to_string(), Value::String(model.clone()));
    out.insert("messages".to_string(), Value::Array(messages));
    out.insert("stream".to_string(), Value::Bool(false));

    let provider = Provider::detect(profile);
    // OpenAI o-series / gpt-5 models reject `max_tokens` and require
    // `max_completion_tokens`. Only OpenAI/Azure enforce this; OpenRouter
    // normalizes `max_tokens` itself and other providers never serve these models.
    if matches!(provider, Provider::OpenAI | Provider::Azure)
        && model_requires_max_completion_tokens(&model)
    {
        if let Some(value) = request.get("max_tokens").filter(|value| value.is_number()) {
            out.insert("max_completion_tokens".to_string(), value.clone());
        }
    } else {
        copy_number(request, &mut out, "max_tokens");
    }
    copy_number(request, &mut out, "temperature");
    copy_number(request, &mut out, "top_p");
    if let Some(stop) = request.get("stop_sequences") {
        out.insert("stop".to_string(), stop.clone());
    }

    let tools_supported = model_supports_tools(&model);
    if tools_supported {
        if let Some(tools) = request.get("tools").and_then(Value::as_array) {
            let converted = tools
                .iter()
                .filter_map(openai_tool_from_anthropic_tool)
                .collect::<Vec<_>>();
            if !converted.is_empty() {
                out.insert("tools".to_string(), Value::Array(converted));
                let parallel = !request
                    .pointer("/tool_choice/disable_parallel_tool_use")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                out.insert("parallel_tool_calls".to_string(), Value::Bool(parallel));
            }
        }

        if let Some(tool_choice) = request.get("tool_choice") {
            if let Some(converted) = openai_tool_choice(tool_choice) {
                out.insert("tool_choice".to_string(), converted);
            }
        }
    }

    // Auto-attach reasoning_effort for o-series / reasoning models when the caller
    // didn't supply one. Use Anthropic's thinking.budget_tokens (if present) to pick
    // a tier; otherwise default to "medium". Inserted BEFORE extra_body merge so the
    // user's explicit `openai_extra_body` value still wins. Gated on the provider
    // accepting this field — DeepSeek/Qwen/Kimi/GLM reject unknown params.
    if model_is_reasoning(&model) && provider.accepts_reasoning_effort() {
        let effort = anthropic_thinking_to_reasoning_effort(request);
        out.insert("reasoning_effort".to_string(), Value::String(effort));
    }

    for (key, value) in profile.openai_extra_body_fields()? {
        // Suppress tool-related fields from extra_body too when the model does not
        // accept them, so users with a stale OpenAI body do not regress.
        if !tools_supported
            && matches!(
                key.as_str(),
                "tools" | "tool_choice" | "parallel_tool_calls"
            )
        {
            continue;
        }
        out.insert(key, value);
    }

    Ok(Value::Object(out))
}

fn anthropic_thinking_to_reasoning_effort(request: &Value) -> String {
    let budget = request
        .pointer("/thinking/budget_tokens")
        .and_then(Value::as_u64);
    match budget {
        Some(n) if n >= 32_000 => "high".to_string(),
        Some(n) if n >= 8_000 => "medium".to_string(),
        Some(_) => "low".to_string(),
        None => "medium".to_string(),
    }
}

fn openai_proxy_compat_prompt_enabled(profile: &LlmProfile) -> bool {
    if let Some(value) = profile.extra_env_value("CLAUDIE_PROXY_COMPAT_PROMPT") {
        return !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        );
    }
    // Default: skip the compat sentence for recognized commercial providers (OpenAI/Azure/
    // DeepSeek/Qwen/Kimi/GLM/OpenRouter) so their automatic prefix-cache stays hot.
    // Only the Generic catch-all (incl. self-hosted OneAPI/NewAPI) gets it by default.
    Provider::detect(profile).compat_prompt_default_on()
}

fn append_openai_messages(messages: &mut Vec<Value>, message: &Value, vision_enabled: bool) {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user");
    let content = message.get("content").unwrap_or(&Value::Null);

    match role {
        "assistant" => {
            let (text, tool_calls) = assistant_content_to_openai(content);
            let mut out = Map::new();
            out.insert("role".to_string(), Value::String("assistant".to_string()));
            out.insert("content".to_string(), Value::String(text));
            if !tool_calls.is_empty() {
                out.insert("tool_calls".to_string(), Value::Array(tool_calls));
            }
            messages.push(Value::Object(out));
        }
        _ => append_user_content(messages, content, vision_enabled),
    }
}

#[derive(Debug)]
enum UserPart {
    Text(String),
    Image(Value),
}

fn flush_user_parts(messages: &mut Vec<Value>, parts: &mut Vec<UserPart>) {
    if parts.is_empty() {
        return;
    }
    let has_image = parts.iter().any(|p| matches!(p, UserPart::Image(_)));
    if !has_image {
        let text: Vec<String> = parts
            .drain(..)
            .filter_map(|p| match p {
                UserPart::Text(t) => Some(t),
                UserPart::Image(_) => None,
            })
            .collect();
        let joined = text.join("\n");
        if !joined.trim().is_empty() {
            messages.push(json!({ "role": "user", "content": joined }));
        }
        return;
    }

    // Mixed content with image(s). Collapse consecutive text parts so the array stays compact.
    let mut array: Vec<Value> = Vec::new();
    let mut text_buf = String::new();
    let flush_text = |array: &mut Vec<Value>, text_buf: &mut String| {
        if !text_buf.trim().is_empty() {
            array.push(json!({ "type": "text", "text": text_buf.clone() }));
        }
        text_buf.clear();
    };
    for part in parts.drain(..) {
        match part {
            UserPart::Text(t) => {
                if !text_buf.is_empty() {
                    text_buf.push('\n');
                }
                text_buf.push_str(&t);
            }
            UserPart::Image(image_url_block) => {
                flush_text(&mut array, &mut text_buf);
                array.push(image_url_block);
            }
        }
    }
    flush_text(&mut array, &mut text_buf);
    if !array.is_empty() {
        messages.push(json!({ "role": "user", "content": Value::Array(array) }));
    }
}

fn anthropic_image_to_openai_part(block: &Value, vision_enabled: bool) -> Option<UserPart> {
    let source = block.get("source")?;
    if !vision_enabled {
        return Some(UserPart::Text(VISION_PLACEHOLDER_TEXT.to_string()));
    }
    let source_type = source.get("type").and_then(Value::as_str)?;
    let url = match source_type {
        "base64" => {
            let media_type = source
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            let data = source.get("data").and_then(Value::as_str)?;
            format!("data:{media_type};base64,{data}")
        }
        "url" => source.get("url").and_then(Value::as_str)?.to_string(),
        _ => return None,
    };
    let mut image_url = Map::new();
    image_url.insert("url".to_string(), Value::String(url));
    if let Some(detail) = source.get("detail").and_then(Value::as_str) {
        image_url.insert("detail".to_string(), Value::String(detail.to_string()));
    }
    Some(UserPart::Image(json!({
        "type": "image_url",
        "image_url": Value::Object(image_url)
    })))
}

fn tool_result_content_to_openai(block: &Value, vision_enabled: bool) -> (String, Vec<Value>) {
    let raw = block.get("content").unwrap_or(&Value::Null);
    let mut text_chunks: Vec<String> = Vec::new();
    let mut image_parts: Vec<Value> = Vec::new();
    match raw {
        Value::String(text) => text_chunks.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = item.get("text").and_then(Value::as_str) {
                            text_chunks.push(t.to_string());
                        }
                    }
                    Some("image") => {
                        if let Some(UserPart::Image(image_value)) =
                            anthropic_image_to_openai_part(item, vision_enabled)
                        {
                            text_chunks.push(TOOL_RESULT_IMAGE_PLACEHOLDER.to_string());
                            image_parts.push(image_value);
                        } else if !vision_enabled {
                            text_chunks.push(VISION_PLACEHOLDER_TEXT.to_string());
                        }
                    }
                    _ => {
                        let fallback = content_to_text(item);
                        if !fallback.trim().is_empty() {
                            text_chunks.push(fallback);
                        }
                    }
                }
            }
        }
        Value::Null => {}
        other => text_chunks.push(content_to_text(other)),
    }
    (text_chunks.join("\n"), image_parts)
}

fn append_user_content(messages: &mut Vec<Value>, content: &Value, vision_enabled: bool) {
    if !content.is_array() {
        let text = content_to_text(content);
        if !text.trim().is_empty() {
            messages.push(json!({ "role": "user", "content": text }));
        }
        return;
    }

    if content
        .as_array()
        .expect("checked array")
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
    {
        append_tool_result_user_content(messages, content, vision_enabled);
        return;
    }

    let mut parts: Vec<UserPart> = Vec::new();
    for block in content.as_array().expect("checked array") {
        append_regular_user_block(&mut parts, block, vision_enabled);
    }
    flush_user_parts(messages, &mut parts);
}

fn append_tool_result_user_content(
    messages: &mut Vec<Value>,
    content: &Value,
    vision_enabled: bool,
) {
    let mut deferred_parts: Vec<UserPart> = Vec::new();
    for block in content.as_array().expect("checked array") {
        match block.get("type").and_then(Value::as_str) {
            Some("tool_result") => {
                let tool_call_id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_call");
                let (text_for_tool, image_followup) =
                    tool_result_content_to_openai(block, vision_enabled);
                // OpenAI tool messages have no is_error flag; mark failed tool
                // results with a deterministic textual prefix instead.
                let is_error = block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let text_for_tool = if is_error {
                    if text_for_tool.trim().is_empty() {
                        "[tool_error] tool execution failed".to_string()
                    } else {
                        format!("[tool_error] {text_for_tool}")
                    }
                } else {
                    text_for_tool
                };
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": text_for_tool
                }));
                if !image_followup.is_empty() {
                    deferred_parts.push(UserPart::Text(format!(
                        "Images attached for tool_result {tool_call_id}:"
                    )));
                    deferred_parts.extend(image_followup.into_iter().map(UserPart::Image));
                }
            }
            _ => append_regular_user_block(&mut deferred_parts, block, vision_enabled),
        }
    }
    flush_user_parts(messages, &mut deferred_parts);
}

fn append_regular_user_block(parts: &mut Vec<UserPart>, block: &Value, vision_enabled: bool) {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                parts.push(UserPart::Text(text.to_string()));
            }
        }
        Some("image") => {
            if let Some(part) = anthropic_image_to_openai_part(block, vision_enabled) {
                parts.push(part);
            }
        }
        _ => {
            let text = content_to_text(block);
            if !text.trim().is_empty() {
                parts.push(UserPart::Text(text));
            }
        }
    }
}

fn assistant_content_to_openai(content: &Value) -> (String, Vec<Value>) {
    if !content.is_array() {
        return (content_to_text(content), Vec::new());
    }

    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    for block in content.as_array().expect("checked array") {
        match block.get("type").and_then(Value::as_str) {
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_call")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let arguments = block
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| json!({}))
                    .to_string();
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }));
            }
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    text_parts.push(text.to_string());
                }
            }
            // Claude Code echoes back the thinking blocks this proxy emitted for
            // reasoning upstreams; OpenAI-format APIs must not receive prior
            // reasoning content, so drop them instead of serializing raw JSON.
            Some("thinking") | Some("redacted_thinking") => {}
            _ => {
                let text = content_to_text(block);
                if !text.trim().is_empty() {
                    text_parts.push(text);
                }
            }
        }
    }
    (text_parts.join("\n"), tool_calls)
}

fn openai_tool_from_anthropic_tool(tool: &Value) -> Option<Value> {
    // Anthropic server tools (web_search_*, code_execution_*, ...) carry a
    // versioned `type` and no input_schema. They cannot run on an OpenAI
    // upstream, so drop them instead of fabricating a callable function the
    // client cannot execute.
    if let Some(tool_type) = tool.get("type").and_then(Value::as_str)
        && tool_type != "custom"
    {
        return None;
    }
    let name = tool.get("name").and_then(Value::as_str)?;
    Some(json!({
        "type": "function",
        "function": {
            "name": name,
            "description": tool.get("description").and_then(Value::as_str).unwrap_or(""),
            "parameters": tool.get("input_schema").cloned().unwrap_or_else(|| json!({ "type": "object" }))
        }
    }))
}

fn openai_tool_choice(tool_choice: &Value) -> Option<Value> {
    let choice_type = tool_choice.get("type").and_then(Value::as_str)?;
    match choice_type {
        "auto" => Some(Value::String("auto".to_string())),
        "any" => Some(Value::String("required".to_string())),
        "none" => Some(Value::String("none".to_string())),
        "tool" => tool_choice.get("name").and_then(Value::as_str).map(|name| {
            json!({
                "type": "function",
                "function": { "name": name }
            })
        }),
        _ => None,
    }
}

fn content_to_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                Some("tool_result") => Some(content_to_text(
                    block.get("content").unwrap_or(&Value::Null),
                )),
                Some("image") => Some("[image omitted by claudie OpenAI proxy]".to_string()),
                _ => block.as_str().map(str::to_string),
            })
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Rough input-token estimate for a native Anthropic request, used by the
/// `/v1/messages/count_tokens` endpoint and the streaming `message_start` event.
/// Converts to the OpenAI shape so only text that actually reaches the prompt is
/// counted (no JSON envelope, field names, or base64 image bytes), then applies
/// the same ~4-chars-per-token heuristic the optimizer uses.
pub(super) fn estimate_request_input_tokens(request: &Value, profile: &LlmProfile) -> u64 {
    let Ok(openai) = anthropic_to_openai_request(request, profile) else {
        return 1;
    };
    let mut chars = 0usize;
    if let Some(messages) = openai.get("messages").and_then(Value::as_array) {
        for message in messages {
            chars += openai_message_chars(message);
        }
    }
    if let Some(tools) = openai.get("tools") {
        chars += tools.to_string().chars().count();
    }
    (chars / 4).max(1) as u64
}

fn openai_message_chars(message: &Value) -> usize {
    let mut chars = message
        .get("content")
        .map(openai_content_chars)
        .unwrap_or(0);
    if let Some(tool_calls) = message.get("tool_calls") {
        chars += tool_calls.to_string().chars().count();
    }
    chars
}

fn openai_content_chars(content: &Value) -> usize {
    match content {
        Value::String(text) => text.chars().count(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .map(|text| text.chars().count())
                    .unwrap_or(0)
            })
            .sum(),
        _ => 0,
    }
}

fn copy_number(input: &Value, output: &mut Map<String, Value>, key: &str) {
    if let Some(value) = input.get(key)
        && value.is_number()
    {
        output.insert(key.to_string(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_basic_text_request() {
        let profile = LlmProfile {
            model: "gpt-test".to_string(),
            openai_extra_body: r#"{"reasoning_effort":"xhigh"}"#.to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "system": "be brief",
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hello" }] }],
            "max_tokens": 128,
            "stream": true
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["model"], "gpt-test");
        assert_eq!(converted["stream"], false);
        assert_eq!(converted["messages"][0]["role"], "system");
        assert!(
            converted["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("be brief")
        );
        assert!(
            converted["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Tool-result messages")
        );
        assert_eq!(converted["messages"][1]["content"], "hello");
        assert_eq!(converted["reasoning_effort"], "xhigh");
    }

    #[test]
    fn openai_proxy_compat_prompt_can_be_disabled() {
        let profile = LlmProfile {
            model: "gpt-test".to_string(),
            extra_env: "CLAUDIE_PROXY_COMPAT_PROMPT=0".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "system": "be brief",
            "messages": [{ "role": "user", "content": "hello" }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["messages"][0]["content"], "be brief");
        assert_eq!(converted["messages"][1]["content"], "hello");
    }

    #[test]
    fn request_model_overrides_profile_model() {
        let profile = LlmProfile {
            model: "profile-model".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "model": "session-model",
            "messages": [{ "role": "user", "content": "hello" }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["model"], "session-model");
    }

    #[test]
    fn enables_parallel_tool_calls_by_default_when_tools_are_present() {
        let profile = LlmProfile {
            model: "gpt-test".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "edit README" }],
            "tools": [{
                "name": "Update",
                "description": "Edit a file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "old_string": { "type": "string" },
                        "new_string": { "type": "string" }
                    },
                    "required": ["file_path", "old_string", "new_string"]
                }
            }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["parallel_tool_calls"], true);
    }

    #[test]
    fn openai_extra_body_can_override_parallel_tool_call_default() {
        let profile = LlmProfile {
            model: "gpt-test".to_string(),
            openai_extra_body: r#"{"parallel_tool_calls":false}"#.to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "inspect files" }],
            "tools": [{ "name": "Read", "input_schema": { "type": "object" } }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["parallel_tool_calls"], false);
    }

    #[test]
    fn compat_prompt_off_by_default_for_official_openai_host() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "system": "be brief",
            "messages": [{ "role": "user", "content": "hi" }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["messages"][0]["content"], "be brief");
        assert!(
            !converted["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Tool-result")
        );
    }

    #[test]
    fn compat_prompt_can_be_forced_on_official_openai_host() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            extra_env: "CLAUDIE_PROXY_COMPAT_PROMPT=1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "system": "be brief",
            "messages": [{ "role": "user", "content": "hi" }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert!(
            converted["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Tool-result messages")
        );
    }

    #[test]
    fn forwards_base64_image_alongside_text_for_vision_model() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "what is this?" },
                    { "type": "image", "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": "AAAA"
                    }}
                ]
            }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let user = &converted["messages"][0];
        assert_eq!(user["role"], "user");
        let content = user["content"].as_array().expect("content should be array");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "what is this?");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn forwards_url_image_for_vision_model() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image", "source": {
                        "type": "url",
                        "url": "https://example.com/cat.png"
                    }}
                ]
            }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let content = converted["messages"][0]["content"]
            .as_array()
            .expect("content should be array");
        assert_eq!(
            content[0]["image_url"]["url"],
            "https://example.com/cat.png"
        );
    }

    #[test]
    fn text_only_user_message_keeps_string_content_when_vision_supported() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "hi" }]
            }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["messages"][0]["content"], "hi");
    }

    #[test]
    fn drops_image_with_placeholder_for_non_vision_model() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-3.5-turbo".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "what is this?" },
                    { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" }}
                ]
            }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let content = converted["messages"][0]["content"].as_str().unwrap();
        assert!(content.contains("what is this?"));
        assert!(content.contains("does not support vision"));
    }

    #[test]
    fn forward_images_never_overrides_vision_model() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            extra_env: "CLAUDIE_PROXY_FORWARD_IMAGES=never".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" }}
                ]
            }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let content = converted["messages"][0]["content"].as_str().unwrap();
        assert!(content.contains("does not support vision"));
    }

    #[test]
    fn tool_result_with_image_emits_followup_user_message() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "call_1",
                    "content": [
                        { "type": "text", "text": "screenshot follows" },
                        { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "BBBB" }}
                    ]
                }]
            }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let messages = converted["messages"].as_array().unwrap();
        // No system because gpt-4o + official host = no compat prompt and no source system.
        let tool_msg = &messages[0];
        assert_eq!(tool_msg["role"], "tool");
        assert_eq!(tool_msg["tool_call_id"], "call_1");
        let tool_content = tool_msg["content"].as_str().unwrap();
        assert!(tool_content.contains("screenshot follows"));
        assert!(tool_content.contains("see attached image(s) in next message"));

        let followup = &messages[1];
        assert_eq!(followup["role"], "user");
        let parts = followup["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert!(
            parts[0]["text"]
                .as_str()
                .unwrap()
                .contains("Images attached for tool_result call_1")
        );
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,BBBB");
    }

    #[test]
    fn mixed_tool_results_are_grouped_before_user_text() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [
                {
                    "role": "assistant",
                    "content": [
                        { "type": "tool_use", "id": "call_a", "name": "Bash", "input": { "command": "git status" } },
                        { "type": "tool_use", "id": "call_b", "name": "Bash", "input": { "command": "cargo check" } }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "system reminder before results" },
                        { "type": "tool_result", "tool_use_id": "call_a", "content": "status output" },
                        { "type": "text", "text": "system reminder between results" },
                        { "type": "tool_result", "tool_use_id": "call_b", "content": "check output" },
                        { "type": "text", "text": "system reminder after results" }
                    ]
                }
            ]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let messages = converted["messages"].as_array().unwrap();
        let roles = messages
            .iter()
            .map(|message| message["role"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(roles, vec!["assistant", "tool", "tool", "user"]);
        assert_eq!(messages[1]["tool_call_id"], "call_a");
        assert_eq!(messages[2]["tool_call_id"], "call_b");
        let deferred = messages[3]["content"].as_str().unwrap();
        assert!(deferred.contains("before results"));
        assert!(deferred.contains("between results"));
        assert!(deferred.contains("after results"));
    }

    #[test]
    fn tool_result_with_image_drops_followup_for_non_vision_model() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-3.5-turbo".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "call_1",
                    "content": [
                        { "type": "text", "text": "ok" },
                        { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "BBBB" }}
                    ]
                }]
            }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let messages = converted["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "tool");
        let content = messages[0]["content"].as_str().unwrap();
        assert!(content.contains("does not support vision"));
    }

    #[test]
    fn reasoning_effort_auto_inserted_for_o3_models() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "o3-mini".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "solve" }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["reasoning_effort"], "medium");
    }

    #[test]
    fn reasoning_effort_uses_anthropic_thinking_budget() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "o3-mini".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "solve" }],
            "thinking": { "type": "enabled", "budget_tokens": 64000 }
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["reasoning_effort"], "high");
    }

    #[test]
    fn reasoning_effort_extra_body_overrides_auto() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "o3-mini".to_string(),
            openai_extra_body: r#"{"reasoning_effort":"low"}"#.to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "solve" }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["reasoning_effort"], "low");
    }

    #[test]
    fn reasoning_effort_not_set_for_non_reasoning_models() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }]
        });

        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert!(converted.get("reasoning_effort").is_none());
    }

    #[test]
    fn compat_prompt_off_by_default_for_deepseek() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "system": "you are concise",
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["messages"][0]["content"], "you are concise");
    }

    #[test]
    fn compat_prompt_off_by_default_for_qwen_kimi_glm_openrouter() {
        for url in [
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "https://api.moonshot.cn/v1",
            "https://open.bigmodel.cn/api/paas/v4",
            "https://openrouter.ai/api/v1",
        ] {
            let profile = LlmProfile {
                base_url: url.to_string(),
                model: "any".to_string(),
                ..LlmProfile::default()
            };
            let request = json!({
                "system": "x",
                "messages": [{ "role": "user", "content": "y" }]
            });
            let converted = anthropic_to_openai_request(&request, &profile).unwrap();
            let sys = converted["messages"][0]["content"].as_str().unwrap();
            assert!(
                !sys.contains("Tool-result messages"),
                "compat prompt should be off for {url}: {sys}"
            );
        }
    }

    #[test]
    fn compat_prompt_on_by_default_for_generic_provider() {
        let profile = LlmProfile {
            base_url: "http://my-oneapi.local:3000/v1".to_string(),
            model: "anything".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "system": "be brief",
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let sys = converted["messages"][0]["content"].as_str().unwrap();
        assert!(sys.contains("Tool-result messages"));
    }

    #[test]
    fn compat_prompt_force_on_via_env_overrides_provider_default() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            extra_env: "CLAUDIE_PROXY_COMPAT_PROMPT=1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "system": "x",
            "messages": [{ "role": "user", "content": "y" }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert!(
            converted["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Tool-result messages")
        );
    }

    #[test]
    fn reasoning_effort_not_emitted_for_deepseek_r1() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-r1".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "solve" }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert!(converted.get("reasoning_effort").is_none());
    }

    #[test]
    fn reasoning_effort_not_emitted_for_qwen_qwq() {
        let profile = LlmProfile {
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            model: "qwq-32b-preview".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "solve" }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert!(converted.get("reasoning_effort").is_none());
    }

    #[test]
    fn reasoning_effort_emitted_for_openrouter_routed_o3() {
        let profile = LlmProfile {
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model: "openai/o3-mini".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "solve" }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["reasoning_effort"], "medium");
    }

    #[test]
    fn tools_stripped_for_deepseek_reasoner() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-reasoner".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "Read", "input_schema": { "type": "object" } }],
            "tool_choice": { "type": "auto" }
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert!(converted.get("tools").is_none());
        assert!(converted.get("tool_choice").is_none());
        assert!(converted.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn tools_kept_for_deepseek_chat() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "Read", "input_schema": { "type": "object" } }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert!(converted.get("tools").is_some());
        assert_eq!(converted["parallel_tool_calls"], true);
    }

    #[test]
    fn extra_body_tools_fields_dropped_when_model_does_not_support_tools() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-reasoner".to_string(),
            openai_extra_body: r#"{"parallel_tool_calls":true}"#.to_string(),
            ..LlmProfile::default()
        };
        let request = json!({ "messages": [{ "role": "user", "content": "hi" }] });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert!(converted.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn assistant_thinking_blocks_are_dropped() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "secret chain of thought" },
                    { "type": "redacted_thinking", "data": "opaque" },
                    { "type": "text", "text": "the answer is 4" },
                    { "type": "tool_use", "id": "call_1", "name": "Read", "input": { "file": "a.rs" } }
                ]
            }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let assistant = &converted["messages"][0];
        assert_eq!(assistant["content"], "the answer is 4");
        let serialized = assistant.to_string();
        assert!(!serialized.contains("thinking"));
        assert!(!serialized.contains("secret chain of thought"));
        assert_eq!(assistant["tool_calls"][0]["function"]["name"], "Read");
    }

    #[test]
    fn server_tool_definitions_are_filtered_out() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [
                { "type": "web_search_20260209", "name": "web_search" },
                { "name": "Read", "input_schema": { "type": "object" } },
                { "type": "custom", "name": "Write", "input_schema": { "type": "object" } }
            ]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let tools = converted["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["function"]["name"], "Read");
        assert_eq!(tools[1]["function"]["name"], "Write");
    }

    #[test]
    fn tool_choice_none_is_forwarded() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "Read", "input_schema": { "type": "object" } }],
            "tool_choice": { "type": "none" }
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["tool_choice"], "none");
    }

    #[test]
    fn disable_parallel_tool_use_disables_parallel_tool_calls() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "Read", "input_schema": { "type": "object" } }],
            "tool_choice": { "type": "auto", "disable_parallel_tool_use": true }
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["parallel_tool_calls"], false);
        assert_eq!(converted["tool_choice"], "auto");
    }

    #[test]
    fn o_series_on_openai_uses_max_completion_tokens() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "o3-mini".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 4096
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["max_completion_tokens"], 4096);
        assert!(converted.get("max_tokens").is_none());
    }

    #[test]
    fn non_reasoning_openai_model_keeps_max_tokens() {
        let profile = LlmProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 4096
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["max_tokens"], 4096);
        assert!(converted.get("max_completion_tokens").is_none());
    }

    #[test]
    fn deepseek_keeps_max_tokens_even_for_reasoning_model() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-reasoner".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 4096
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        assert_eq!(converted["max_tokens"], 4096);
        assert!(converted.get("max_completion_tokens").is_none());
    }

    #[test]
    fn tool_result_is_error_adds_error_prefix() {
        let profile = LlmProfile {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            ..LlmProfile::default()
        };
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "tool_result", "tool_use_id": "call_1", "content": "file not found", "is_error": true },
                    { "type": "tool_result", "tool_use_id": "call_2", "content": "ok" },
                    { "type": "tool_result", "tool_use_id": "call_3", "is_error": true }
                ]
            }]
        });
        let converted = anthropic_to_openai_request(&request, &profile).unwrap();
        let messages = converted["messages"].as_array().unwrap();
        assert_eq!(messages[0]["content"], "[tool_error] file not found");
        assert_eq!(messages[1]["content"], "ok");
        assert_eq!(messages[2]["content"], "[tool_error] tool execution failed");
    }
}
