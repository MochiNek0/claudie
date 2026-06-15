mod capability_cache;
mod http;
mod provider;
mod request_conv;
mod response_conv;
mod streaming;
mod tool_history;
mod upstream;

use std::borrow::Cow;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::app::{AppState, PetMood};
use crate::config::DEFAULT_PROXY_PORT;
use crate::proxy_optimizer;
use crate::settings::LlmProfile;
use crate::util::ConnectionLimiter;

use capability_cache::{cached_tool_history_needs_transcript, save_tool_history_capability};
use http::{HttpRequest, read_http_request, write_json_response, write_json_response_with_headers};
use provider::model_supports_tools;
use request_conv::anthropic_to_openai_request;
use response_conv::openai_to_anthropic_response;
use streaming::run_streaming_request;
use tool_history::{should_retry_with_tool_transcript, tool_history_as_text_transcript};
use upstream::{UpstreamError, call_openai, proxy_status_for_upstream_error};

const MAX_PROXY_CONNECTIONS: usize = 32;

pub(crate) fn start_openai_proxy_server(state: Arc<Mutex<AppState>>) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", DEFAULT_PROXY_PORT))
        .map_err(|err| format!("OpenAI proxy failed: {err}"))?;

    thread::spawn(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(600))
            .build();
        let limiter = ConnectionLimiter::new(MAX_PROXY_CONNECTIONS);

        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let Some(permit) = limiter.try_acquire() else {
                        write_error_response(&mut stream, 503, "claudie proxy is busy");
                        continue;
                    };
                    let state = state.clone();
                    let agent = agent.clone();
                    thread::spawn(move || {
                        let _permit = permit;
                        handle_proxy_client(stream, state, agent);
                    });
                }
                Err(err) => {
                    record_proxy_error(&state, format!("OpenAI proxy accept failed: {err}"))
                }
            }
        }
    });

    Ok(())
}

/// User-Agent for model-list probes. Some `/models` endpoints (coding plans,
/// Claude Code relays) gate on a Claude Code style agent string.
const MODEL_FETCH_USER_AGENT: &str = "claude-code/2.1";

/// Known "Anthropic protocol mounted on a sub-path" suffixes. When a base URL
/// ends with one of these, the OpenAI-style model list usually lives at the
/// provider root, so candidates also probe the stripped root. Ordered most
/// specific first so `/api/anthropic` wins over `/anthropic`.
const KNOWN_COMPAT_SUFFIXES: &[&str] = &[
    "/api/claudecode",
    "/api/anthropic",
    "/apps/anthropic",
    "/api/coding",
    "/claudecode",
    "/anthropic",
    "/step_plan",
    "/coding",
    "/claude",
];

/// Fetch the provider's model list for the Settings UI.
///
/// Providers — OpenAI aggregators and Anthropic-compatible relays alike — expose
/// the list through an OpenAI-style `GET /v1/models` (or `/models`) endpoint
/// authenticated with a bearer token, so that is the primary probe. The genuine
/// Anthropic API (api.anthropic.com) instead wants `x-api-key`, and the keyless
/// official profile reuses Claude Code's OAuth token. Several candidate URLs are
/// tried in turn; the first that answers wins.
pub(crate) fn fetch_provider_models(profile: &LlmProfile) -> Result<Vec<String>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();

    // Resolve the credential. The OAuth token is only ever sent to the official
    // profile so it cannot leak to a third-party base URL.
    let api_key = profile.api_key.trim();
    let auth_token = profile.auth_token.trim();
    let (key, is_oauth) = if !api_key.is_empty() {
        (api_key.to_string(), false)
    } else if !auth_token.is_empty() {
        (auth_token.to_string(), false)
    } else if profile.is_official() {
        match crate::official_usage::oauth_access_token() {
            Some(token) => (token, true),
            None => return Err("No Claude Code OAuth token is available.".to_string()),
        }
    } else {
        return Err("Set an API key or auth token before fetching models.".to_string());
    };

    let mut last_err = String::new();
    for url in model_url_candidates(profile) {
        let host_is_anthropic = url.starts_with("https://api.anthropic.com")
            || url.starts_with("http://api.anthropic.com");
        let request = agent.get(&url).set("User-Agent", MODEL_FETCH_USER_AGENT);
        // One auth style per case: bearer for OpenAI-compatible hosts, `x-api-key`
        // for the genuine Anthropic API, OAuth bearer + beta for the official one.
        let request = if is_oauth {
            request
                .set("Authorization", &format!("Bearer {key}"))
                .set("anthropic-beta", crate::official_usage::oauth_beta_header())
                .set("anthropic-version", "2023-06-01")
        } else if host_is_anthropic {
            request
                .set("x-api-key", &key)
                .set("anthropic-version", "2023-06-01")
        } else {
            request.set("Authorization", &format!("Bearer {key}"))
        };
        match parse_models_response(request.call()) {
            Ok(models) => return Ok(models),
            Err(err) => last_err = err,
        }
    }
    Err(if last_err.is_empty() {
        "No model list endpoint responded.".to_string()
    } else {
        last_err
    })
}

