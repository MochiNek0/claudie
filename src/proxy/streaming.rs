use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::app::AppState;
use crate::settings::LlmProfile;

use super::capability_cache::save_tool_history_capability;
use super::http::{shorten_for_error, write_json_response};
use super::request_conv::estimate_request_input_tokens;
use super::response_conv::cached_input_tokens;
use super::tool_history::{should_retry_with_tool_transcript, tool_history_as_text_transcript};
use super::upstream::{call_openai_streaming, proxy_status_for_upstream_error};
use super::{record_proxy_error, record_proxy_event};

#[allow(clippy::too_many_arguments)]
pub(super) fn run_streaming_request(
    stream: &mut TcpStream,
    state: &Arc<Mutex<AppState>>,
    agent: &ureq::Agent,
    profile: &LlmProfile,
    anthropic_request: &Value,
    openai_request: &Value,
    outbound_request: &Value,
    use_tool_transcript: bool,
    known_native: bool,
) {
    match call_openai_streaming(agent, profile, outbound_request) {
        Ok(reader) => {
            let _ = stream_sse_to_client(stream, reader, state, profile, anthropic_request);
        }
        Err(err)
            if !use_tool_transcript
                && !known_native
                && should_retry_with_tool_transcript(&err, openai_request) =>
        {
            record_proxy_event(
                state,
                "retrying OpenAI streaming with text tool transcript".to_string(),
            );
            if let Err(cache_err) = save_tool_history_capability(profile, openai_request, false) {
                record_proxy_event(state, format!("capability cache save failed: {cache_err}"));
            }
            let fallback = tool_history_as_text_transcript(openai_request);
            match call_openai_streaming(agent, profile, &fallback) {
                Ok(reader) => {
                    let _ = stream_sse_to_client(stream, reader, state, profile, anthropic_request);
                }
                Err(retry_err) => {
                    let combined = format!("{err}; text tool transcript retry failed: {retry_err}");
                    record_proxy_error(state, combined.clone());
                    let _ = write_json_response(
                        stream,
                        proxy_status_for_upstream_error(&retry_err),
                        json!({ "error": combined }),
                    );
                }
            }
        }
        Err(err) => {
            let message = err.to_string();
            record_proxy_error(state, message.clone());
            let _ = write_json_response(
                stream,
                proxy_status_for_upstream_error(&err),
                json!({ "error": message }),
            );
        }
    }
}

fn stream_sse_to_client<R: BufRead>(
    stream: &mut TcpStream,
    reader: R,
    state: &Arc<Mutex<AppState>>,
    profile: &LlmProfile,
    anthropic_request: &Value,
) -> std::io::Result<()> {
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    stream.write_all(header.as_bytes())?;
    stream.flush()?;

    let fallback_model = anthropic_request
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| profile.model.trim().to_string());
    let estimated_input_tokens = estimate_request_input_tokens(anthropic_request, profile);

    let mut translator = StreamTranslator::new(stream, fallback_model, estimated_input_tokens);
    let result = translator.run(reader);
    // Surface mid-stream upstream errors to the pet/event log; the SSE `error`
    // event has already been written to the client inside run().
    if let Some(message) = translator.error_message.take() {
        record_proxy_error(state, message);
    }
    result
}

struct ToolBlock {
    block_index: usize,
}

struct StreamTranslator<'a, W: Write> {
    out: &'a mut W,
    fallback_model: String,
    estimated_input_tokens: u64,
    message_id: Option<String>,
    model: Option<String>,
    started: bool,
    thinking_block_index: Option<usize>,
    text_block_index: Option<usize>,
    next_block_index: usize,
    tool_block_by_openai_index: HashMap<u64, ToolBlock>,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    stop_reason: Option<&'static str>,
    error_message: Option<String>,
}

impl<'a, W: Write> StreamTranslator<'a, W> {
    fn new(out: &'a mut W, fallback_model: String, estimated_input_tokens: u64) -> Self {
        Self {
            out,
            fallback_model,
            estimated_input_tokens,
            message_id: None,
            model: None,
            started: false,
            thinking_block_index: None,
            text_block_index: None,
            next_block_index: 0,
            tool_block_by_openai_index: HashMap::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            stop_reason: None,
            error_message: None,
        }
    }

