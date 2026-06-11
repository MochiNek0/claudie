use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::app::AppState;
use crate::settings::LlmProfile;

use super::capability_cache::save_tool_history_capability;
use super::http::shorten_for_error;
use super::request_conv::estimate_request_input_tokens;
use super::response_conv::{cached_input_tokens, map_finish_reason};
use super::tool_history::{should_retry_with_tool_transcript, tool_history_as_text_transcript};
use super::upstream::call_openai_streaming;
use super::{anthropic_error_type_for_upstream, record_proxy_error, write_upstream_error};

/// Upper bound for a single SSE line from the upstream. Normal chunks are a
/// few KB; the generous limit only guards against a broken upstream streaming
/// an endless line into memory.
const MAX_SSE_LINE_BYTES: usize = 16 * 1024 * 1024;

enum SseLine {
    Eof,
    Line,
    TooLong,
}

/// Read one `\n`-terminated line into `line`, giving up once it exceeds `cap`
/// bytes. Unlike `BufRead::read_line`, the cap is enforced while reading, so a
/// broken upstream cannot grow an endless line into memory before the check.
fn read_sse_line<R: BufRead>(
    reader: &mut R,
    line: &mut String,
    cap: usize,
) -> std::io::Result<SseLine> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(SseLine::Eof);
            }
            return line_from_bytes(bytes, line);
        }
        match available.iter().position(|&byte| byte == b'\n') {
            Some(pos) => {
                bytes.extend_from_slice(&available[..=pos]);
                reader.consume(pos + 1);
                return line_from_bytes(bytes, line);
            }
            None => {
                let len = available.len();
                bytes.extend_from_slice(available);
                reader.consume(len);
                if bytes.len() > cap {
                    return Ok(SseLine::TooLong);
                }
            }
        }
    }
}