/// Candidate `/models` URLs to probe for a profile, in priority order.
fn model_url_candidates(profile: &LlmProfile) -> Vec<String> {
    let base = profile.base_url.trim().trim_end_matches('/');
    // OpenAI chat-completions style: derive the sibling `/models` endpoint.
    if profile.is_openai_chat_proxy() || base.contains("/chat/completions") {
        return vec![
            profile
                .openai_chat_completions_url()
                .replace("/chat/completions", "/models"),
        ];
    }
    anthropic_model_candidates(base)
}

/// Candidate model-list URLs for an Anthropic-format base URL: the endpoint under
/// the configured base, then the provider root in OpenAI form (covering relays
/// like `https://host/anthropic` whose `/models` lives at the root).
fn anthropic_model_candidates(base_url: &str) -> Vec<String> {
    let base = base_url.trim().trim_end_matches('/');
    let base = if base.is_empty() {
        "https://api.anthropic.com"
    } else {
        base
    };
    let mut urls: Vec<String> = Vec::new();
    let mut push = |url: String| {
        if !urls.contains(&url) {
            urls.push(url);
        }
    };

    // A base already ending in a version segment (`/v1`, `/api/coding/paas/v4`)
    // takes `/models`; otherwise the OpenAI convention is `/v1/models`.
    if ends_with_version_segment(base) {
        push(format!("{base}/models"));
        if !base.ends_with("/v1") {
            push(format!("{base}/v1/models"));
        }
    } else {
        push(format!("{base}/v1/models"));
    }

    if let Some(root) = strip_compat_suffix(base) {
        let root = root.trim_end_matches('/');
        if root.contains("://") && !root.ends_with("://") {
            push(format!("{root}/v1/models"));
            push(format!("{root}/models"));
        }
    }
    urls
}