    fn run<R: BufRead>(&mut self, mut reader: R) -> std::io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(err) => {
                    if is_client_disconnect(&err) {
                        return Ok(());
                    }
                    break;
                }
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }
            let payload = match trimmed.strip_prefix("data:") {
                Some(rest) => rest.trim_start(),
                None => continue,
            };
            if payload == "[DONE]" {
                break;
            }
            let chunk: Value = match serde_json::from_str(payload) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(error) = chunk.get("error") {
                let _ = self.emit_stream_error(error);
                return Ok(());
            }
            if let Err(err) = self.absorb(&chunk) {
                if is_client_disconnect(&err) {
                    return Ok(());
                }
                break;
            }
        }
        self.finalize()
    }

    fn absorb(&mut self, chunk: &Value) -> std::io::Result<()> {
        if self.message_id.is_none() {
            if let Some(id) = chunk.get("id").and_then(Value::as_str) {
                self.message_id = Some(id.to_string());
            }
        }
        if self.model.is_none() {
            if let Some(model) = chunk.get("model").and_then(Value::as_str) {
                if !model.is_empty() {
                    self.model = Some(model.to_string());
                }
            }
        }
        if !self.started {
            self.emit_message_start()?;
        }

        let choice = chunk.pointer("/choices/0").cloned().unwrap_or(Value::Null);

        // Reasoning chain (DeepSeek-r1 / QwQ / GLM-Zero) arrives in delta.reasoning_content
        // before the final answer streams via delta.content. Emit it as an Anthropic
        // thinking block so Claude Code surfaces the collapsible thinking UI.
        if let Some(reasoning) = choice
            .pointer("/delta/reasoning_content")
            .and_then(Value::as_str)
        {
            if !reasoning.is_empty() {
                self.ensure_thinking_block()?;
                let index = self.thinking_block_index.expect("thinking block opened");
                self.write_event(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": { "type": "thinking_delta", "thinking": reasoning }
                    }),
                )?;
            }
        }

        if let Some(text) = choice.pointer("/delta/content").and_then(Value::as_str) {
            if !text.is_empty() {
                self.ensure_text_block()?;
                let index = self.text_block_index.expect("text block opened");
                self.write_event(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": { "type": "text_delta", "text": text }
                    }),
                )?;
            }
        }

        if let Some(tool_calls) = choice
            .pointer("/delta/tool_calls")
            .and_then(Value::as_array)
        {
            for tc in tool_calls {
                self.absorb_tool_call(tc)?;
            }
        }

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop_reason = Some(map_finish_reason(reason));
        }

        if let Some(usage) = chunk.get("usage") {
            if let Some(input) = usage.get("prompt_tokens").and_then(Value::as_u64) {
                self.input_tokens = input;
            }
            if let Some(output) = usage.get("completion_tokens").and_then(Value::as_u64) {
                self.output_tokens = output;
            }
            self.cache_read_tokens = cached_input_tokens(usage);
        }

        Ok(())
    }

    fn absorb_tool_call(&mut self, tc: &Value) -> std::io::Result<()> {
        let openai_index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
        let block_index = if let Some(existing) = self.tool_block_by_openai_index.get(&openai_index)
        {
            existing.block_index
        } else {
            let block_index = self.next_block_index;
            self.next_block_index += 1;
            let id = tc
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("tool_call")
                .to_string();
            let name = tc
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            self.tool_block_by_openai_index
                .insert(openai_index, ToolBlock { block_index });
            self.write_event(
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": block_index,
                    "content_block": {
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": {}
                    }
                }),
            )?;
            block_index
        };

        if let Some(arg_frag) = tc.pointer("/function/arguments").and_then(Value::as_str) {
            if !arg_frag.is_empty() {
                self.write_event(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": block_index,
                        "delta": { "type": "input_json_delta", "partial_json": arg_frag }
                    }),
                )?;
            }
        }
        Ok(())
    }

    fn ensure_text_block(&mut self) -> std::io::Result<()> {
        if self.text_block_index.is_some() {
            return Ok(());
        }
        let index = self.next_block_index;
        self.next_block_index += 1;
        self.text_block_index = Some(index);
        self.write_event(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": { "type": "text", "text": "" }
            }),
        )
    }

    fn ensure_thinking_block(&mut self) -> std::io::Result<()> {
        if self.thinking_block_index.is_some() {
            return Ok(());
        }
        let index = self.next_block_index;
        self.next_block_index += 1;
        self.thinking_block_index = Some(index);
        self.write_event(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": { "type": "thinking", "thinking": "" }
            }),
        )
    }

    fn emit_message_start(&mut self) -> std::io::Result<()> {
        let id = self
            .message_id
            .clone()
            .unwrap_or_else(|| "msg_claudie_proxy".to_string());
        let model = self
            .model
            .clone()
            .unwrap_or_else(|| self.fallback_model.clone());
        self.write_event(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": {
                        // OpenAI usage only arrives at the end of the stream, so seed
                        // input_tokens with an estimate now (overwritten by the real
                        // value in message_delta) to mirror native message_start.
                        "input_tokens": self.estimated_input_tokens,
                        "output_tokens": 0,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0
                    }
                }
            }),
        )?;
        self.started = true;
        // Native Anthropic streams interleave a ping right after message_start.
        self.write_event("ping", json!({ "type": "ping" }))?;
        Ok(())
    }

    /// Upstream signalled an error partway through the stream. Emit a native
    /// Anthropic `error` event (after closing any open blocks) so Claude Code
    /// surfaces the failure and can retry, instead of treating a truncated
    /// stream as a successful turn. The message is also recorded by the caller.
    fn emit_stream_error(&mut self, error: &Value) -> std::io::Result<()> {
        if !self.started {
            self.emit_message_start()?;
        }
        self.close_open_blocks()?;
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("upstream streaming error");
        let message = shorten_for_error(message);
        let err_type = error
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("api_error")
            .to_string();
        self.error_message = Some(message.clone());
        self.write_event(
            "error",
            json!({
                "type": "error",
                "error": { "type": err_type, "message": message }
            }),
        )
    }

    /// Close every opened block in the order they were assigned. Anthropic SDK
    /// tolerates any order, but ascending matches the natural DeepSeek/Qwen flow
    /// (thinking → text → tool) and reads cleanly in logs.
    fn close_open_blocks(&mut self) -> std::io::Result<()> {
        let mut open_indices: Vec<usize> = Vec::new();
        if let Some(index) = self.thinking_block_index {
            open_indices.push(index);
        }
        if let Some(index) = self.text_block_index {
            open_indices.push(index);
        }
        open_indices.extend(
            self.tool_block_by_openai_index
                .values()
                .map(|b| b.block_index),
        );
        open_indices.sort();
        for index in open_indices {
            self.write_event(
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": index }),
            )?;
        }
        Ok(())
    }

    fn finalize(&mut self) -> std::io::Result<()> {
        if !self.started {
            self.emit_message_start()?;
        }
        self.close_open_blocks()?;
        let stop_reason = self.stop_reason.unwrap_or("end_turn");
        // OpenAI folds cached prompt tokens into prompt_tokens; Anthropic reports
        // them separately, so subtract to keep Claude Code's /cost accurate.
        let cache_read = self.cache_read_tokens.min(self.input_tokens);
        self.write_event(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": Value::Null },
                "usage": {
                    "input_tokens": self.input_tokens - cache_read,
                    "output_tokens": self.output_tokens,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": cache_read
                }
            }),
        )?;
        self.write_event("message_stop", json!({ "type": "message_stop" }))
    }

    fn write_event(&mut self, event: &str, data: Value) -> std::io::Result<()> {
        write!(self.out, "event: {event}\n")?;
        write!(self.out, "data: {data}\n\n")?;
        self.out.flush()
    }
}

