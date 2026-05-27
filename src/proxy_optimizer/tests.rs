use serde_json::{Value, json};
use std::fs;
use std::time::SystemTime;

use crate::settings::LlmProfile;
use crate::settings::storage::save_pretty_json;

use super::cache::{SummaryCacheFile, chunk_summary_cache_key, summary_cache_key};
use super::config::{
    DEFAULT_MAX_OUTPUT_TOKENS, DEFAULT_TOOL_RESULT_LIMIT_TOKENS, ProxyOptimizationConfig,
};
use super::summary::{local_summary_from_messages, local_summary_with_chunk_cache_at};
use super::{
    CHARS_PER_TOKEN, OPTIMIZER_VERSION, avoid_leading_tool_messages, now_millis,
    optimize_openai_request, prune_cache_dir, summary_text_from_openai_response,
};

fn profile() -> LlmProfile {
    profile_with_id("test-profile")
}

fn profile_with_id(id: &str) -> LlmProfile {
    LlmProfile {
        id: id.to_string(),
        model: "gpt-test".to_string(),
        ..LlmProfile::default()
    }
}

fn request_with_messages(messages: Vec<Value>) -> Value {
    json!({
        "model": "gpt-test",
        "messages": messages,
        "stream": false
    })
}

fn estimate_tokens(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count().div_ceil(CHARS_PER_TOKEN),
        _ => value.to_string().chars().count().div_ceil(CHARS_PER_TOKEN),
    }
}

#[test]
fn below_threshold_request_does_not_summarize() {
    let request = request_with_messages(vec![json!({ "role": "user", "content": "hello" })]);

    let optimized = optimize_openai_request(request.clone(), &profile());

    assert!(optimized.pending_summary.is_none());
    assert!(!optimized.cache_hit);
    assert_eq!(optimized.request, request);
}

#[test]
fn disabled_optimizer_leaves_request_unchanged() {
    let mut profile = profile();
    profile.extra_env = "CLAUDIE_PROXY_OPTIMIZE=0".to_string();
    let request = request_with_messages(vec![json!({
        "role": "tool",
        "tool_call_id": "call_1",
        "content": "x".repeat(DEFAULT_TOOL_RESULT_LIMIT_TOKENS * CHARS_PER_TOKEN + 50)
    })]);

    let optimized = optimize_openai_request(request.clone(), &profile);

    assert_eq!(optimized.request, request);
    assert!(optimized.pending_summary.is_none());
    assert!(!optimized.compressed);
}

#[test]
fn long_tool_result_is_head_tail_compressed() {
    let text = format!(
        "{}{}{}",
        "start-",
        "x".repeat(DEFAULT_TOOL_RESULT_LIMIT_TOKENS * CHARS_PER_TOKEN + 500),
        "-end"
    );
    let request = request_with_messages(vec![json!({
        "role": "tool",
        "tool_call_id": "call_1",
        "content": text
    })]);

    let optimized = optimize_openai_request(request, &profile());
    let content = optimized.request["messages"][0]["content"]
        .as_str()
        .unwrap();

    assert!(optimized.compressed);
    assert!(content.starts_with("start-"));
    assert!(content.ends_with("-end"));
    assert!(content.contains("claudie proxy omitted"));
}

#[test]
fn over_threshold_uses_local_summary_by_default_and_keeps_recent_messages() {
    let profile = profile_with_id(&format!("local-summary-test-{}", now_millis()));
    let messages = (0..30)
        .map(|index| {
            json!({
                "role": if index % 2 == 0 { "user" } else { "assistant" },
                "content": format!("message-{index}-{}", "x".repeat(8_000))
            })
        })
        .collect::<Vec<_>>();
    let request = request_with_messages(messages);

    let optimized = optimize_openai_request(request, &profile);
    let output_messages = optimized.request["messages"].as_array().unwrap();

    assert!(optimized.local_summary);
    assert!(optimized.pending_summary.is_none());
    assert!(output_messages.iter().any(|message| {
        message["content"]
            .as_str()
            .unwrap_or("")
            .contains("Local extractive summary")
    }));
    assert!(output_messages.iter().any(|message| {
        message["content"]
            .as_str()
            .unwrap_or("")
            .contains("message-29")
    }));
}