/// True when a URL's last path segment is an OpenAI-style version like `v1`/`v4`.
fn ends_with_version_segment(url: &str) -> bool {
    let last = url.rsplit('/').next().unwrap_or("");
    last.strip_prefix('v')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// If the base ends with a known Anthropic-compat sub-path, return the prefix.
fn strip_compat_suffix(base_url: &str) -> Option<&str> {
    KNOWN_COMPAT_SUFFIXES
        .iter()
        .find_map(|suffix| base_url.strip_suffix(suffix))
}

/// Extract model ids from an OpenAI- or Anthropic-style `{ "data": [{ "id" }] }`
/// list response.
fn parse_models_response(
    response: Result<ureq::Response, ureq::Error>,
) -> Result<Vec<String>, String> {
    let text = match response {
        Ok(resp) => resp
            .into_string()
            .map_err(|err| format!("Model list response failed: {err}"))?,
        Err(ureq::Error::Status(status, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            return Err(format!(
                "Provider returned HTTP {status}: {}",
                http::shorten_for_error(&body)
            ));
        }
        Err(err) => return Err(format!("Model list request failed: {err}")),
    };

    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("Model list JSON was invalid: {err}"))?;
    let mut models: Vec<String> = value
        .get("data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    models.retain(|model| !model.trim().is_empty());
    if models.is_empty() {
        return Err("Provider returned no models.".to_string());
    }
    Ok(models)
}

fn handle_proxy_client(mut stream: TcpStream, state: Arc<Mutex<AppState>>, agent: ureq::Agent) {
    let request = match read_http_request(&mut stream) {
        Ok(request) => request,
        Err(err) => {
            write_error_response(&mut stream, 400, &err);
            return;
        }
    };

    let path = request.path.split('?').next().unwrap_or(&request.path);
    if request.method == "GET" && path.ends_with("/models") {
        let profile = active_openai_profile(&state);
        if let Some(profile) = profile.as_ref()
            && !proxy_auth_authorized(&request, profile)
        {
            write_error_response(&mut stream, 401, PROXY_AUTH_ERROR_MESSAGE);
            return;
        }
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
        write_error_response(&mut stream, 405, "claudie proxy accepts POST /v1/messages");
        return;
    }

    if path.ends_with("/messages/count_tokens") {
        if let Some(profile) = active_openai_profile(&state)
            && !proxy_auth_authorized(&request, &profile)
        {
            write_error_response(&mut stream, 401, PROXY_AUTH_ERROR_MESSAGE);
            return;
        }
        handle_count_tokens(&mut stream, &state, &request.body);
        return;
    }

    if !path.ends_with("/messages") {
        write_error_response(
            &mut stream,
            404,
            "claudie proxy only implements /v1/messages",
        );
        return;
    }

    let Some(profile) = active_openai_profile(&state) else {
        write_error_response(
            &mut stream,
            503,
            "No active OpenAI chat/completions profile is configured in claudie.",
        );
        return;
    };

    if !proxy_auth_authorized(&request, &profile) {
        write_error_response(&mut stream, 401, PROXY_AUTH_ERROR_MESSAGE);
        return;
    }

    if profile.openai_upstream_api_key().is_empty() {
        write_error_response(
            &mut stream,
            400,
            "The active OpenAI proxy profile is missing an API key or auth token.",
        );
        return;
    }

    let anthropic_request: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(err) => {
            write_error_response(&mut stream, 400, &err.to_string());
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
            write_error_response(&mut stream, 400, &err);
            return;
        }
    };
    let openai_request = proxy_optimizer::optimize_openai_request(openai_request, &profile).request;

    let outbound_model = openai_request
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| profile.model.trim().to_string());
    let force_transcript_no_tools = !model_supports_tools(&outbound_model);
    let use_tool_transcript = force_transcript_no_tools
        || cached_tool_history_needs_transcript(&profile, &openai_request);
    // Borrow in the common case; only the transcript fallback needs an owned copy.
    let outbound_request: Cow<Value> = if use_tool_transcript {
        Cow::Owned(tool_history_as_text_transcript(&openai_request))
    } else {
        Cow::Borrowed(&openai_request)
    };

    if wants_stream {
        run_streaming_request(
            &mut stream,
            &state,
            &agent,
            &profile,
            &anthropic_request,
            &openai_request,
            &outbound_request,
            use_tool_transcript,
        );
        return;
    }

    let upstream = match call_openai(&agent, &profile, &outbound_request) {
        Ok(value) => value,
        Err(err)
            if !use_tool_transcript && should_retry_with_tool_transcript(&err, &openai_request) =>
        {
            let _ = save_tool_history_capability(&profile, &openai_request, false);
            let fallback_request = tool_history_as_text_transcript(&openai_request);
            match call_openai(&agent, &profile, &fallback_request) {
                Ok(value) => value,
                Err(retry_err) => {
                    let combined = format!("{err}; text tool transcript retry failed: {retry_err}");
                    write_upstream_error(&mut stream, &state, &retry_err, combined);
                    return;
                }
            }
        }
        Err(err) => {
            let message = err.to_string();
            write_upstream_error(&mut stream, &state, &err, message);
            return;
        }
    };

    let anthropic_response = openai_to_anthropic_response(&upstream, &anthropic_request, &profile);
    let _ = write_json_response(&mut stream, 200, anthropic_response);
}

fn active_openai_profile(state: &Arc<Mutex<AppState>>) -> Option<LlmProfile> {
    let state = state.lock().expect("state poisoned");
    state
        .llm_profiles
        .active_profile()
        .filter(|profile| profile.is_openai_chat_proxy())
        .cloned()
}

fn proxy_auth_authorized(request: &HttpRequest, profile: &LlmProfile) -> bool {
    let expected = proxy_auth_token(profile);
    request_auth_candidates(request).any(|candidate| constant_time_eq(candidate, &expected))
}

