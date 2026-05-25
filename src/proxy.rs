use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::app::{AppState, PetMood};
use crate::config::DEFAULT_PROXY_PORT;
use crate::proxy_optimizer;
use crate::settings::LlmProfile;
use crate::settings::storage::{read_json_or_default, save_pretty_json};

const MAX_PROXY_REQUEST_BYTES: usize = 10 * 1024 * 1024;
const CAPABILITY_CACHE_VERSION: &str = "v1";
const DEFAULT_CAPABILITY_CACHE_TTL_HOURS: u64 = 720;
const DEFAULT_CAPABILITY_CACHE_MAX_ENTRIES: usize = 200;
const DEFAULT_PROXY_CACHE_MAX_BYTES: u64 = 10 * 1024 * 1024;
const OPENAI_PROXY_COMPAT_PROMPT: &str = "\
Tool-result messages are observations from your prior tool calls; continue \
the task across multiple tool calls without re-asking the user. Prefer parallel \
tool calls when actions are independent (e.g. reading several files, staging \
multiple paths in one git command). If an edit tool fails, re-read the relevant \
file section before retrying.";

pub(crate) fn start_openai_proxy_server(state: Arc<Mutex<AppState>>) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", DEFAULT_PROXY_PORT))
        .map_err(|err| format!("OpenAI proxy failed: {err}"))?;

    thread::spawn(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(600))
            .build();

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let state = state.clone();
                    let agent = agent.clone();
                    thread::spawn(move || handle_proxy_client(stream, state, agent));
                }
                Err(err) => {
                    record_proxy_error(&state, format!("OpenAI proxy accept failed: {err}"))
                }
            }
        }
    });

    Ok(())
}