#[test]
fn model_summary_mode_creates_pending_summary() {
    let mut profile = profile();
    profile.extra_env = "CLAUDIE_PROXY_SUMMARY_MODE=model".to_string();
    let messages = (0..30)
        .map(|index| {
            json!({
                "role": if index % 2 == 0 { "user" } else { "assistant" },
                "content": format!("message-{index}-{}", "x".repeat(8_000))
            })
        })
        .collect::<Vec<_>>();
    let request = request_with_messages(messages);

    let optimized = optimize_openai_request(request, &profile);

    assert!(!optimized.local_summary);
    assert!(optimized.pending_summary.is_some());
}

#[test]
fn local_summary_stays_within_budget() {
    let messages = (0..20)
        .map(|index| {
            json!({
                "role": "tool",
                "tool_call_id": format!("call_{index}"),
                "content": format!("tool-result-{index}-{}", "x".repeat(20_000))
            })
        })
        .collect::<Vec<_>>();

    let summary = local_summary_from_messages(&messages, 1_000);

    assert!(estimate_tokens(&Value::String(summary)) <= 1_300);
}

#[test]
fn local_summary_preserves_original_user_goal() {
    let mut messages = vec![json!({
        "role": "user",
        "content": "Please optimize README and AGENTS, fill missing parts, and fix inaccurate parts."
    })];
    messages.extend((0..30).map(|index| {
        json!({
            "role": "tool",
            "tool_call_id": format!("call_{index}"),
            "content": format!("tool-result-{index}-{}", "x".repeat(20_000))
        })
    }));

    let summary = local_summary_from_messages(&messages, 250);

    assert!(summary.contains("original user request"));
    assert!(summary.contains("optimize README and AGENTS"));
}

#[test]
fn output_token_budget_is_capped_by_default() {
    let request = json!({
        "model": "gpt-test",
        "messages": [{ "role": "user", "content": "hello" }],
        "max_tokens": 100_000_u64,
        "max_completion_tokens": 100_000_u64
    });

    let optimized = optimize_openai_request(request, &profile());

    assert_eq!(optimized.request["max_tokens"], DEFAULT_MAX_OUTPUT_TOKENS);
    assert_eq!(
        optimized.request["max_completion_tokens"],
        DEFAULT_MAX_OUTPUT_TOKENS
    );
    assert!(optimized.compressed);
}

#[test]
fn output_token_cap_can_be_disabled() {
    let mut profile = profile();
    profile.extra_env = "CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=0".to_string();
    let request = json!({
        "model": "gpt-test",
        "messages": [{ "role": "user", "content": "hello" }],
        "max_tokens": 100_000_u64
    });

    let optimized = optimize_openai_request(request, &profile);

    assert_eq!(optimized.request["max_tokens"], 100_000_u64);
    assert!(!optimized.compressed);
}

#[test]
fn cache_key_is_stable_and_changes_with_tools() {
    let config = ProxyOptimizationConfig::default();
    let old = vec![json!({ "role": "user", "content": "same old history" })];
    let request_a = json!({
        "model": "gpt-test",
        "tools": [{ "type": "function", "function": { "name": "Read" } }]
    });
    let request_b = json!({
        "model": "gpt-test",
        "tools": [{ "type": "function", "function": { "name": "Write" } }]
    });

    let key_a1 = summary_cache_key(&profile(), &request_a, &old, &config);
    let key_a2 = summary_cache_key(&profile(), &request_a, &old, &config);
    let key_b = summary_cache_key(&profile(), &request_b, &old, &config);

    assert_eq!(key_a1, key_a2);
    assert_ne!(key_a1, key_b);
}