fn map_finish_reason(reason: &str) -> &'static str {
    match reason {
        "tool_calls" | "function_call" => "tool_use",
        "length" => "max_tokens",
        "content_filter" => "stop_sequence",
        _ => "end_turn",
    }
}

fn is_client_disconnect(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_translator(chunks: &[Value]) -> Vec<(String, Value)> {
        let mut sse_input = String::new();
        for chunk in chunks {
            sse_input.push_str("data: ");
            sse_input.push_str(&chunk.to_string());
            sse_input.push_str("\n\n");
        }
        sse_input.push_str("data: [DONE]\n\n");
        let reader = std::io::BufReader::new(std::io::Cursor::new(sse_input.into_bytes()));
        let mut out: Vec<u8> = Vec::new();
        {
            let mut translator = StreamTranslator::new(&mut out, "fallback-model".to_string(), 0);
            translator.run(reader).expect("translator runs");
        }
        // Translator output does not include HTTP header; feed raw events directly.
        // `ping` is filtered out so structural assertions below focus on content
        // events; a dedicated test covers the ping itself.
        let mut events = Vec::new();
        let text = String::from_utf8(out).expect("utf8");
        let mut current_event: Option<String> = None;
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("event:") {
                current_event = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                let payload: Value = serde_json::from_str(rest.trim()).expect("valid json data");
                let name = current_event.clone().unwrap_or_default();
                if name != "ping" {
                    events.push((name, payload));
                }
            }
        }
        events
    }

    #[test]
    fn translator_emits_text_stream_events() {
        let chunks = vec![
            json!({ "id": "chatcmpl-1", "model": "gpt-4o-test", "choices": [{ "delta": { "content": "Hel" } }] }),
            json!({ "id": "chatcmpl-1", "choices": [{ "delta": { "content": "lo" } }] }),
            json!({ "id": "chatcmpl-1", "choices": [{ "delta": { "content": "!" }, "finish_reason": "stop" }] }),
            json!({ "id": "chatcmpl-1", "choices": [], "usage": { "prompt_tokens": 5, "completion_tokens": 3 } }),
        ];

        let events = run_translator(&chunks);
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
        assert_eq!(events[0].1["message"]["model"], "gpt-4o-test");
        assert_eq!(events[1].1["index"], 0);
        assert_eq!(events[1].1["content_block"]["type"], "text");
        assert_eq!(events[2].1["delta"]["text"], "Hel");
        assert_eq!(events[3].1["delta"]["text"], "lo");
        assert_eq!(events[4].1["delta"]["text"], "!");
        assert_eq!(events[5].1["index"], 0);
        assert_eq!(events[6].1["delta"]["stop_reason"], "end_turn");
        assert_eq!(events[6].1["usage"]["input_tokens"], 5);
        assert_eq!(events[6].1["usage"]["output_tokens"], 3);
    }

    #[test]
    fn translator_emits_single_tool_call_with_argument_fragments() {
        let chunks = vec![
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "index": 0, "id": "call_1", "type": "function",
                    "function": { "name": "read_file", "arguments": "{\"path\":\"" }
                }] } }]
            }),
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "index": 0, "function": { "arguments": "/tmp/a" }
                }] } }]
            }),
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "index": 0, "function": { "arguments": "\"}" }
                }] }, "finish_reason": "tool_calls" }]
            }),
        ];

        let events = run_translator(&chunks);
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
        let block_start = &events[1].1;
        assert_eq!(block_start["content_block"]["type"], "tool_use");
        assert_eq!(block_start["content_block"]["id"], "call_1");
        assert_eq!(block_start["content_block"]["name"], "read_file");
        assert_eq!(events[2].1["delta"]["partial_json"], "{\"path\":\"");
        assert_eq!(events[3].1["delta"]["partial_json"], "/tmp/a");
        assert_eq!(events[4].1["delta"]["partial_json"], "\"}");
        assert_eq!(events[6].1["delta"]["stop_reason"], "tool_use");
    }

    #[test]
    fn translator_handles_parallel_tool_calls() {
        let chunks = vec![json!({
            "id": "chatcmpl-1",
            "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "id": "call_a", "type": "function", "function": { "name": "Read", "arguments": "{}" } },
                { "index": 1, "id": "call_b", "type": "function", "function": { "name": "Bash", "arguments": "{}" } }
            ] }, "finish_reason": "tool_calls" }]
        })];

        let events = run_translator(&chunks);
        let starts: Vec<&Value> = events
            .iter()
            .filter(|(name, _)| name == "content_block_start")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0]["index"], 0);
        assert_eq!(starts[0]["content_block"]["id"], "call_a");
        assert_eq!(starts[1]["index"], 1);
        assert_eq!(starts[1]["content_block"]["id"], "call_b");
        let stops: Vec<&Value> = events
            .iter()
            .filter(|(name, _)| name == "content_block_stop")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0]["index"], 0);
        assert_eq!(stops[1]["index"], 1);
    }

    #[test]
    fn translator_maps_cached_tokens_into_message_delta_usage() {
        let chunks = vec![
            json!({ "id": "c", "choices": [{ "delta": { "content": "hi" }, "finish_reason": "stop" }] }),
            json!({ "id": "c", "choices": [], "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 4,
                "prompt_tokens_details": { "cached_tokens": 900 }
            } }),
        ];
        let events = run_translator(&chunks);
        let delta = events
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta");
        assert_eq!(delta.1["usage"]["input_tokens"], 100);
        assert_eq!(delta.1["usage"]["cache_read_input_tokens"], 900);
    }

    #[test]
    fn translator_maps_length_finish_reason_to_max_tokens() {
        let chunks = vec![
            json!({ "id": "chatcmpl-1", "choices": [{ "delta": { "content": "hi" } }] }),
            json!({ "id": "chatcmpl-1", "choices": [{ "delta": {}, "finish_reason": "length" }] }),
        ];
        let events = run_translator(&chunks);
        let delta = events
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta");
        assert_eq!(delta.1["delta"]["stop_reason"], "max_tokens");
    }

    #[test]
    fn translator_emits_error_event_on_mid_stream_error_chunk() {
        let chunks = vec![
            json!({ "id": "chatcmpl-1", "choices": [{ "delta": { "content": "hi" } }] }),
            json!({ "error": { "type": "server_error", "message": "boom" } }),
        ];
        let events = run_translator(&chunks);
        // Open blocks are closed, then a native Anthropic `error` event is emitted
        // (no message_delta/message_stop) so Claude Code can surface and retry.
        let error = events
            .iter()
            .find(|(name, _)| name == "error")
            .expect("error event emitted");
        assert_eq!(error.1["error"]["type"], "server_error");
        assert_eq!(error.1["error"]["message"], "boom");
        assert_eq!(events.last().unwrap().0, "error");
        assert!(events.iter().any(|(name, _)| name == "content_block_stop"));
        assert!(events.iter().all(|(name, _)| name != "message_stop"));
    }

    #[test]
    fn translator_emits_ping_after_message_start() {
        let sse_input = "data: {\"id\":\"c\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
        let reader = std::io::BufReader::new(std::io::Cursor::new(sse_input.as_bytes().to_vec()));
        let mut out: Vec<u8> = Vec::new();
        {
            let mut translator = StreamTranslator::new(&mut out, "m".to_string(), 7);
            translator.run(reader).unwrap();
        }
        let text = String::from_utf8(out).unwrap();
        let start = text.find("event: message_start").expect("message_start");
        let ping = text.find("event: ping").expect("ping emitted");
        assert!(ping > start, "ping must follow message_start");
        // message_start seeds input_tokens with the estimate.
        assert!(text.contains("\"input_tokens\":7"));
    }

    #[test]
    fn translator_writes_minimal_message_when_no_chunks_received() {
        let reader = std::io::BufReader::new(std::io::Cursor::new(b"data: [DONE]\n\n".to_vec()));
        let mut out: Vec<u8> = Vec::new();
        {
            let mut translator = StreamTranslator::new(&mut out, "fallback-model".to_string(), 0);
            translator.run(reader).unwrap();
        }
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("event: message_start"));
        assert!(text.contains("event: message_delta"));
        assert!(text.contains("event: message_stop"));
    }

    #[test]
    fn translator_emits_thinking_then_text_for_reasoning_stream() {
        let chunks = vec![
            json!({ "id": "chatcmpl-r1", "model": "deepseek-r1", "choices": [{ "delta": { "reasoning_content": "I " } }] }),
            json!({ "id": "chatcmpl-r1", "choices": [{ "delta": { "reasoning_content": "think" } }] }),
            json!({ "id": "chatcmpl-r1", "choices": [{ "delta": { "reasoning_content": "..." } }] }),
            json!({ "id": "chatcmpl-r1", "choices": [{ "delta": { "content": "Final" } }] }),
            json!({ "id": "chatcmpl-r1", "choices": [{ "delta": { "content": " answer" }, "finish_reason": "stop" }] }),
        ];
        let events = run_translator(&chunks);
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "message_start",
                "content_block_start", // thinking
                "content_block_delta", // thinking_delta "I "
                "content_block_delta", // thinking_delta "think"
                "content_block_delta", // thinking_delta "..."
                "content_block_start", // text
                "content_block_delta", // text_delta "Final"
                "content_block_delta", // text_delta " answer"
                "content_block_stop",  // thinking
                "content_block_stop",  // text
                "message_delta",
                "message_stop",
            ]
        );
        assert_eq!(events[1].1["index"], 0);
        assert_eq!(events[1].1["content_block"]["type"], "thinking");
        assert_eq!(events[2].1["delta"]["type"], "thinking_delta");
        assert_eq!(events[2].1["delta"]["thinking"], "I ");
        assert_eq!(events[5].1["index"], 1);
        assert_eq!(events[5].1["content_block"]["type"], "text");
        assert_eq!(events[6].1["delta"]["text"], "Final");
        // Close thinking (index 0) before text (index 1).
        assert_eq!(events[8].1["index"], 0);
        assert_eq!(events[9].1["index"], 1);
    }

    #[test]
    fn translator_thinking_with_tool_call_indexes_correctly() {
        let chunks = vec![
            json!({ "id": "c", "choices": [{ "delta": { "reasoning_content": "thinking..." } }] }),
            json!({ "id": "c", "choices": [{ "delta": { "tool_calls": [{
                "index": 0, "id": "call_x", "type": "function",
                "function": { "name": "Read", "arguments": "{}" }
            }] }, "finish_reason": "tool_calls" }] }),
        ];
        let events = run_translator(&chunks);
        let starts: Vec<&Value> = events
            .iter()
            .filter(|(name, _)| name == "content_block_start")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0]["index"], 0);
        assert_eq!(starts[0]["content_block"]["type"], "thinking");
        assert_eq!(starts[1]["index"], 1);
        assert_eq!(starts[1]["content_block"]["type"], "tool_use");
    }
}