fn handle_proxy_client(mut stream: TcpStream, state: Arc<Mutex<AppState>>, agent: ureq::Agent) {
    let request = match read_http_request(&mut stream) {
        Ok(request) => request,
        Err(err) => {
            let _ = write_json_response(&mut stream, 400, json!({ "error": err }));
            return;
        }
    };

    let path = request.path.split('?').next().unwrap_or(&request.path);
    if request.method == "GET" && path.ends_with("/models") {
        let profile = active_openai_profile(&state);
        let model = profile
            .as_ref()
            .map(|profile| profile.model.trim())
            .filter(|model| !model.is_empty())
            .unwrap_or("claudie-openai-proxy");
        let _ = write_json_response(
            &mut stream,
            200,
            json!({
                "object": "list",
                "data": [{ "id": model, "object": "model", "owned_by": "claudie" }]
            }),
        );
        return;
    }

    if request.method != "POST" {
        let _ = write_json_response(
            &mut stream,
            405,
            json!({ "error": "claudie proxy accepts POST /v1/messages" }),
        );
        return;
    }

    if path.ends_with("/messages/count_tokens") {
        handle_count_tokens(&mut stream, &request.body);
        return;
    }

    if !path.ends_with("/messages") {
        let _ = write_json_response(
            &mut stream,
            404,
            json!({ "error": "claudie proxy only implements /v1/messages" }),
        );
        return;
    }

    let Some(profile) = active_openai_profile(&state) else {
        let _ = write_json_response(
            &mut stream,
            503,
            json!({ "error": "No active OpenAI chat/completions profile is configured in claudie." }),
        );
        return;
    };

    if profile.openai_upstream_api_key().is_empty() {
        let _ = write_json_response(
            &mut stream,
            400,
            json!({ "error": "The active OpenAI proxy profile is missing an API key or auth token." }),
        );
        return;
    }

    let anthropic_request: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(err) => {
            let _ = write_json_response(&mut stream, 400, json!({ "error": err.to_string() }));
            return;
        }
    };

    let wants_stream = anthropic_request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let openai_request = match anthropic_to_openai_request(&anthropic_request, &profile) {
        Ok(value) => value,
        Err(err) => {
            let _ = write_json_response(&mut stream, 400, json!({ "error": err }));
            return;
        }
    };
    let optimized = proxy_optimizer::optimize_openai_request(openai_request, &profile);
    if optimized.cache_hit {
        record_proxy_event(&state, "summary cache hit".to_string());
    } else if optimized.local_summary {
        record_proxy_event(&state, "local context summary applied".to_string());
    } else if optimized.compressed {
        record_proxy_event(&state, "compressed OpenAI proxy context".to_string());
    }
    let openai_request = if let Some(pending) = optimized.pending_summary {
        match call_openai(&agent, &profile, &pending.summary_request) {
            Ok(summary_response) => {
                match proxy_optimizer::summary_text_from_openai_response(&summary_response) {
                    Some(summary) => {
                        if let Err(err) =
                            proxy_optimizer::save_summary(&pending.cache_key, &summary, &profile)
                        {
                            record_proxy_event(&state, format!("summary cache save failed: {err}"));
                        } else {
                            record_proxy_event(&state, "summary cache saved".to_string());
                        }
                        pending.request_with_summary(&summary)
                    }
                    None => {
                        record_proxy_event(
                            &state,
                            "summary response had no text; using compressed fallback".to_string(),
                        );
                        pending.fallback_request
                    }
                }
            }
            Err(err) => {
                record_proxy_event(
                    &state,
                    format!("summary request failed; using compressed fallback: {err}"),
                );
                pending.fallback_request
            }
        }
    } else {
        optimized.request
    };

    let use_tool_transcript = cached_tool_history_needs_transcript(&profile, &openai_request);
    let outbound_request = if use_tool_transcript {
        record_proxy_event(
            &state,
            "using cached OpenAI tool transcript compatibility mode".to_string(),
        );
        tool_history_as_text_transcript(&openai_request)
    } else {
        openai_request.clone()
    };

    let upstream = match call_openai(&agent, &profile, &outbound_request) {
        Ok(value) => value,
        Err(err)
            if !use_tool_transcript && should_retry_with_tool_transcript(&err, &openai_request) =>
        {
            record_proxy_event(
                &state,
                "retrying OpenAI request with text tool transcript".to_string(),
            );
            if let Err(cache_err) = save_tool_history_capability(&profile, &openai_request, false) {
                record_proxy_event(&state, format!("capability cache save failed: {cache_err}"));
            }
            let fallback_request = tool_history_as_text_transcript(&openai_request);
            match call_openai(&agent, &profile, &fallback_request) {
                Ok(value) => value,
                Err(retry_err) => {
                    let combined = format!("{err}; text tool transcript retry failed: {retry_err}");
                    record_proxy_error(&state, combined.clone());
                    let _ = write_json_response(
                        &mut stream,
                        proxy_status_for_upstream_error(&retry_err),
                        json!({ "error": combined }),
                    );
                    return;
                }
            }
        }
        Err(err) => {
            let message = err.to_string();
            record_proxy_error(&state, message.clone());
            let _ = write_json_response(
                &mut stream,
                proxy_status_for_upstream_error(&err),
                json!({ "error": message }),
            );
            return;
        }
    };

    let anthropic_response = openai_to_anthropic_response(&upstream, &anthropic_request, &profile);
    if wants_stream {
        let _ = write_anthropic_sse_response(&mut stream, &anthropic_response);
    } else {
        let _ = write_json_response(&mut stream, 200, anthropic_response);
    }
}

fn active_openai_profile(state: &Arc<Mutex<AppState>>) -> Option<LlmProfile> {
    let state = state.lock().expect("state poisoned");
    state
        .llm_profiles
        .active_profile()
        .filter(|profile| profile.is_openai_chat_proxy())
        .cloned()
}

#[derive(Clone, Debug)]
struct UpstreamError {
    status: Option<u16>,
    message: String,
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn call_openai(
    agent: &ureq::Agent,
    profile: &LlmProfile,
    body: &Value,
) -> Result<Value, UpstreamError> {
    let auth = format!("Bearer {}", profile.openai_upstream_api_key());
    let response = agent
        .post(&profile.openai_chat_completions_url())
        .set("Authorization", &auth)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());