fn proxy_auth_token(profile: &LlmProfile) -> String {
    let token = profile.auth_token.trim();
    if token.is_empty() {
        "claudie-openai-proxy".to_string()
    } else {
        token.to_string()
    }
}

fn request_auth_candidates(request: &HttpRequest) -> impl Iterator<Item = &str> {
    [
        request.header("authorization").and_then(bearer_token),
        request.header("x-api-key").map(str::trim),
        request.header("anthropic-api-key").map(str::trim),
        request.header("api-key").map(str::trim),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.is_empty())
}

fn bearer_token(value: &str) -> Option<&str> {
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    parts
        .next()
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

fn constant_time_eq(candidate: &str, expected: &str) -> bool {
    let candidate = candidate.as_bytes();
    let expected = expected.as_bytes();
    if candidate.len() != expected.len() {
        return false;
    }
    candidate
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

const PROXY_AUTH_ERROR_MESSAGE: &str = "Unauthorized claudie proxy request.";

/// Anthropic error envelope. Claude Code's SDK parses this exact shape for
/// error display and retry decisions; plain `{"error": "..."}` bodies render
/// as opaque failures.
pub(super) fn anthropic_error_body(error_type: &str, message: &str) -> Value {
    json!({
        "type": "error",
        "error": { "type": error_type, "message": message }
    })
}

fn anthropic_error_type_for_status(status: u16) -> &'static str {
    match status {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 | 405 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        503 | 529 => "overloaded_error",
        _ => "api_error",
    }
}

/// Fold arbitrary OpenAI-style `error.type` strings onto the closest of
/// Anthropic's native error types, which are the only ones Claude Code knows.
pub(super) fn anthropic_error_type_for_upstream(raw: &str) -> &'static str {
    let lowered = raw.to_ascii_lowercase();
    if lowered.contains("rate_limit") || lowered.contains("insufficient_quota") {
        "rate_limit_error"
    } else if lowered.contains("overloaded") {
        "overloaded_error"
    } else if lowered.contains("authentication") {
        "authentication_error"
    } else if lowered.contains("permission") {
        "permission_error"
    } else if lowered.contains("invalid_request") {
        "invalid_request_error"
    } else {
        "api_error"
    }
}

fn write_error_response(stream: &mut TcpStream, status: u16, message: &str) {
    let _ = write_json_response(
        stream,
        status,
        anthropic_error_body(anthropic_error_type_for_status(status), message),
    );
}