fn line_from_bytes(bytes: Vec<u8>, line: &mut String) -> std::io::Result<SseLine> {
    match String::from_utf8(bytes) {
        Ok(text) => {
            *line = text;
            Ok(SseLine::Line)
        }
        // Match BufRead::read_line, which fails on invalid UTF-8.
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )),
    }
}

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
) {
    match call_openai_streaming(agent, profile, outbound_request) {
        Ok(reader) => {
            let _ = stream_sse_to_client(stream, reader, state, profile, anthropic_request);
        }
        Err(err)
            if !use_tool_transcript && should_retry_with_tool_transcript(&err, openai_request) =>
        {
            let _ = save_tool_history_capability(profile, openai_request, false);
            let fallback = tool_history_as_text_transcript(openai_request);
            match call_openai_streaming(agent, profile, &fallback) {
                Ok(reader) => {
                    let _ = stream_sse_to_client(stream, reader, state, profile, anthropic_request);
                }
                Err(retry_err) => {
                    let combined = format!("{err}; text tool transcript retry failed: {retry_err}");
                    write_upstream_error(stream, state, &retry_err, combined);
                }
            }
        }
        Err(err) => {
            let message = err.to_string();
            write_upstream_error(stream, state, &err, message);
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

struct ToolCallState {
    block_index: usize,
    id: Option<String>,
    name: Option<String>,
    pending_arguments: String,
    started: bool,
}

impl ToolCallState {
    fn new(block_index: usize) -> Self {
        Self {
            block_index,
            id: None,
            name: None,
            pending_arguments: String::new(),
            started: false,
        }
    }

    fn ready_to_start(&self) -> bool {
        self.id.is_some() && self.name.is_some()
    }

    fn absorb_id(&mut self, id: Option<&str>) {
        if self.id.is_none() {
            if let Some(id) = id.filter(|value| !value.is_empty()) {
                self.id = Some(id.to_string());
            }
        }
    }

    fn absorb_name(&mut self, name: Option<&str>) {
        if self.name.is_none() {
            if let Some(name) = name.filter(|value| !value.is_empty()) {
                self.name = Some(name.to_string());
            }
        }
    }

    fn start_payload(&mut self) -> ToolCallStart {
        self.started = true;
        ToolCallStart {
            block_index: self.block_index,
            id: self.id.clone().unwrap_or_else(|| "tool_call".to_string()),
            name: self.name.clone().unwrap_or_else(|| "tool".to_string()),
            arguments: std::mem::take(&mut self.pending_arguments),
        }
    }
}

struct ToolCallStart {
    block_index: usize,
    id: String,
    name: String,
    arguments: String,
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
    /// Keyed by the upstream `index`, or by a synthetic slot for upstreams
    /// that omit `index` on tool_call deltas (see `resolve_tool_slot`).
    tool_calls_by_openai_index: HashMap<u64, ToolCallState>,
    /// tool_call id → slot, for index-less deltas that repeat a known id.
    id_to_slot: Vec<(String, u64)>,
    last_tool_slot: Option<u64>,
    /// Synthetic slots start far above any real upstream index so the two
    /// key spaces never collide when an upstream mixes both styles.
    synthetic_next: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    stop_reason: Option<&'static str>,
    error_message: Option<String>,
    malformed_chunk_count: usize,
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
            tool_calls_by_openai_index: HashMap::new(),
            id_to_slot: Vec::new(),
            last_tool_slot: None,
            synthetic_next: 1 << 40,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            stop_reason: None,
            error_message: None,
            malformed_chunk_count: 0,
        }
    }

    fn run<R: BufRead>(&mut self, mut reader: R) -> std::io::Result<()> {
        let mut line = String::new();
        loop {
            match read_sse_line(&mut reader, &mut line, MAX_SSE_LINE_BYTES) {
                Ok(SseLine::Eof) => break,
                Ok(SseLine::Line) => {}
                Ok(SseLine::TooLong) => {
                    self.error_message.get_or_insert_with(|| {
                        "upstream sent an oversized SSE line; stream truncated".to_string()
                    });
                    break;
                }
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
                Err(_) => {
                    // Hot path: only count here; one summary line is surfaced
                    // via error_message after the stream ends.
                    self.malformed_chunk_count += 1;
                    continue;
                }
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

    /// Map a tool_call delta to a stable slot key. Most upstreams tag every
    /// delta with `index`; some OpenAI-compatible servers omit it, sending the
    /// id only on the first frame of each call. Without a fallback, all such
    /// calls would collapse into slot 0 and their arguments would interleave.
    fn resolve_tool_slot(&mut self, explicit_index: Option<u64>, id: Option<&str>) -> u64 {
        let id = id.filter(|value| !value.is_empty());
        let known = id.and_then(|id| {
            self.id_to_slot
                .iter()
                .find_map(|(known, slot)| (known == id).then_some(*slot))
        });
        let slot = if let Some(index) = explicit_index {
            index
        } else if let Some(slot) = known {
            slot
        } else if id.is_some() {
            // A fresh id without an index opens a new tool call.
            self.allocate_synthetic_slot()
        } else if let Some(slot) = self.last_tool_slot {
            // Argument fragments often arrive with neither index nor id; they
            // belong to the most recent call.
            slot
        } else {
            self.allocate_synthetic_slot()
        };
        if known.is_none() {
            if let Some(id) = id {
                self.id_to_slot.push((id.to_string(), slot));
            }
        }
        self.last_tool_slot = Some(slot);
        slot
    }

    fn allocate_synthetic_slot(&mut self) -> u64 {
        let slot = self.synthetic_next;
        self.synthetic_next += 1;
        slot
    }

    fn absorb_tool_call(&mut self, tc: &Value) -> std::io::Result<()> {
        let explicit_index = tc.get("index").and_then(Value::as_u64);
        let id = tc.get("id").and_then(Value::as_str);
        let slot_key = self.resolve_tool_slot(explicit_index, id);
        let name = tc.pointer("/function/name").and_then(Value::as_str);
        let arguments = tc
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .filter(|arg_frag| !arg_frag.is_empty());

        let delta = {
            let tool_call = self
                .tool_calls_by_openai_index
                .entry(slot_key)
                .or_insert_with(|| {
                    let block_index = self.next_block_index;
                    self.next_block_index += 1;
                    ToolCallState::new(block_index)
                });
            tool_call.absorb_id(id);
            tool_call.absorb_name(name);

            if tool_call.started {
                arguments.map(|arguments| (tool_call.block_index, arguments.to_string()))
            } else {
                if let Some(arguments) = arguments {
                    tool_call.pending_arguments.push_str(arguments);
                }
                None
            }
        };

        if let Some((block_index, arguments)) = delta {
            self.write_tool_arguments_delta(block_index, &arguments)?;
        }
        self.flush_ready_tool_blocks(false)
    }

    fn write_tool_block_start(
        &mut self,
        block_index: usize,
        id: &str,
        name: &str,
    ) -> std::io::Result<()> {
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
        )
    }

    fn write_tool_arguments_delta(
        &mut self,
        block_index: usize,
        arguments: &str,
    ) -> std::io::Result<()> {
        if arguments.is_empty() {
            return Ok(());
        }
        self.write_event(
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": block_index,
                "delta": { "type": "input_json_delta", "partial_json": arguments }
            }),
        )
    }

    fn flush_pending_tool_blocks(&mut self) -> std::io::Result<()> {
        self.flush_ready_tool_blocks(true)
    }

    fn flush_ready_tool_blocks(&mut self, force: bool) -> std::io::Result<()> {
        let mut keys = self
            .tool_calls_by_openai_index
            .iter()
            .map(|(key, tool_call)| (tool_call.block_index, *key))
            .collect::<Vec<_>>();
        keys.sort_by_key(|(block_index, _)| *block_index);

        let mut starts = Vec::new();
        for (_, key) in keys {
            let Some(tool_call) = self.tool_calls_by_openai_index.get_mut(&key) else {
                continue;
            };
            if tool_call.started {
                continue;
            }
            if force || tool_call.ready_to_start() {
                starts.push(tool_call.start_payload());
            } else {
                break;
            }
        }
        for start in starts {
            self.write_tool_block_start(start.block_index, &start.id, &start.name)?;
            self.write_tool_arguments_delta(start.block_index, &start.arguments)?;
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
        // Claude Code only understands Anthropic's native error types.
        let err_type = anthropic_error_type_for_upstream(
            error
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("api_error"),
        );
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
        self.flush_pending_tool_blocks()?;
        let mut open_indices: Vec<usize> = Vec::new();
        if let Some(index) = self.thinking_block_index {
            open_indices.push(index);
        }
        if let Some(index) = self.text_block_index {
            open_indices.push(index);
        }
        open_indices.extend(
            self.tool_calls_by_openai_index
                .values()
                .filter(|b| b.started)
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
        if self.malformed_chunk_count > 0 {
            // A real mid-stream error message takes priority over this summary.
            self.error_message.get_or_insert_with(|| {
                format!(
                    "skipped {} malformed SSE chunk(s) from upstream",
                    self.malformed_chunk_count
                )
            });
        }
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
    fn translator_buffers_tool_arguments_until_id_and_name_arrive() {
        let chunks = vec![
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "index": 0,
                    "function": { "arguments": "{\"path\":\"" }
                }] } }]
            }),
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "index": 0, "id": "call_late", "type": "function",
                    "function": { "name": "Read", "arguments": "README.md\"}" }
                }] }, "finish_reason": "tool_calls" }]
            }),
        ];

        let events = run_translator(&chunks);
        let starts: Vec<&Value> = events
            .iter()
            .filter(|(name, _)| name == "content_block_start")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["content_block"]["id"], "call_late");
        assert_eq!(starts[0]["content_block"]["name"], "Read");

        let deltas: Vec<&Value> = events
            .iter()
            .filter(|(name, value)| {
                name == "content_block_delta" && value["delta"]["type"] == "input_json_delta"
            })
            .map(|(_, v)| v)
            .collect();
        assert_eq!(deltas.len(), 1);
        assert_eq!(
            deltas[0]["delta"]["partial_json"],
            "{\"path\":\"README.md\"}"
        );
    }

    #[test]
    fn translator_flushes_incomplete_tool_call_on_done() {
        let chunks = vec![json!({
            "id": "chatcmpl-1",
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "function": { "arguments": "{\"query\":\"status\"}" }
            }] }, "finish_reason": "tool_calls" }]
        })];

        let events = run_translator(&chunks);
        let start = events
            .iter()
            .find(|(name, _)| name == "content_block_start")
            .expect("fallback tool block starts");
        assert_eq!(start.1["content_block"]["id"], "tool_call");
        assert_eq!(start.1["content_block"]["name"], "tool");
        let delta = events
            .iter()
            .find(|(name, value)| {
                name == "content_block_delta" && value["delta"]["type"] == "input_json_delta"
            })
            .expect("buffered tool arguments flush");
        assert_eq!(delta.1["delta"]["partial_json"], "{\"query\":\"status\"}");
        assert!(events.iter().any(|(name, _)| name == "content_block_stop"));
    }

    #[test]
    fn translator_keeps_tool_block_start_order_when_later_call_is_ready_first() {
        let chunks = vec![
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [
                    { "index": 0, "function": { "arguments": "{\"a\":" } },
                    { "index": 1, "id": "call_b", "type": "function", "function": { "name": "Second", "arguments": "{}" } }
                ] } }]
            }),
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "index": 0, "id": "call_a", "type": "function",
                    "function": { "name": "First", "arguments": "1}" }
                }] }, "finish_reason": "tool_calls" }]
            }),
        ];

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
    }

    #[test]
    fn translator_separates_indexless_tool_calls_by_id() {
        // Some OpenAI-compatible upstreams omit `index` on tool_call deltas.
        // Two calls with distinct ids must land in distinct blocks instead of
        // collapsing into slot 0.
        let chunks = vec![
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "id": "call_a", "type": "function",
                    "function": { "name": "Read", "arguments": "{}" }
                }] } }]
            }),
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "id": "call_b", "type": "function",
                    "function": { "name": "Bash", "arguments": "{}" }
                }] }, "finish_reason": "tool_calls" }]
            }),
        ];

        let events = run_translator(&chunks);
        let starts: Vec<&Value> = events
            .iter()
            .filter(|(name, _)| name == "content_block_start")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0]["content_block"]["id"], "call_a");
        assert_eq!(starts[1]["content_block"]["id"], "call_b");
        assert_ne!(starts[0]["index"], starts[1]["index"]);
    }

    #[test]
    fn translator_joins_indexless_argument_fragments_to_last_tool_call() {
        // Continuation frames often carry neither index nor id; they belong to
        // the most recent call.
        let chunks = vec![
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "id": "call_a", "type": "function",
                    "function": { "name": "Read", "arguments": "{\"p\":\"" }
                }] } }]
            }),
            json!({
                "id": "chatcmpl-1",
                "choices": [{ "delta": { "tool_calls": [{
                    "function": { "arguments": "x\"}" }
                }] }, "finish_reason": "tool_calls" }]
            }),
        ];

        let events = run_translator(&chunks);
        let starts: Vec<&Value> = events
            .iter()
            .filter(|(name, _)| name == "content_block_start")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["content_block"]["id"], "call_a");
        let arguments: String = events
            .iter()
            .filter(|(name, value)| {
                name == "content_block_delta" && value["delta"]["type"] == "input_json_delta"
            })
            .map(|(_, v)| v["delta"]["partial_json"].as_str().unwrap())
            .collect();
        assert_eq!(arguments, "{\"p\":\"x\"}");
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
    fn translator_maps_content_filter_finish_reason_to_refusal() {
        let chunks = vec![
            json!({ "id": "chatcmpl-1", "choices": [{ "delta": { "content": "hi" } }] }),
            json!({ "id": "chatcmpl-1", "choices": [{ "delta": {}, "finish_reason": "content_filter" }] }),
        ];
        let events = run_translator(&chunks);
        let delta = events
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta");
        assert_eq!(delta.1["delta"]["stop_reason"], "refusal");
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
        assert_eq!(error.1["error"]["type"], "api_error");
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
    fn read_sse_line_caps_endless_lines_without_buffering_them() {
        // A long line with no newline must stop at the cap instead of buffering
        // the entire stream.
        let input = vec![b'x'; 64];
        let mut reader = std::io::BufReader::with_capacity(8, std::io::Cursor::new(input));
        let mut line = String::new();
        assert!(matches!(
            read_sse_line(&mut reader, &mut line, 16).unwrap(),
            SseLine::TooLong
        ));

        // A line within the cap is returned intact, newline preserved.
        let mut reader =
            std::io::BufReader::with_capacity(8, std::io::Cursor::new(b"data: hi\nrest".to_vec()));
        assert!(matches!(
            read_sse_line(&mut reader, &mut line, 16).unwrap(),
            SseLine::Line
        ));
        assert_eq!(line, "data: hi\n");

        // EOF without trailing newline still yields the final line, then Eof.
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(b"tail".to_vec()));
        assert!(matches!(
            read_sse_line(&mut reader, &mut line, 16).unwrap(),
            SseLine::Line
        ));
        assert_eq!(line, "tail");
        assert!(matches!(
            read_sse_line(&mut reader, &mut line, 16).unwrap(),
            SseLine::Eof
        ));
    }

    #[test]
    fn translator_truncates_stream_on_oversized_sse_line() {
        // One valid chunk, then a line just over MAX_SSE_LINE_BYTES with no
        // terminating newline: the run loop must truncate and finalize.
        let mut sse_input = Vec::new();
        sse_input.extend_from_slice(
            b"data: {\"id\":\"c\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
        );
        sse_input.extend_from_slice(b"data: ");
        sse_input.extend(std::iter::repeat_n(b'x', MAX_SSE_LINE_BYTES + 1));
        let reader = std::io::BufReader::new(std::io::Cursor::new(sse_input));
        let mut out: Vec<u8> = Vec::new();
        let error_message = {
            let mut translator = StreamTranslator::new(&mut out, "m".to_string(), 0);
            translator.run(reader).unwrap();
            translator.error_message.clone()
        };
        assert!(error_message.unwrap().contains("oversized SSE line"));
        let text = String::from_utf8(out).unwrap();
        // Stream is finalized normally so the client still gets a valid message.
        assert!(text.contains("event: message_stop"));
    }

    #[test]
    fn translator_counts_malformed_chunks_and_keeps_stream_alive() {
        let sse_input = "data: {not json\n\n\
            data: also not json\n\n\
            data: {\"id\":\"c\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n\
            data: [DONE]\n\n";
        let reader = std::io::BufReader::new(std::io::Cursor::new(sse_input.as_bytes().to_vec()));
        let mut out: Vec<u8> = Vec::new();
        let (malformed, error_message) = {
            let mut translator = StreamTranslator::new(&mut out, "m".to_string(), 0);
            translator.run(reader).unwrap();
            (
                translator.malformed_chunk_count,
                translator.error_message.clone(),
            )
        };
        assert_eq!(malformed, 2);
        assert!(
            error_message
                .unwrap()
                .contains("skipped 2 malformed SSE chunk(s)")
        );
        let text = String::from_utf8(out).unwrap();
        // Valid chunks still translate and the stream closes cleanly.
        assert!(text.contains("\"text\":\"hi\""));
        assert!(text.contains("event: message_stop"));
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