#[test]
fn cache_dir_prune_removes_expired_and_over_limit_files() {
    let dir = std::env::temp_dir().join(format!("claudie-cache-prune-{}", now_millis()));
    let payload = SummaryCacheFile {
        version: OPTIMIZER_VERSION.to_string(),
        kind: "summary".to_string(),
        summary: "payload".to_string(),
        created_at_ms: now_millis(),
        last_used_at_ms: now_millis(),
    };
    let old_path = dir.join("old.json");
    let fresh_a_path = dir.join("fresh-a.json");
    let fresh_b_path = dir.join("fresh-b.json");
    save_pretty_json(&old_path, &payload).unwrap();
    save_pretty_json(&fresh_a_path, &payload).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    save_pretty_json(&fresh_b_path, &payload).unwrap();
    // Force old_path's mtime to the unix epoch so the TTL check trips it.
    let past = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1);
    fs::OpenOptions::new()
        .write(true)
        .open(&old_path)
        .unwrap()
        .set_modified(past)
        .unwrap();

    prune_cache_dir(&dir, 1, 1, 1024 * 1024).unwrap();

    assert!(!old_path.exists());
    assert!(!fresh_a_path.exists());
    assert!(fresh_b_path.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn chunk_summary_cache_reuses_existing_chunk_file() {
    let dir = std::env::temp_dir().join(format!("claudie-chunk-cache-{}", now_millis()));
    let config = ProxyOptimizationConfig {
        chunk_size_messages: 2,
        local_summary_tokens: 2_000,
        ..ProxyOptimizationConfig::default()
    };
    let request = json!({
        "model": "gpt-test",
        "tools": [{ "type": "function", "function": { "name": "Read" } }]
    });
    let messages = vec![
        json!({ "role": "user", "content": "original user request: optimize proxy cache" }),
        json!({ "role": "assistant", "content": "I will inspect the cache" }),
        json!({ "role": "tool", "content": "cache file contents" }),
        json!({ "role": "assistant", "content": "I found a large JSON file" }),
        json!({ "role": "user", "content": "continue" }),
    ];
    let first_chunk_key = chunk_summary_cache_key(&profile(), &request, &messages[..2], &config);
    let first_chunk_path = dir.join(format!("{first_chunk_key}.json"));
    let cached_payload = SummaryCacheFile {
        version: OPTIMIZER_VERSION.to_string(),
        kind: "chunk_summary".to_string(),
        summary: "cached first chunk marker".to_string(),
        created_at_ms: now_millis(),
        last_used_at_ms: now_millis(),
    };
    save_pretty_json(&first_chunk_path, &cached_payload).unwrap();

    let summary = local_summary_with_chunk_cache_at(&profile(), &request, &messages, &config, &dir);

    assert!(summary.contains("Chunked local summary"));
    assert!(summary.contains("cached first chunk marker"));
    assert!(dir.read_dir().unwrap().count() >= 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn chunk_summary_can_be_disabled_from_profile_env() {
    let mut profile = profile();
    profile.extra_env = "CLAUDIE_PROXY_CHUNK_SUMMARY=0".to_string();

    let config = ProxyOptimizationConfig::from_profile(&profile);

    assert!(!config.chunk_summary_enabled);
}

#[test]
fn summary_text_extracts_openai_message_content() {
    let response = json!({
        "choices": [{ "message": { "content": " summary text " } }]
    });

    assert_eq!(
        summary_text_from_openai_response(&response).as_deref(),
        Some("summary text")
    );
}

#[test]
fn leading_tool_messages_pull_in_previous_assistant() {
    let messages = vec![
        json!({ "role": "user", "content": "old" }),
        json!({ "role": "assistant", "content": null, "tool_calls": [{ "id": "c1" }] }),
        json!({ "role": "tool", "tool_call_id": "c1", "content": "result" }),
        json!({ "role": "user", "content": "next" }),
    ];

    assert_eq!(avoid_leading_tool_messages(&messages, 0, 2), 1);
}