    let text = match response {
        Ok(response) => response.into_string().map_err(|err| UpstreamError {
            status: None,
            message: format!("OpenAI proxy upstream response failed: {err}"),
        })?,
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(UpstreamError {
                status: Some(status),
                message: format!(
                    "OpenAI proxy upstream returned HTTP {status}: {}",
                    shorten_for_error(&text)
                ),
            });
        }
        Err(err) => {
            return Err(UpstreamError {
                status: None,
                message: format!("OpenAI proxy upstream request failed: {err}"),
            });
        }
    };

    serde_json::from_str(&text).map_err(|err| UpstreamError {
        status: None,
        message: format!("OpenAI proxy upstream returned invalid JSON: {err}"),
    })
}

fn proxy_status_for_upstream_error(err: &UpstreamError) -> u16 {
    if upstream_error_is_temporarily_unavailable(err) {
        return 503;
    }
    match err.status {
        Some(400..=499) => err.status.unwrap_or(502),
        Some(500..=599) | None => 502,
        Some(_) => 502,
    }
}

fn upstream_error_is_temporarily_unavailable(err: &UpstreamError) -> bool {
    let message = err.message.to_ascii_lowercase();
    let temporary_wording = message.contains("temporarily")
        || message.contains("temporary")
        || message.contains("try again later")
        || message.contains("engine is not available");
    let known_transient_code = message.contains("failed_precondition_error")
        && (message.contains("\"code\":\"9\"") || message.contains("\"code\": \"9\""));
    matches!(err.status, Some(400..=499)) && (temporary_wording || known_transient_code)
}

fn anthropic_to_openai_request(request: &Value, profile: &LlmProfile) -> Result<Value, String> {
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
        append_openai_messages(&mut messages, message);
    }

    let mut out = Map::new();
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| profile.model.trim());
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert("messages".to_string(), Value::Array(messages));
    out.insert("stream".to_string(), Value::Bool(false));

    copy_number(request, &mut out, "max_tokens");
    copy_number(request, &mut out, "temperature");
    copy_number(request, &mut out, "top_p");
    if let Some(stop) = request.get("stop_sequences") {
        out.insert("stop".to_string(), stop.clone());
    }

    if let Some(tools) = request.get("tools").and_then(Value::as_array) {
        let converted = tools
            .iter()
            .filter_map(openai_tool_from_anthropic_tool)
            .collect::<Vec<_>>();
        if !converted.is_empty() {
            out.insert("tools".to_string(), Value::Array(converted));
            out.insert("parallel_tool_calls".to_string(), Value::Bool(true));
        }
    }

    if let Some(tool_choice) = request.get("tool_choice") {
        if let Some(converted) = openai_tool_choice(tool_choice) {
            out.insert("tool_choice".to_string(), converted);
        }
    }

    for (key, value) in profile.openai_extra_body_fields()? {
        out.insert(key, value);
    }

    Ok(Value::Object(out))
}

fn openai_proxy_compat_prompt_enabled(profile: &LlmProfile) -> bool {
    profile
        .extra_env_value("CLAUDIE_PROXY_COMPAT_PROMPT")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

fn append_openai_messages(messages: &mut Vec<Value>, message: &Value) {
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
        _ => append_user_content(messages, content),
    }
}

fn append_user_content(messages: &mut Vec<Value>, content: &Value) {
    if !content.is_array() {
        let text = content_to_text(content);
        if !text.trim().is_empty() {
            messages.push(json!({ "role": "user", "content": text }));
        }
        return;
    }

    let mut text_parts = Vec::new();
    for block in content.as_array().expect("checked array") {
        match block.get("type").and_then(Value::as_str) {
            Some("tool_result") => {
                if !text_parts.is_empty() {
                    messages.push(json!({ "role": "user", "content": text_parts.join("\n") }));
                    text_parts.clear();
                }
                let tool_call_id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_call");
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content_to_text(block.get("content").unwrap_or(&Value::Null))
                }));
            }
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    text_parts.push(text.to_string());
                }
            }
            Some("image") => text_parts.push("[image omitted by claudie OpenAI proxy]".to_string()),
            _ => {
                let text = content_to_text(block);
                if !text.trim().is_empty() {
                    text_parts.push(text);
                }
            }
        }
    }
    if !text_parts.is_empty() {
        messages.push(json!({ "role": "user", "content": text_parts.join("\n") }));
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
        "tool" => tool_choice.get("name").and_then(Value::as_str).map(|name| {
            json!({
                "type": "function",
                "function": { "name": name }
            })
        }),
        _ => None,
    }
}

