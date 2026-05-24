use serde_json::{Map, Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::app::{AppState, PetMood};
use crate::config::DEFAULT_PROXY_PORT;
use crate::proxy_optimizer;
use crate::settings::LlmProfile;

const MAX_PROXY_REQUEST_BYTES: usize = 10 * 1024 * 1024;

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
                            proxy_optimizer::save_summary(&pending.cache_key, &summary)
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

    let upstream = match call_openai(&agent, &profile, &openai_request) {
        Ok(value) => value,
        Err(err) => {
            record_proxy_error(&state, err.clone());
            let _ = write_json_response(&mut stream, 502, json!({ "error": err }));
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

fn call_openai(agent: &ureq::Agent, profile: &LlmProfile, body: &Value) -> Result<Value, String> {
    let auth = format!("Bearer {}", profile.openai_upstream_api_key());
    let response = agent
        .post(&profile.openai_chat_completions_url())
        .set("Authorization", &auth)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());

    let text = match response {
        Ok(response) => response
            .into_string()
            .map_err(|err| format!("OpenAI proxy upstream response failed: {err}"))?,
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(format!(
                "OpenAI proxy upstream returned HTTP {status}: {}",
                shorten_for_error(&text)
            ));
        }
        Err(err) => return Err(format!("OpenAI proxy upstream request failed: {err}")),
    };

    serde_json::from_str(&text)
        .map_err(|err| format!("OpenAI proxy upstream returned invalid JSON: {err}"))
}

fn anthropic_to_openai_request(request: &Value, profile: &LlmProfile) -> Result<Value, String> {
    let mut messages = Vec::new();
    if let Some(system) = request.get("system") {
        let text = content_to_text(system);
        if !text.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": text }));
        }
    }

    let Some(input_messages) = request.get("messages").and_then(Value::as_array) else {
        return Err("messages must be an array".to_string());
    };
    for message in input_messages {
        append_openai_messages(&mut messages, message);
    }

    let mut out = Map::new();
    let model = profile
        .model
        .trim()
        .is_empty()
        .then(|| {
            request
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
        })
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
            out.insert(
                "content".to_string(),
                if text.trim().is_empty() {
                    Value::Null
                } else {
                    Value::String(text)
                },
            );
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

    if let Some(text) = message.get("content").and_then(Value::as_str)
        && !text.is_empty()
    {
        content.push(json!({ "type": "text", "text": text }));
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
        "model": profile.model.trim().is_empty().then(|| request.get("model").and_then(Value::as_str).unwrap_or("claudie-openai-proxy")).unwrap_or_else(|| profile.model.trim()),
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
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let input =
        serde_json::from_str(arguments).unwrap_or_else(|_| json!({ "arguments": arguments }));
    Some(json!({
        "type": "tool_use",
        "id": tool_call.get("id").and_then(Value::as_str).unwrap_or("tool_call"),
        "name": name,
        "input": input
    }))
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
        assert_eq!(converted["messages"][1]["content"], "hello");
        assert_eq!(converted["reasoning_effort"], "xhigh");
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
}
