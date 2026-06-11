use std::io::{BufReader, Read};

use serde_json::{Map, Value};

use crate::settings::LlmProfile;

use super::http::shorten_for_error;

#[derive(Clone, Debug)]
pub(super) struct UpstreamError {
    pub(super) status: Option<u16>,
    pub(super) message: String,
    /// Verbatim `retry-after` header from the upstream, forwarded to Claude
    /// Code on 429/529 so its backoff matches the upstream's schedule.
    pub(super) retry_after: Option<String>,
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(super) fn call_openai(
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
            retry_after: None,
        })?,
        Err(ureq::Error::Status(status, response)) => {
            let retry_after = response.header("retry-after").map(str::to_string);
            let text = response.into_string().unwrap_or_default();
            return Err(UpstreamError {
                status: Some(status),
                message: format!(
                    "OpenAI proxy upstream returned HTTP {status}: {}",
                    shorten_for_error(&text)
                ),
                retry_after,
            });
        }
        Err(err) => {
            return Err(UpstreamError {
                status: None,
                message: format!("OpenAI proxy upstream request failed: {err}"),
                retry_after: None,
            });
        }
    };

    serde_json::from_str(&text).map_err(|err| UpstreamError {
        status: None,
        message: format!("OpenAI proxy upstream returned invalid JSON: {err}"),
        retry_after: None,
    })
}

pub(super) fn call_openai_streaming(
    agent: &ureq::Agent,
    profile: &LlmProfile,
    body: &Value,
) -> Result<BufReader<Box<dyn Read + Send + Sync + 'static>>, UpstreamError> {
    let auth = format!("Bearer {}", profile.openai_upstream_api_key());
    let mut body = body.clone();
    if let Some(map) = body.as_object_mut() {
        map.insert("stream".to_string(), Value::Bool(true));
        let stream_options = map
            .entry("stream_options".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(opt_map) = stream_options.as_object_mut() {
            opt_map
                .entry("include_usage".to_string())
                .or_insert(Value::Bool(true));
        }
    }
    let response = agent
        .post(&profile.openai_chat_completions_url())
        .set("Authorization", &auth)
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .send_string(&body.to_string());

    match response {
        Ok(resp) => Ok(BufReader::new(resp.into_reader())),
        Err(ureq::Error::Status(status, resp)) => {
            let retry_after = resp.header("retry-after").map(str::to_string);
            let text = resp.into_string().unwrap_or_default();
            Err(UpstreamError {
                status: Some(status),
                message: format!(
                    "OpenAI proxy upstream returned HTTP {status}: {}",
                    shorten_for_error(&text)
                ),
                retry_after,
            })
        }
        Err(err) => Err(UpstreamError {
            status: None,
            message: format!("OpenAI proxy upstream request failed: {err}"),
            retry_after: None,
        }),
    }
}

pub(super) fn proxy_status_for_upstream_error(err: &UpstreamError) -> u16 {
    if upstream_error_is_temporarily_unavailable(err) {
        // 529 is Anthropic's native "overloaded" status; Claude Code's SDK
        // retries it with backoff just like 429/5xx.
        return 529;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_4xx_status_is_preserved_for_client() {
        let err = UpstreamError {
            status: Some(400),
            message: "bad request".to_string(),
            retry_after: None,
        };
        assert_eq!(proxy_status_for_upstream_error(&err), 400);

        let err = UpstreamError {
            status: Some(503),
            message: "unavailable".to_string(),
            retry_after: None,
        };
        assert_eq!(proxy_status_for_upstream_error(&err), 502);
    }

    #[test]
    fn temporary_engine_4xx_maps_to_retryable_529() {
        let err = UpstreamError {
            status: Some(400),
            message: r#"OpenAI proxy upstream returned HTTP 400: {"error":{"message":"engine is not available temporarily","type":"failed_precondition_error","code":"9"}}"#
                .to_string(),
            retry_after: None,
        };

        assert_eq!(proxy_status_for_upstream_error(&err), 529);
    }
}