fn openai_to_anthropic_response(openai: &Value, request: &Value, profile: &LlmProfile) -> Value {
    let choice = openai
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
    let mut content = Vec::new();

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

    json!({
        "id": openai.get("id").and_then(Value::as_str).unwrap_or("msg_claudie_proxy"),
        "type": "message",
        "role": "assistant",
        "model": response_model(openai, request, profile),
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": openai.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            "output_tokens": openai.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0)
        }
    })
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

fn openai_function_arguments_to_value(arguments: Option<&Value>) -> Value {
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

fn should_retry_with_tool_transcript(err: &UpstreamError, request: &Value) -> bool {
    if !request_has_tool_history(request) {
        return false;
    }
    let err = err.message.to_ascii_lowercase();
    let status_matches = err.contains("http 400") || err.contains("http 422");
    let shape_matches = ["tool", "tool_call", "messages", "role"]
        .iter()
        .any(|needle| err.contains(needle));
    status_matches && shape_matches
}

fn request_has_tool_history(request: &Value) -> bool {
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

fn cached_tool_history_needs_transcript(profile: &LlmProfile, request: &Value) -> bool {
    if !request_has_tool_history(request) {
        return false;
    }
    let path = capability_cache_file_path(profile, request);
    cached_tool_history_needs_transcript_at(profile, &path)
}

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

fn save_tool_history_capability(
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

fn tool_history_as_text_transcript(request: &Value) -> Value {
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

fn openai_message_content_to_text(content: &Value) -> String {
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

fn copy_number(input: &Value, output: &mut Map<String, Value>, key: &str) {
    if let Some(value) = input.get(key)
        && value.is_number()
    {
        output.insert(key.to_string(), value.clone());
    }
}

fn handle_count_tokens(stream: &mut TcpStream, body: &[u8]) {
    let input: Value = serde_json::from_slice(body).unwrap_or_else(|_| json!({}));
    let text = input.to_string();
    let estimate = (text.chars().count() / 4).max(1);
    let _ = write_json_response(stream, 200, json!({ "input_tokens": estimate }));
}

fn write_anthropic_sse_response(stream: &mut TcpStream, message: &Value) -> std::io::Result<()> {
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    stream.write_all(header.as_bytes())?;
    write_sse_event(
        stream,
        "message_start",
        json!({
            "type": "message_start",
            "message": {
                "id": message.get("id").cloned().unwrap_or_else(|| json!("msg_claudie_proxy")),
                "type": "message",
                "role": "assistant",
                "model": message.get("model").cloned().unwrap_or_else(|| json!("claudie-openai-proxy")),
                "content": [],
                "stop_reason": Value::Null,
                "stop_sequence": Value::Null,
                "usage": { "input_tokens": message.pointer("/usage/input_tokens").and_then(Value::as_u64).unwrap_or(0), "output_tokens": 0 }
            }
        }),
    )?;

    for (index, block) in message
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        write_sse_event(
            stream,
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": sse_content_block_start(block)
            }),
        )?;
        match block.get("type").and_then(Value::as_str) {
            Some("tool_use") => {
                write_sse_event(
                    stream,
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": block.get("input").cloned().unwrap_or_else(|| json!({})).to_string()
                        }
                    }),
                )?;
            }
            _ => {
                write_sse_event(
                    stream,
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "text_delta",
                            "text": block.get("text").and_then(Value::as_str).unwrap_or("")
                        }
                    }),
                )?;
            }
        }
        write_sse_event(
            stream,
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": index }),
        )?;
    }

    write_sse_event(
        stream,
        "message_delta",
        json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": message.get("stop_reason").and_then(Value::as_str).unwrap_or("end_turn"),
                "stop_sequence": Value::Null
            },
            "usage": { "output_tokens": message.pointer("/usage/output_tokens").and_then(Value::as_u64).unwrap_or(0) }
        }),
    )?;
    write_sse_event(stream, "message_stop", json!({ "type": "message_stop" }))?;
    stream.flush()
}