fn handle_count_tokens(stream: &mut TcpStream, state: &Arc<Mutex<AppState>>, body: &[u8]) {
    let input: Value = serde_json::from_slice(body).unwrap_or_else(|_| json!({}));
    // Estimate over the text that actually reaches the prompt (messages + tool
    // schemas) instead of the whole serialized JSON, so Claude Code's context
    // meter and auto-compact timing track reality. Fall back to the coarse
    // whole-blob estimate when no OpenAI profile is active.
    let estimate = match active_openai_profile(state) {
        Some(profile) => request_conv::estimate_request_input_tokens(&input, &profile),
        None => (input.to_string().chars().count() / 4).max(1) as u64,
    };
    let _ = write_json_response(stream, 200, json!({ "input_tokens": estimate }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_header(name: &str, value: &str) -> HttpRequest {
        HttpRequest {
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            headers: vec![(name.to_string(), value.to_string())],
            body: Vec::new(),
        }
    }

    #[test]
    fn anthropic_candidates_strip_compat_subpath() {
        // A relay that nests the Anthropic API under `/anthropic` also gets its
        // host-root OpenAI-style model list tried.
        assert_eq!(
            anthropic_model_candidates("https://token-plan-cn.xiaomimimo.com/anthropic"),
            vec![
                "https://token-plan-cn.xiaomimimo.com/anthropic/v1/models".to_string(),
                "https://token-plan-cn.xiaomimimo.com/v1/models".to_string(),
                "https://token-plan-cn.xiaomimimo.com/models".to_string(),
            ]
        );

        // The longest matching suffix wins (`/api/anthropic`, not `/anthropic`).
        assert_eq!(
            anthropic_model_candidates("https://api.z.ai/api/anthropic"),
            vec![
                "https://api.z.ai/api/anthropic/v1/models".to_string(),
                "https://api.z.ai/v1/models".to_string(),
                "https://api.z.ai/models".to_string(),
            ]
        );

        // The plain official base only probes `/v1/models`.
        assert_eq!(
            anthropic_model_candidates(""),
            vec!["https://api.anthropic.com/v1/models".to_string()]
        );
    }

    #[test]
    fn anthropic_candidates_handle_version_segments() {
        // A base ending in a version segment takes `/models`, not `/v1/models`.
        assert_eq!(
            anthropic_model_candidates("https://open.bigmodel.cn/api/coding/paas/v4"),
            vec![
                "https://open.bigmodel.cn/api/coding/paas/v4/models".to_string(),
                "https://open.bigmodel.cn/api/coding/paas/v4/v1/models".to_string(),
            ]
        );
        assert_eq!(
            anthropic_model_candidates("https://api.example.com/v1"),
            vec!["https://api.example.com/v1/models".to_string()]
        );
    }

    #[test]
    fn proxy_auth_accepts_bearer_token() {
        let profile = LlmProfile {
            auth_token: "local-token".to_string(),
            ..LlmProfile::default()
        };
        let request = request_with_header("Authorization", "BEARER local-token");
        assert!(proxy_auth_authorized(&request, &profile));
    }

    #[test]
    fn proxy_auth_accepts_default_token_when_profile_token_empty() {
        let profile = LlmProfile::default();
        let request = request_with_header("x-api-key", "claudie-openai-proxy");
        assert!(proxy_auth_authorized(&request, &profile));
    }

    #[test]
    fn proxy_auth_rejects_wrong_token() {
        let profile = LlmProfile {
            auth_token: "local-token".to_string(),
            ..LlmProfile::default()
        };
        let request = request_with_header("Authorization", "Bearer wrong");
        assert!(!proxy_auth_authorized(&request, &profile));
    }

    #[test]
    fn error_body_uses_anthropic_envelope() {
        let body = anthropic_error_body(anthropic_error_type_for_status(401), "denied");
        assert_eq!(body["type"], "error");
        assert_eq!(body["error"]["type"], "authentication_error");
        assert_eq!(body["error"]["message"], "denied");
    }

    #[test]
    fn error_types_map_from_status_codes() {
        assert_eq!(
            anthropic_error_type_for_status(400),
            "invalid_request_error"
        );
        assert_eq!(anthropic_error_type_for_status(404), "not_found_error");
        assert_eq!(anthropic_error_type_for_status(405), "not_found_error");
        assert_eq!(anthropic_error_type_for_status(429), "rate_limit_error");
        assert_eq!(anthropic_error_type_for_status(502), "api_error");
        assert_eq!(anthropic_error_type_for_status(503), "overloaded_error");
        assert_eq!(anthropic_error_type_for_status(529), "overloaded_error");
    }

    #[test]
    fn upstream_error_types_fold_onto_native_set() {
        assert_eq!(
            anthropic_error_type_for_upstream("insufficient_quota"),
            "rate_limit_error"
        );
        assert_eq!(
            anthropic_error_type_for_upstream("rate_limit_exceeded"),
            "rate_limit_error"
        );
        assert_eq!(
            anthropic_error_type_for_upstream("invalid_request_error"),
            "invalid_request_error"
        );
        assert_eq!(
            anthropic_error_type_for_upstream("server_error"),
            "api_error"
        );
    }
}

pub(super) fn record_proxy_error(state: &Arc<Mutex<AppState>>, err: String) {
    let mut state = state.lock().expect("state poisoned");
    state.last_error = err;
    state.set_mood(PetMood::Error);
}

/// Record an upstream failure and report it to the client with the proxy
/// status mapped from the upstream error.
fn write_upstream_error(
    stream: &mut TcpStream,
    state: &Arc<Mutex<AppState>>,
    err: &UpstreamError,
    message: String,
) {
    record_proxy_error(state, message.clone());
    let status = proxy_status_for_upstream_error(err);
    let body = anthropic_error_body(anthropic_error_type_for_status(status), &message);
    let mut headers: Vec<(&str, &str)> = Vec::new();
    if let Some(retry_after) = err.retry_after.as_deref()
        && matches!(status, 429 | 529)
    {
        headers.push(("Retry-After", retry_after));
    }
    let _ = write_json_response_with_headers(stream, status, &headers, body);
}