fn sse_content_block_start(block: &Value) -> Value {
    match block.get("type").and_then(Value::as_str) {
        Some("tool_use") => json!({
            "type": "tool_use",
            "id": block.get("id").and_then(Value::as_str).unwrap_or("tool_call"),
            "name": block.get("name").and_then(Value::as_str).unwrap_or("tool"),
            "input": {}
        }),
        _ => json!({ "type": "text", "text": "" }),
    }
}

fn write_sse_event(stream: &mut TcpStream, event: &str, data: Value) -> std::io::Result<()> {
    stream.write_all(format!("event: {event}\n").as_bytes())?;
    stream.write_all(format!("data: {data}\n\n").as_bytes())
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|err| err.to_string())?;

    let mut buffer = Vec::with_capacity(8192);
    let mut temp = [0_u8; 4096];
    let header_end;

    loop {
        let count = stream.read(&mut temp).map_err(|err| err.to_string())?;
        if count == 0 {
            return Err("connection closed".to_string());
        }
        buffer.extend_from_slice(&temp[..count]);
        if let Some(pos) = find_header_end(&buffer) {
            header_end = pos;
            break;
        }
        if buffer.len() > 64 * 1024 {
            return Err("request header too large".to_string());
        }
    }

    let header = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = header.lines();
    let request_line = lines.next().ok_or_else(|| "empty request".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut content_length = 0_usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value
                .trim()
                .parse::<usize>()
                .map_err(|err| err.to_string())?;
        }
    }
    if content_length > MAX_PROXY_REQUEST_BYTES {
        return Err("request body too large".to_string());
    }

    let body_start = header_end + 4;
    let mut body = buffer[body_start..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let chunk_len = remaining.min(temp.len());
        let count = stream
            .read(&mut temp[..chunk_len])
            .map_err(|err| err.to_string())?;
        if count == 0 {
            return Err("connection closed before body completed".to_string());
        }
        body.extend_from_slice(&temp[..count]);
    }
    body.truncate(content_length);

    Ok(HttpRequest { method, path, body })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_json_response(stream: &mut TcpStream, status: u16, body: Value) -> std::io::Result<()> {
    write_response(stream, status, "application/json", body.to_string())
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: String,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())
}

fn shorten_for_error(text: &str) -> String {
    let mut shortened = text.trim().replace(['\r', '\n'], " ");
    if shortened.len() > 500 {
        shortened.truncate(500);
        shortened.push_str("...");
    }
    shortened
}

fn record_proxy_error(state: &Arc<Mutex<AppState>>, err: String) {
    let mut state = state.lock().expect("state poisoned");
    state.last_error = err.clone();
    state.set_mood(PetMood::Error);
    state.push_event("proxy", err);
}

fn record_proxy_event(state: &Arc<Mutex<AppState>>, event: String) {
    let mut state = state.lock().expect("state poisoned");
    state.push_event("proxy optimizer", event);
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
    fn upstream_4xx_status_is_preserved_for_client() {
        let err = UpstreamError {
            status: Some(400),
            message: "bad request".to_string(),
        };
        assert_eq!(proxy_status_for_upstream_error(&err), 400);

        let err = UpstreamError {
            status: Some(503),
            message: "unavailable".to_string(),
        };
        assert_eq!(proxy_status_for_upstream_error(&err), 502);
    }

    #[test]
    fn temporary_engine_4xx_maps_to_retryable_503() {
        let err = UpstreamError {
            status: Some(400),
            message: r#"OpenAI proxy upstream returned HTTP 400: {"error":{"message":"engine is not available temporarily","type":"failed_precondition_error","code":"9"}}"#
                .to_string(),
        };

        assert_eq!(proxy_status_for_upstream_error(&err), 503);
    }

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
