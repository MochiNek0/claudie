use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::app::stats::tool_stats_kind;
use crate::app::{
    AppState, ChoiceDecision, ChoiceKind, ChoiceOption, ChoiceQuestion, ChoiceWaiter,
    ClaudeSessionStatus, PendingChoice, PendingPermission, PermissionDecision, PermissionWaiter,
    PetMood,
};
use crate::globals::APP_STATE;
use crate::util::{diff_lines_text, shorten, shorten_block};

use super::quota::{scan_transcript_usage, update_quota_from_value};

const PAYLOAD_SUMMARY_MAX_CHARS: usize = 2_000;
const WRITE_CONTENT_MAX_LINES: usize = 80;
const MAX_MULTIEDIT_EDITS: usize = 6;
const TRANSCRIPT_DENIAL_POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HookActivity {
    Idle,
    Thinking,
    StartTool(PetMood),
    FinishTool(PetMood),
    FinishToolBatch,
    Error,
    PermissionDenied,
    StartSubagent,
    FinishSubagent,
    Done,
    EndSession,
}

enum PermissionResponseKind {
    PermissionRequest,
    PreToolUse { tool_input: Value },
}

pub(crate) fn process_hook(payload: Value, state: Arc<Mutex<AppState>>) -> Value {
    let event = string_field(&payload, "hook_event_name")
        .or_else(|| string_field(&payload, "hookEventName"))
        .unwrap_or_else(|| "unknown".to_string());
    let session_id = session_id_from_payload(&payload);
    let cwd = string_field(&payload, "cwd").unwrap_or_default();
    let tool_name = string_field(&payload, "tool_name")
        .or_else(|| string_field(&payload, "toolName"))
        .unwrap_or_default();

    if event == "PermissionRequest" {
        return handle_permission_request(
            payload,
            state,
            session_id,
            cwd,
            tool_name,
            PermissionResponseKind::PermissionRequest,
        );
    }
    if event == "PreToolUse" {
        if let Some(response) =
            handle_interactive_pre_tool_use(&payload, state.clone(), &session_id, &cwd, &tool_name)
        {
            return response;
        }
    }

    let transcript_path = string_field(&payload, "transcript_path")
        .or_else(|| string_field(&payload, "transcriptPath"));

    {
        let mut state = state.lock().expect("state poisoned");
        state.note_session_event(
            &session_id,
            &cwd,
            event.as_str(),
            &tool_name,
            mood_for_tool_use(&tool_name, &payload),
        );
        if clears_pending_interaction(event.as_str()) {
            clear_stale_interactions(&mut state, &session_id);
        }
        let capture_official = state.llm_profiles.official_profile_active();
        update_quota_from_value(&mut state.quota, &payload, capture_official);
        if let Some(path) = transcript_path.as_deref() {
            state.quota.transcript_path = path.to_string();
        }
        record_daily_stats(&mut state, event.as_str(), &tool_name);
        state.record_token_snapshot();

        if let Some(activity) = hook_activity(event.as_str(), &tool_name, &payload) {
            apply_hook_activity(&mut state, activity, &payload);
        }

        state.last_activity = Instant::now();
    }

    if matches!(
        transcript_path.as_deref(),
        Some(path) if event == "Stop" || event == "SessionEnd" || event == "PostToolBatch"
    ) {
        if let Some(snapshot) = scan_transcript_usage(Path::new(transcript_path.as_ref().unwrap()))
        {
            let mut state = state.lock().expect("state poisoned");
            state.quota.observed_total_tokens = snapshot.observed_total_tokens;
            if snapshot.input_tokens > 0 {
                state.quota.input_tokens = snapshot.input_tokens;
            }
            if snapshot.output_tokens > 0 {
                state.quota.output_tokens = snapshot.output_tokens;
            }
            if snapshot.cache_creation_tokens > 0 {
                state.quota.cache_creation_tokens = snapshot.cache_creation_tokens;
            }
            if snapshot.cache_read_tokens > 0 {
                state.quota.cache_read_tokens = snapshot.cache_read_tokens;
            }
            if !snapshot.provider.is_empty() {
                state.quota.provider = snapshot.provider;
            }
            if !snapshot.quota_remaining.is_empty() {
                state.quota.quota_remaining = snapshot.quota_remaining;
            }
            if !snapshot.quota_limit.is_empty() {
                state.quota.quota_limit = snapshot.quota_limit;
            }
            if !snapshot.quota_reset.is_empty() {
                state.quota.quota_reset = snapshot.quota_reset;
            }
            if !snapshot.last_model.is_empty() {
                state.quota.last_model = snapshot.last_model;
            }
            if !snapshot.rate_limits.is_empty() {
                state.quota.rate_limits = snapshot.rate_limits;
            }
            state.record_token_snapshot();
        }
    }

    json!({})
}

fn handle_permission_request(
    payload: Value,
    state: Arc<Mutex<AppState>>,
    session_id: String,
    cwd: String,
    tool_name: String,
    response_kind: PermissionResponseKind,
) -> Value {
    let transcript_path = string_field(&payload, "transcript_path")
        .or_else(|| string_field(&payload, "transcriptPath"));
    let transcript_start = transcript_path
        .as_deref()
        .and_then(transcript_len)
        .unwrap_or(0);

    let waiter = Arc::new(PermissionWaiter {
        decision: Mutex::new(None),
        ready: Condvar::new(),
    });

    let suggestions = payload
        .get("permission_suggestions")
        .or_else(|| payload.get("permissionSuggestions"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let permission = {
        let mut state = state.lock().expect("state poisoned");
        let id = state.next_permission_id;
        state.next_permission_id += 1;
        let interaction_sequence = state.next_interaction_sequence;
        state.next_interaction_sequence += 1;
        let permission = PendingPermission {
            id,
            interaction_sequence,
            session_id,
            tool_name: if tool_name.is_empty() {
                "tool".to_string()
            } else {
                tool_name
            },
            summary: summarize_payload(&payload),
            cwd,
            suggestions,
            waiter: waiter.clone(),
        };
        state.pending_permissions.push_back(permission.clone());
        let mood = permission_visual_mood(&payload, &permission.tool_name);
        state.mark_session_waiting_permission(
            &permission.session_id,
            &permission.cwd,
            &permission.tool_name,
            permission.interaction_sequence,
        );
        state.start_permission_activity(permission.id, &permission.session_id, mood);
        state.set_resting_mood(mood, true);
        state.record_permission_stats();
        let capture_official = state.llm_profiles.official_profile_active();
        update_quota_from_value(&mut state.quota, &payload, capture_official);
        state.record_token_snapshot();
        permission
    };

    let decision = {
        let mut guard = waiter.decision.lock().expect("permission waiter poisoned");
        loop {
            if guard.is_some() {
                break *guard;
            }
            if transcript_path
                .as_deref()
                .is_some_and(|path| transcript_has_terminal_denial(path, transcript_start))
            {
                break Some(PermissionDecision::Ignore);
            }
            let (next_guard, _) = waiter
                .ready
                .wait_timeout(guard, TRANSCRIPT_DENIAL_POLL)
                .expect("permission waiter poisoned");
            guard = next_guard;
        }
    };

    {
        let mut state = state.lock().expect("state poisoned");
        state.finish_permission_activity(permission.id);
        state
            .pending_permissions
            .retain(|pending| pending.id != permission.id);
        if state.pending_permissions.is_empty() && state.pending_choices.is_empty() {
            let mood = match decision {
                Some(PermissionDecision::Deny) => PetMood::Error,
                Some(PermissionDecision::Ignore) => state.activity_mood().unwrap_or(PetMood::Idle),
                _ => state.activity_mood().unwrap_or(PetMood::Happy),
            };
            state.set_resting_mood(mood, matches!(mood, PetMood::Error));
        }
        let (status, detail) = match decision {
            Some(PermissionDecision::Deny) => {
                (ClaudeSessionStatus::Error, "Permission denied".to_string())
            }
            Some(PermissionDecision::Ignore) | None => {
                (ClaudeSessionStatus::Done, "Permission closed".to_string())
            }
            _ => (ClaudeSessionStatus::Streaming, "Streaming".to_string()),
        };
        state.mark_session_interaction_finished(
            &permission.session_id,
            permission.interaction_sequence,
            status,
            detail,
        );
    }

    permission_response(
        decision.unwrap_or(PermissionDecision::Ignore),
        &permission,
        response_kind,
    )
}

fn permission_response(
    decision: PermissionDecision,
    permission: &PendingPermission,
    response_kind: PermissionResponseKind,
) -> Value {
    match response_kind {
        PermissionResponseKind::PermissionRequest => {
            permission_request_response(decision, permission)
        }
        PermissionResponseKind::PreToolUse { tool_input } => {
            pre_tool_permission_response(decision, tool_input)
        }
    }
}

fn permission_request_response(
    decision: PermissionDecision,
    permission: &PendingPermission,
) -> Value {
    match decision {
        PermissionDecision::AllowOnce => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "allow"
                }
            }
        }),
        PermissionDecision::AllowAlways => {
            let mut decision = json!({ "behavior": "allow" });
            if !permission.suggestions.is_empty() {
                if let Some(obj) = decision.as_object_mut() {
                    obj.insert(
                        "updatedPermissions".to_string(),
                        Value::Array(permission.suggestions.clone()),
                    );
                }
            }
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": decision
                }
            })
        }
        PermissionDecision::Deny => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "deny",
                    "interrupt": false
                }
            }
        }),
        PermissionDecision::Ignore => json!({}),
    }
}

fn pre_tool_permission_response(decision: PermissionDecision, tool_input: Value) -> Value {
    match decision {
        PermissionDecision::AllowOnce | PermissionDecision::AllowAlways => {
            pre_tool_allow(tool_input, "Approved in claudie")
        }
        PermissionDecision::Deny => pre_tool_deny("Denied in claudie"),
        PermissionDecision::Ignore => json!({}),
    }
}

fn handle_interactive_pre_tool_use(
    payload: &Value,
    state: Arc<Mutex<AppState>>,
    session_id: &str,
    cwd: &str,
    tool_name: &str,
) -> Option<Value> {
    let tool_input = payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))
        .cloned()
        .unwrap_or_else(|| json!({}));

    match tool_name {
        "AskUserQuestion" => Some(handle_choice_request(
            state,
            session_id.to_string(),
            ChoiceKind::AskUserQuestion,
            "Question from Claude".to_string(),
            String::new(),
            parse_ask_user_questions(&tool_input),
            tool_input,
        )),
        "ExitPlanMode" => Some(handle_choice_request(
            state,
            session_id.to_string(),
            ChoiceKind::ExitPlanMode,
            "Approve plan".to_string(),
            summarize_exit_plan(&tool_input),
            exit_plan_questions(&tool_input),
            tool_input,
        )),
        _ if is_web_search_tool(tool_name) => Some(handle_permission_request(
            payload.clone(),
            state,
            session_id.to_string(),
            cwd.to_string(),
            tool_name.to_string(),
            PermissionResponseKind::PreToolUse { tool_input },
        )),
        _ => None,
    }
}

fn handle_choice_request(
    state: Arc<Mutex<AppState>>,
    session_id: String,
    kind: ChoiceKind,
    title: String,
    detail: String,
    questions: Vec<ChoiceQuestion>,
    tool_input: Value,
) -> Value {
    if questions.is_empty() {
        return json!({});
    }

    let waiter = Arc::new(ChoiceWaiter {
        decision: Mutex::new(None),
        ready: Condvar::new(),
    });

    let choice = {
        let mut state = state.lock().expect("state poisoned");
        let id = state.next_choice_id;
        state.next_choice_id += 1;
        let interaction_sequence = state.next_interaction_sequence;
        state.next_interaction_sequence += 1;
        let selected = vec![Vec::new(); questions.len()];
        let other_text = vec![String::new(); questions.len()];
        let choice = PendingChoice {
            id,
            interaction_sequence,
            session_id,
            kind,
            title,
            detail,
            questions,
            selected,
            other_text,
            tool_input,
            waiter: waiter.clone(),
        };
        state.pending_choices.push_back(choice.clone());
        let resting = state.activity_mood().unwrap_or(PetMood::Thinking);
        state.mark_session_waiting_choice(
            &choice.session_id,
            &choice.title,
            choice.interaction_sequence,
        );
        state.start_choice_activity(choice.id, &choice.session_id, resting);
        state.set_resting_mood(resting, false);
        state.record_choice_stats();
        update_quota_from_value(&mut state.quota, &json!({}), false);
        state.record_token_snapshot();
        choice
    };

    let decision = {
        let mut guard = waiter.decision.lock().expect("choice waiter poisoned");
        while guard.is_none() {
            guard = waiter.ready.wait(guard).expect("choice waiter poisoned");
        }
        guard.clone()
    };

    {
        let mut state = state.lock().expect("state poisoned");
        state.finish_choice_activity(choice.id);
        state
            .pending_choices
            .retain(|pending| pending.id != choice.id);
        if state.pending_permissions.is_empty() && state.pending_choices.is_empty() {
            let mood = match decision {
                Some(ChoiceDecision::Submit { .. }) => {
                    state.activity_mood().unwrap_or(PetMood::Happy)
                }
                Some(ChoiceDecision::Deny) => state.activity_mood().unwrap_or(PetMood::Thinking),
                Some(ChoiceDecision::Ignore) | None => {
                    state.activity_mood().unwrap_or(PetMood::Idle)
                }
            };
            state.set_resting_mood(mood, false);
        }
        let (status, detail) = match decision {
            Some(ChoiceDecision::Submit { .. }) | Some(ChoiceDecision::Deny) => {
                (ClaudeSessionStatus::Streaming, "Streaming".to_string())
            }
            Some(ChoiceDecision::Ignore) | None => {
                (ClaudeSessionStatus::Done, "Choice closed".to_string())
            }
        };
        state.mark_session_interaction_finished(
            &choice.session_id,
            choice.interaction_sequence,
            status,
            detail,
        );
    }

    choice_response(&choice, decision.unwrap_or(ChoiceDecision::Ignore))
}

fn choice_response(choice: &PendingChoice, decision: ChoiceDecision) -> Value {
    match (choice.kind, decision) {
        (
            ChoiceKind::AskUserQuestion,
            ChoiceDecision::Submit {
                selected,
                other_text,
            },
        ) => {
            let mut updated_input = choice.tool_input.clone();
            if let Some(obj) = updated_input.as_object_mut() {
                let mut answers = serde_json::Map::new();
                for (question_index, question) in choice.questions.iter().enumerate() {
                    let parts: Vec<String> = selected
                        .get(question_index)
                        .into_iter()
                        .flat_map(|items| items.iter())
                        .filter_map(|option_index| {
                            let option = question.options.get(*option_index)?;
                            if option.is_other {
                                let text = other_text
                                    .get(question_index)
                                    .map(|s| s.trim())
                                    .unwrap_or("");
                                if text.is_empty() {
                                    None
                                } else {
                                    Some(text.to_string())
                                }
                            } else {
                                Some(option.label.clone())
                            }
                        })
                        .collect();
                    answers.insert(question.question.clone(), Value::String(parts.join(", ")));
                }
                obj.insert("answers".to_string(), Value::Object(answers));
            }
            pre_tool_allow(updated_input, "Answered in claudie")
        }
        (
            ChoiceKind::ExitPlanMode,
            ChoiceDecision::Submit {
                selected,
                other_text,
            },
        ) => {
            let first = selected.first();
            if first.is_some_and(|items| items.contains(&0)) {
                pre_tool_allow(choice.tool_input.clone(), "Plan approved in claudie")
            } else if first.is_some_and(|items| items.contains(&2)) {
                let feedback = other_text.first().map(|s| s.trim()).unwrap_or("");
                if feedback.is_empty() {
                    pre_tool_deny("User chose to keep planning in claudie")
                } else {
                    pre_tool_deny(&format!("Plan rejected: {feedback}"))
                }
            } else {
                pre_tool_deny("User chose to keep planning in claudie")
            }
        }
        (_, ChoiceDecision::Deny) => pre_tool_deny("User declined in claudie"),
        (_, ChoiceDecision::Ignore) => json!({}),
    }
}

fn pre_tool_allow(updated_input: Value, reason: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": reason,
            "updatedInput": updated_input
        }
    })
}

fn pre_tool_deny(reason: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason
        }
    })
}

fn parse_ask_user_questions(tool_input: &Value) -> Vec<ChoiceQuestion> {
    tool_input
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|questions| questions.iter())
        .filter_map(|question| {
            let text = string_field(question, "question")?;
            let mut options = question
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flat_map(|options| options.iter())
                .filter_map(|option| {
                    let label = string_field(option, "label")?;
                    let description = string_field(option, "description").unwrap_or_default();
                    Some(ChoiceOption {
                        label,
                        description,
                        is_other: false,
                    })
                })
                .collect::<Vec<_>>();
            if options.is_empty() {
                return None;
            }
            options.push(ChoiceOption {
                label: "Other...".to_string(),
                description: "Type a custom answer.".to_string(),
                is_other: true,
            });
            Some(ChoiceQuestion {
                header: string_field(question, "header").unwrap_or_default(),
                question: text,
                multi_select: question
                    .get("multiSelect")
                    .or_else(|| question.get("multi_select"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                options,
            })
        })
        .collect()
}

fn exit_plan_questions(_tool_input: &Value) -> Vec<ChoiceQuestion> {
    vec![ChoiceQuestion {
        header: "Plan mode".to_string(),
        question: "Choose how claudie should answer Claude Code.".to_string(),
        multi_select: false,
        options: vec![
            ChoiceOption {
                label: "Approve plan".to_string(),
                description: "Leave plan mode and start implementation.".to_string(),
                is_other: false,
            },
            ChoiceOption {
                label: "Keep planning".to_string(),
                description: "Reject for now and continue refining the plan.".to_string(),
                is_other: false,
            },
            ChoiceOption {
                label: "Other...".to_string(),
                description: "Give Claude specific feedback before deciding.".to_string(),
                is_other: true,
            },
        ],
    }]
}

fn summarize_exit_plan(tool_input: &Value) -> String {
    let plan = string_field(tool_input, "plan").unwrap_or_default();
    let path = string_field(tool_input, "planFilePath")
        .or_else(|| string_field(tool_input, "plan_file_path"))
        .unwrap_or_default();
    if plan.is_empty() {
        return path;
    }
    let plan_text = shorten_block(&plan, 16_000);
    let mut detail = if path.is_empty() {
        plan_text
    } else {
        format!("{plan_text}\n{path}")
    };
    if let Some(prompts) = tool_input.get("allowedPrompts").and_then(Value::as_array) {
        for prompt in prompts.iter().take(8) {
            let tool = string_field(prompt, "tool").unwrap_or_else(|| "Tool".to_string());
            let text = string_field(prompt, "prompt").unwrap_or_default();
            detail.push_str(&format!("\nAllow {tool}: {}", shorten(&text, 120)));
        }
    }
    detail
}

fn transcript_len(path: &str) -> Option<u64> {
    fs::metadata(Path::new(path))
        .ok()
        .map(|metadata| metadata.len())
}

fn transcript_has_terminal_denial(path: &str, start: u64) -> bool {
    let Ok(mut file) = fs::File::open(Path::new(path)) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    if metadata.len() <= start {
        return false;
    }
    let read_start = start.max(metadata.len().saturating_sub(256_000));
    if file.seek(SeekFrom::Start(read_start)).is_err() {
        return false;
    }

    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return false;
    }
    let text = text.to_ascii_lowercase();
    (text.contains("permission") && text.contains("denied"))
        || text.contains("user denied")
        || text.contains("denied by user")
}

pub(crate) fn decide_current_permission(decision: PermissionDecision) {
    if let Some(state) = APP_STATE.get() {
        let pending = {
            let mut state = state.lock().expect("state poisoned");
            let pending = state.current_pending_permission().map(|pending| pending.id);
            let pending = pending.and_then(|id| {
                state
                    .pending_permissions
                    .iter()
                    .position(|pending| pending.id == id)
                    .and_then(|index| state.pending_permissions.remove(index))
            });
            if let Some(pending) = pending.as_ref() {
                state.finish_permission_activity(pending.id);
            }
            pending
        };
        if let Some(pending) = pending {
            let mut slot = pending
                .waiter
                .decision
                .lock()
                .expect("permission waiter poisoned");
            if slot.is_none() {
                *slot = Some(decision);
                pending.waiter.ready.notify_all();
            }
        }
    }
}

pub(crate) fn toggle_current_choice_option(question_index: usize, option_index: usize) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        let Some(choice) = state.current_pending_choice_mut() else {
            return;
        };
        let Some(question) = choice.questions.get(question_index) else {
            return;
        };
        if option_index >= question.options.len() {
            return;
        }
        let selected = &mut choice.selected[question_index];
        if question.multi_select {
            if let Some(position) = selected.iter().position(|item| *item == option_index) {
                selected.remove(position);
            } else {
                selected.push(option_index);
            }
        } else {
            selected.clear();
            selected.push(option_index);
        }
    }
}

pub(crate) fn submit_current_choice() {
    if let Some(state) = APP_STATE.get() {
        let pending = {
            let mut state = state.lock().expect("state poisoned");
            let Some(choice) = state.current_pending_choice() else {
                return;
            };
            if !choice.is_submittable() {
                return;
            }
            let id = choice.id;
            let selected = choice.selected.clone();
            let other_text = choice.other_text.clone();
            state
                .pending_choices
                .iter()
                .position(|pending| pending.id == id)
                .and_then(|index| state.pending_choices.remove(index))
                .map(|pending| {
                    state.finish_choice_activity(pending.id);
                    (
                        pending,
                        ChoiceDecision::Submit {
                            selected,
                            other_text,
                        },
                    )
                })
        };
        if let Some((pending, decision)) = pending {
            let mut slot = pending
                .waiter
                .decision
                .lock()
                .expect("choice waiter poisoned");
            if slot.is_none() {
                *slot = Some(decision);
                pending.waiter.ready.notify_all();
            }
        }
    }
}

pub(crate) fn deny_current_choice() {
    decide_current_choice(ChoiceDecision::Deny);
}

fn decide_current_choice(decision: ChoiceDecision) {
    if let Some(state) = APP_STATE.get() {
        let pending = {
            let mut state = state.lock().expect("state poisoned");
            let pending = state.current_pending_choice().map(|pending| pending.id);
            let pending = pending.and_then(|id| {
                state
                    .pending_choices
                    .iter()
                    .position(|pending| pending.id == id)
                    .and_then(|index| state.pending_choices.remove(index))
            });
            if let Some(pending) = pending.as_ref() {
                state.finish_choice_activity(pending.id);
            }
            pending
        };
        if let Some(pending) = pending {
            let mut slot = pending
                .waiter
                .decision
                .lock()
                .expect("choice waiter poisoned");
            if slot.is_none() {
                *slot = Some(decision);
                pending.waiter.ready.notify_all();
            }
        }
    }
}

pub(crate) fn set_current_choice_other_text(question_index: usize, text: String) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        let Some(choice) = state.current_pending_choice_mut() else {
            return;
        };
        if let Some(slot) = choice.other_text.get_mut(question_index) {
            *slot = text;
        }
    }
}

fn clear_stale_interactions(state: &mut AppState, session_id: &str) {
    let mut stale = Vec::new();
    state.pending_permissions.retain(|pending| {
        if pending.session_id == session_id {
            stale.push(pending.clone());
            false
        } else {
            true
        }
    });

    for pending in stale {
        state.finish_permission_activity(pending.id);
        let mut slot = pending
            .waiter
            .decision
            .lock()
            .expect("permission waiter poisoned");
        if slot.is_none() {
            *slot = Some(PermissionDecision::Ignore);
            pending.waiter.ready.notify_all();
        }
    }

    let mut stale_choices = Vec::new();
    state.pending_choices.retain(|pending| {
        if pending.session_id == session_id {
            stale_choices.push(pending.clone());
            false
        } else {
            true
        }
    });

    for pending in stale_choices {
        state.finish_choice_activity(pending.id);
        let mut slot = pending
            .waiter
            .decision
            .lock()
            .expect("choice waiter poisoned");
        if slot.is_none() {
            *slot = Some(ChoiceDecision::Ignore);
            pending.waiter.ready.notify_all();
        }
    }
}

fn clears_pending_interaction(event: &str) -> bool {
    matches!(
        event,
        "PreToolUse"
            | "PostToolUse"
            | "PostToolBatch"
            | "PermissionDenied"
            | "PostToolUseFailure"
            | "Stop"
            | "SessionEnd"
            | "StopFailure"
    )
}

fn hook_activity(event: &str, tool_name: &str, payload: &Value) -> Option<HookActivity> {
    let tool_mood = mood_for_tool_use(tool_name, payload);
    match event {
        "SessionStart" | "SessionResume" => Some(HookActivity::Idle),
        "UserPromptSubmit" => Some(HookActivity::Thinking),
        "PreToolUse" => Some(HookActivity::StartTool(tool_mood)),
        "PostToolUse" => Some(HookActivity::FinishTool(tool_mood)),
        "PostToolBatch" => Some(HookActivity::FinishToolBatch),
        "PermissionDenied" => Some(HookActivity::PermissionDenied),
        "PostToolUseFailure" | "StopFailure" => Some(HookActivity::Error),
        "SubagentStart" | "TaskCreated" => Some(HookActivity::StartSubagent),
        "SubagentStop" | "TaskCompleted" => Some(HookActivity::FinishSubagent),
        "PreCompact" => Some(HookActivity::Thinking),
        "PostCompact" => Some(HookActivity::Done),
        "Notification" | "Elicitation" => None,
        "WorktreeCreate" => Some(HookActivity::StartTool(PetMood::Building)),
        "Stop" => Some(HookActivity::Done),
        "SessionEnd" => Some(HookActivity::EndSession),
        _ => None,
    }
}

fn apply_hook_activity(state: &mut AppState, activity: HookActivity, payload: &Value) {
    match activity {
        HookActivity::Idle => {
            state.set_resting_mood(PetMood::Idle, false);
        }
        HookActivity::Thinking => {
            state.set_resting_mood(PetMood::Thinking, true);
        }
        HookActivity::StartTool(mood) => {
            state.start_tool_activity(tool_key(payload), tool_name_from_payload(payload), mood);
        }
        HookActivity::FinishTool(mood) => {
            state.finish_tool_activity(
                &candidate_tool_keys(payload),
                &tool_name_from_payload(payload),
                mood,
            );
        }
        HookActivity::FinishToolBatch => {
            state.finish_session_tools(&session_id_from_payload(payload));
        }
        HookActivity::Error => {
            state.finish_session_tools(&session_id_from_payload(payload));
            state.last_error = summarize_payload(payload);
            state.set_resting_mood(PetMood::Error, true);
        }
        HookActivity::PermissionDenied => {
            state.finish_session_tools(&session_id_from_payload(payload));
            if state.activity_mood().is_none() {
                state.set_resting_mood(PetMood::Thinking, false);
            }
        }
        HookActivity::StartSubagent => {
            state.start_subagent();
        }
        HookActivity::FinishSubagent => {
            state.finish_subagent();
        }
        HookActivity::Done => {
            state.finish_session_tools(&session_id_from_payload(payload));
            if state.activity_mood().is_none() {
                state.set_resting_mood(PetMood::Happy, false);
            }
        }
        HookActivity::EndSession => {
            state.finish_session_tools(&session_id_from_payload(payload));
            if state.activity_mood().is_none() {
                state.set_resting_mood(PetMood::Idle, false);
            }
        }
    }
}

fn record_daily_stats(state: &mut AppState, event: &str, tool_name: &str) {
    match event {
        "UserPromptSubmit" => state.record_prompt_stats(),
        "PreToolUse" => state.record_tool_stats(tool_stats_kind(tool_name)),
        "PostToolUseFailure" | "StopFailure" => state.record_error_stats(),
        _ => {}
    }
}

fn mood_for_tool(tool_name: &str) -> PetMood {
    let normalized = tool_name.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "task" | "agent" => PetMood::Thinking,
        "bash" | "shell" => PetMood::Building,
        "edit" | "multiedit" | "write" | "notebookedit" => PetMood::Typing,
        "read" | "grep" | "glob" | "ls" | "webfetch" | "websearch" => PetMood::Search,
        "todoread" | "todowrite" | "askuserquestion" | "exitplanmode" => PetMood::Thinking,
        _ if normalized.contains("edit")
            || normalized.contains("write")
            || normalized.contains("patch")
            || normalized.contains("replace") =>
        {
            PetMood::Typing
        }
        _ if normalized.contains("bash")
            || normalized.contains("shell")
            || normalized.contains("terminal")
            || normalized.contains("command") =>
        {
            PetMood::Building
        }
        _ if normalized.contains("read")
            || normalized.contains("grep")
            || normalized.contains("glob")
            || normalized.contains("search")
            || normalized.contains("find")
            || normalized.contains("lookup")
            || normalized.contains("fetch")
            || normalized.contains("list") =>
        {
            PetMood::Search
        }
        _ => PetMood::Thinking,
    }
}

fn is_web_search_tool(tool_name: &str) -> bool {
    let normalized = compact_tool_name(tool_name);
    normalized == "websearch" || normalized.ends_with("websearch")
}

fn compact_tool_name(tool_name: &str) -> String {
    tool_name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn mood_for_tool_use(tool_name: &str, payload: &Value) -> PetMood {
    let trimmed = tool_name.trim();
    if !trimmed.eq_ignore_ascii_case("Task") && !trimmed.eq_ignore_ascii_case("Agent") {
        return mood_for_tool(tool_name);
    }

    let task_text = task_tool_text(payload);
    if looks_like_building_task(&task_text) {
        return PetMood::Building;
    }
    if looks_like_typing_task(&task_text) {
        return PetMood::Typing;
    }
    if looks_like_subagent_task(&task_text) {
        return PetMood::Subagent;
    }
    if looks_like_search_task(&task_text) {
        return PetMood::Search;
    }
    PetMood::Thinking
}

fn permission_visual_mood(payload: &Value, fallback_tool_name: &str) -> PetMood {
    let tool_name = string_field(payload, "tool_name")
        .or_else(|| string_field(payload, "toolName"))
        .unwrap_or_else(|| fallback_tool_name.to_string());
    let mood = mood_for_tool_use(&tool_name, payload);
    match mood {
        PetMood::Typing | PetMood::Building | PetMood::Search => mood,
        _ => PetMood::Thinking,
    }
}

fn task_tool_text(payload: &Value) -> String {
    let Some(input) = payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))
        .and_then(Value::as_object)
    else {
        return String::new();
    };

    ["description", "prompt", "task", "instructions"]
        .into_iter()
        .filter_map(|key| input.get(key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase()
}

fn tool_key(payload: &Value) -> String {
    let session_id = session_id_from_payload(payload);
    let tool_name = tool_name_from_payload(payload);
    if let Some(id) = string_field(payload, "tool_use_id")
        .or_else(|| string_field(payload, "toolUseId"))
        .or_else(|| string_field(payload, "toolUseID"))
        .filter(|key| !key.trim().is_empty())
    {
        return format!("{session_id}:id:{id}");
    }
    let fingerprint = tool_input_fingerprint(payload).unwrap_or_default();
    format!("{session_id}:tool:{tool_name}:{fingerprint}")
}

fn session_id_from_payload(payload: &Value) -> String {
    string_field(payload, "session_id")
        .or_else(|| string_field(payload, "sessionId"))
        .or_else(|| string_field(payload, "sessionID"))
        .filter(|session_id| !session_id.trim().is_empty())
        .unwrap_or_else(|| "default".to_string())
}

fn candidate_tool_keys(payload: &Value) -> Vec<String> {
    vec![tool_key(payload)]
}

fn tool_input_fingerprint(payload: &Value) -> Option<String> {
    let input = payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))?;
    Some(shorten(&input.to_string(), 96))
}

fn tool_name_from_payload(payload: &Value) -> String {
    string_field(payload, "tool_name")
        .or_else(|| string_field(payload, "toolName"))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "tool".to_string())
}

fn looks_like_typing_task(text: &str) -> bool {
    [
        "\u{4fee}\u{6539}",
        "\u{7f16}\u{8f91}",
        "\u{5199}\u{5165}",
        "\u{66f4}\u{65b0}",
        "\u{5b9e}\u{73b0}",
        "\u{4fee}\u{590d}",
        "\u{91cd}\u{6784}",
        "\u{521b}\u{5efa}",
        "\u{5220}\u{9664}",
        "\u{91cd}\u{547d}\u{540d}",
        "create",
        "delete",
        "rename",
        "edit",
        "write",
        "modify",
        "update",
        "patch",
        "replace",
        "refactor",
        "implement",
        "change",
        "fix",
    ]
    .into_iter()
    .any(|needle| text.contains(needle))
}

fn looks_like_building_task(text: &str) -> bool {
    [
        "\u{6784}\u{5efa}",
        "\u{7f16}\u{8bd1}",
        "\u{6d4b}\u{8bd5}",
        "\u{8fd0}\u{884c}",
        "cargo",
        "npm",
        "powershell",
        "build",
        "compile",
        "test",
        "run",
        "command",
        "shell",
        "bash",
    ]
    .into_iter()
    .any(|needle| text.contains(needle))
}

fn looks_like_subagent_task(text: &str) -> bool {
    [
        "\u{5b50}\u{4ee3}\u{7406}",
        "\u{4ee3}\u{7406}",
        "\u{59d4}\u{6d3e}",
        "\u{5e76}\u{884c}",
        "\u{89c4}\u{5212}",
        "subagent",
        "agent",
        "delegate",
        "parallel",
        "analyze",
        "analyse",
        "planning",
    ]
    .into_iter()
    .any(|needle| text.contains(needle))
}

fn looks_like_search_task(text: &str) -> bool {
    [
        "\u{67e5}\u{627e}",
        "\u{641c}\u{7d22}",
        "\u{641c}\u{5bfb}",
        "\u{68c0}\u{7d22}",
        "\u{8c03}\u{67e5}",
        "\u{7814}\u{7a76}",
        "search",
        "find",
        "grep",
        "glob",
        "lookup",
        "fetch",
        "read",
        "research",
        "investigate",
    ]
    .into_iter()
    .any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Condvar, Mutex};

    #[test]
    fn tool_names_map_to_work_moods() {
        assert_eq!(mood_for_tool("Task"), PetMood::Thinking);
        assert_eq!(mood_for_tool("Agent"), PetMood::Thinking);
        assert_eq!(mood_for_tool("Bash"), PetMood::Building);
        assert_eq!(mood_for_tool("Edit"), PetMood::Typing);
        assert_eq!(
            mood_for_tool("mcp__filesystem__write_file"),
            PetMood::Typing
        );
        assert_eq!(mood_for_tool("shell_command"), PetMood::Building);
        assert_eq!(mood_for_tool("agent_worker"), PetMood::Thinking);
        assert_eq!(mood_for_tool("Read"), PetMood::Search);
        assert_eq!(mood_for_tool("Grep"), PetMood::Search);
        assert_eq!(
            mood_for_tool("mcp__filesystem__search_files"),
            PetMood::Search
        );
        assert_eq!(mood_for_tool("AskUserQuestion"), PetMood::Thinking);
    }

    #[test]
    fn hook_events_map_to_expected_activity() {
        assert_eq!(
            hook_activity("UserPromptSubmit", "", &json!({})),
            Some(HookActivity::Thinking)
        );
        assert_eq!(
            hook_activity("PreToolUse", "Write", &json!({})),
            Some(HookActivity::StartTool(PetMood::Typing))
        );
        assert_eq!(
            hook_activity("PreToolUse", "Task", &json!({})),
            Some(HookActivity::StartTool(PetMood::Thinking))
        );
        assert_eq!(
            hook_activity(
                "PreToolUse",
                "Task",
                &json!({
                    "tool_input": {
                        "description": "Research and find references"
                    }
                })
            ),
            Some(HookActivity::StartTool(PetMood::Search))
        );
        assert_eq!(
            hook_activity(
                "PreToolUse",
                "Task",
                &json!({
                    "tool_input": {
                        "description": "Update the Rust hook state mapping",
                        "prompt": "\u{4fee}\u{6539} hooks \u{903b}\u{8f91}"
                    }
                })
            ),
            Some(HookActivity::StartTool(PetMood::Typing))
        );
        assert_eq!(
            hook_activity(
                "PreToolUse",
                "Task",
                &json!({
                    "tool_input": {
                        "description": "Delegate research to a subagent"
                    }
                })
            ),
            Some(HookActivity::StartTool(PetMood::Subagent))
        );
        assert_eq!(
            hook_activity(
                "PreToolUse",
                "Agent",
                &json!({
                    "tool_input": {
                        "description": "Delegate research to a subagent"
                    }
                })
            ),
            Some(HookActivity::StartTool(PetMood::Subagent))
        );
        assert_eq!(
            hook_activity("PostToolUse", "Task", &json!({})),
            Some(HookActivity::FinishTool(PetMood::Thinking))
        );
        assert_eq!(hook_activity("Notification", "", &json!({})), None);
        assert_eq!(
            hook_activity("PermissionDenied", "Write", &json!({})),
            Some(HookActivity::PermissionDenied)
        );
        assert_eq!(
            hook_activity("SessionEnd", "", &json!({ "reason": "clear" })),
            Some(HookActivity::EndSession)
        );
        assert_eq!(
            hook_activity("SessionEnd", "", &json!({})),
            Some(HookActivity::EndSession)
        );
    }

    #[test]
    fn permission_visual_moods_follow_tool_semantics() {
        assert_eq!(
            permission_visual_mood(&json!({ "tool_name": "Edit" }), ""),
            PetMood::Typing
        );
        assert_eq!(
            permission_visual_mood(&json!({ "tool_name": "Bash" }), ""),
            PetMood::Building
        );
        assert_eq!(
            permission_visual_mood(&json!({ "tool_name": "Read" }), ""),
            PetMood::Search
        );
        assert_eq!(
            permission_visual_mood(&json!({ "tool_name": "UnknownTool" }), ""),
            PetMood::Thinking
        );
    }

    #[test]
    fn pending_permission_keeps_tool_specific_visual() {
        let mut state = AppState::new();
        state.pending_permissions.push_back(PendingPermission {
            id: 1,
            interaction_sequence: 1,
            session_id: "s1".to_string(),
            tool_name: "Edit".to_string(),
            summary: "edit file".to_string(),
            cwd: String::new(),
            suggestions: Vec::new(),
            waiter: Arc::new(PermissionWaiter {
                decision: Mutex::new(None),
                ready: Condvar::new(),
            }),
        });

        state.set_resting_mood(PetMood::Typing, true);

        assert_eq!(state.activity_mood(), None);
        assert_eq!(state.mood, PetMood::Typing);
    }

    #[test]
    fn session_end_clears_matching_session_and_preserves_other_work() {
        let mut state = AppState::new();
        let s1_payload = json!({
            "session_id": "s1",
            "tool_name": "Write",
            "tool_use_id": "write-1",
        });
        let s2_payload = json!({
            "session_id": "s2",
            "tool_name": "Bash",
            "tool_use_id": "bash-1",
        });
        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Write", &s1_payload).unwrap(),
            &s1_payload,
        );
        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Bash", &s2_payload).unwrap(),
            &s2_payload,
        );
        state.start_subagent();
        assert_eq!(state.active_tools, 2);
        assert_eq!(state.active_subagents, 1);

        apply_hook_activity(
            &mut state,
            HookActivity::EndSession,
            &json!({ "session_id": "s1" }),
        );

        assert_eq!(state.active_tools, 1);
        assert_eq!(state.active_subagents, 1);
        assert_eq!(state.activity_mood(), Some(PetMood::Building));
        assert_ne!(state.mood, PetMood::Idle);
        assert_ne!(state.mood, PetMood::Error);
    }

    #[test]
    fn session_end_clears_only_matching_pending_interactions() {
        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            let mut s1 = build_permission("Edit");
            s1.id = 1;
            s1.session_id = "s1".to_string();
            let mut s2 = build_permission("Bash");
            s2.id = 2;
            s2.session_id = "s2".to_string();
            state.pending_permissions.push_back(s1);
            state.pending_permissions.push_back(s2);
        }

        process_hook(
            json!({
                "hook_event_name": "SessionEnd",
                "session_id": "s1"
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert_eq!(state.pending_permissions.len(), 1);
        assert_eq!(state.pending_permissions[0].session_id, "s2");
    }

    #[test]
    fn camel_case_session_id_is_tracked_as_session() {
        let state = Arc::new(Mutex::new(AppState::new()));

        process_hook(
            json!({
                "hook_event_name": "SessionStart",
                "sessionId": "camel-session"
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert!(state.sessions.contains_key("camel-session"));
        assert!(!state.sessions.contains_key("default"));
    }

    #[test]
    fn stop_still_sets_happy() {
        let mut state = AppState::new();

        apply_hook_activity(&mut state, HookActivity::Done, &json!({}));

        assert_eq!(state.resting_mood, PetMood::Happy);
        assert_eq!(state.mood, PetMood::Happy);
    }

    #[test]
    fn process_stop_sets_happy_with_focused_session_state() {
        let state = Arc::new(Mutex::new(AppState::new()));

        process_hook(
            json!({
                "hook_event_name": "Stop",
                "session_id": "s1"
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert_eq!(state.resting_mood, PetMood::Happy);
        assert_eq!(state.mood, PetMood::Happy);
    }

    #[test]
    fn permission_denied_does_not_force_error() {
        let mut state = AppState::new();
        let payload = json!({
            "session_id": "s1",
            "tool_name": "Write",
            "tool_use_id": "write-1",
        });
        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Write", &payload).unwrap(),
            &payload,
        );

        apply_hook_activity(
            &mut state,
            hook_activity("PermissionDenied", "Write", &payload).unwrap(),
            &payload,
        );

        assert_eq!(state.active_tools, 0);
        assert_eq!(state.resting_mood, PetMood::Thinking);
        assert_ne!(state.mood, PetMood::Error);
    }

    #[test]
    fn notification_does_not_pull_completed_work_back_to_thinking() {
        let mut state = AppState::new();
        apply_hook_activity(&mut state, HookActivity::Done, &json!({}));
        assert_eq!(state.mood, PetMood::Happy);

        if let Some(activity) = hook_activity("Notification", "", &json!({})) {
            apply_hook_activity(&mut state, activity, &json!({}));
        }

        assert_eq!(state.mood, PetMood::Happy);
    }

    #[test]
    fn post_tool_keeps_other_active_work_visible() {
        let mut state = AppState::new();

        state.start_tool_mood(PetMood::Thinking);
        state.start_tool_mood(PetMood::Building);
        state.finish_tool_mood(PetMood::Building);
        let next_mood = state.activity_mood().unwrap_or(PetMood::Happy);
        state.set_mood(next_mood);

        assert_eq!(state.active_tools, 1);
        assert_eq!(state.mood, PetMood::Building);
    }

    #[test]
    fn post_tool_reuses_started_task_mood_when_payload_is_sparse() {
        let mut state = AppState::new();
        let start_payload = json!({
            "tool_name": "Task",
            "tool_input": {
                "description": "Run cargo build and fix compile errors"
            }
        });

        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Task", &start_payload).unwrap(),
            &start_payload,
        );
        assert_eq!(state.mood, PetMood::Building);
        assert_eq!(state.active_tools, 1);
        assert_eq!(state.active_subagents, 0);

        let finish_payload = json!({ "tool_name": "Task" });
        apply_hook_activity(
            &mut state,
            hook_activity("PostToolUse", "Task", &finish_payload).unwrap(),
            &finish_payload,
        );
        assert_eq!(state.active_tools, 0);
        assert_eq!(state.active_subagents, 0);
    }

    #[test]
    fn short_write_holds_typing_after_post_tool() {
        let mut state = AppState::new();
        apply_hook_activity(
            &mut state,
            HookActivity::Thinking,
            &json!({ "session_id": "s1" }),
        );
        let start_payload = json!({
            "session_id": "s1",
            "tool_name": "Write",
            "tool_use_id": "write-1",
        });
        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Write", &start_payload).unwrap(),
            &start_payload,
        );
        let typing_started_at = state.mood_started_at;
        let finish_payload = json!({
            "session_id": "s1",
            "tool_name": "Write",
            "tool_use_id": "write-1",
        });

        apply_hook_activity(
            &mut state,
            hook_activity("PostToolUse", "Write", &finish_payload).unwrap(),
            &finish_payload,
        );
        assert_eq!(state.mood, PetMood::Typing);
        state.refresh_visual_mood_at(typing_started_at + Duration::from_millis(2_999));
        assert_eq!(state.mood, PetMood::Typing);
        state.refresh_visual_mood_at(typing_started_at + Duration::from_millis(3_001));
        assert_eq!(state.mood, PetMood::Thinking);
    }

    #[test]
    fn short_bash_holds_building_after_post_tool() {
        let mut state = AppState::new();
        apply_hook_activity(
            &mut state,
            HookActivity::Thinking,
            &json!({ "session_id": "s1" }),
        );
        let start_payload = json!({
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_use_id": "bash-1",
        });
        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Bash", &start_payload).unwrap(),
            &start_payload,
        );
        let building_started_at = state.mood_started_at;
        let finish_payload = json!({
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_use_id": "bash-1",
        });

        apply_hook_activity(
            &mut state,
            hook_activity("PostToolUse", "Bash", &finish_payload).unwrap(),
            &finish_payload,
        );
        assert_eq!(state.mood, PetMood::Building);
        state.refresh_visual_mood_at(building_started_at + Duration::from_millis(2_999));
        assert_eq!(state.mood, PetMood::Building);
        state.refresh_visual_mood_at(building_started_at + Duration::from_millis(3_001));
        assert_eq!(state.mood, PetMood::Thinking);
    }

    #[test]
    fn subagent_yields_to_direct_write_then_returns() {
        let mut state = AppState::new();
        apply_hook_activity(&mut state, HookActivity::StartSubagent, &json!({}));
        assert_eq!(state.mood, PetMood::Subagent);

        let start_payload = json!({
            "session_id": "s1",
            "tool_name": "Write",
            "tool_use_id": "write-1",
        });
        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Write", &start_payload).unwrap(),
            &start_payload,
        );
        assert_eq!(state.mood, PetMood::Typing);
        let typing_started_at = state.mood_started_at;

        let finish_payload = json!({
            "session_id": "s1",
            "tool_name": "Write",
            "tool_use_id": "write-1",
        });
        apply_hook_activity(
            &mut state,
            hook_activity("PostToolUse", "Write", &finish_payload).unwrap(),
            &finish_payload,
        );
        assert_eq!(state.mood, PetMood::Typing);
        state.refresh_visual_mood_at(typing_started_at + Duration::from_millis(3_001));
        assert_eq!(state.mood, PetMood::Subagent);
    }

    #[test]
    fn error_interrupts_work_and_returns_to_active_tool() {
        let mut state = AppState::new();
        let start_payload = json!({
            "session_id": "s1",
            "tool_name": "Write",
            "tool_use_id": "write-1",
        });
        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Write", &start_payload).unwrap(),
            &start_payload,
        );
        assert_eq!(state.mood, PetMood::Typing);

        state.set_mood(PetMood::Error);
        assert_eq!(state.mood, PetMood::Error);
        state.set_resting_mood(PetMood::Happy, false);
        assert_eq!(state.mood, PetMood::Typing);
    }

    #[test]
    fn happy_holds_against_idle_session_start() {
        let mut state = AppState::new();
        apply_hook_activity(&mut state, HookActivity::Done, &json!({}));
        let happy_started_at = state.mood_started_at;
        assert_eq!(state.mood, PetMood::Happy);

        apply_hook_activity(&mut state, HookActivity::Idle, &json!({}));
        assert_eq!(state.mood, PetMood::Happy);
        state.refresh_visual_mood_at(happy_started_at + Duration::from_millis(2_001));
        assert_eq!(state.mood, PetMood::Idle);
    }

    #[test]
    fn direct_tools_have_priority_over_subagents() {
        let mut state = AppState::new();

        state.start_tool_mood(PetMood::Building);
        state.active_subagents = 1;

        assert_eq!(state.activity_mood(), Some(PetMood::Building));
    }

    #[test]
    fn web_search_tool_names_use_pre_tool_permission_popup() {
        assert!(is_web_search_tool("WebSearch"));
        assert!(is_web_search_tool("Web Search"));
        assert!(is_web_search_tool("web_search"));
        assert!(is_web_search_tool("mcp__browser__web_search"));
        assert!(!is_web_search_tool("WebFetch"));
    }

    #[test]
    fn pre_tool_permission_response_allows_or_denies_web_search() {
        let permission = build_permission("WebSearch");
        let tool_input = json!({ "query": "claude code hooks" });

        let allow = permission_response(
            PermissionDecision::AllowOnce,
            &permission,
            PermissionResponseKind::PreToolUse {
                tool_input: tool_input.clone(),
            },
        );
        assert_eq!(
            allow
                .get("hookSpecificOutput")
                .and_then(|h| h.get("hookEventName"))
                .and_then(Value::as_str),
            Some("PreToolUse")
        );
        assert_eq!(
            allow
                .get("hookSpecificOutput")
                .and_then(|h| h.get("permissionDecision"))
                .and_then(Value::as_str),
            Some("allow")
        );
        assert_eq!(
            allow
                .get("hookSpecificOutput")
                .and_then(|h| h.get("updatedInput")),
            Some(&tool_input)
        );

        let deny = permission_response(
            PermissionDecision::Deny,
            &permission,
            PermissionResponseKind::PreToolUse { tool_input },
        );
        assert_eq!(
            deny.get("hookSpecificOutput")
                .and_then(|h| h.get("permissionDecision"))
                .and_then(Value::as_str),
            Some("deny")
        );
    }

    fn build_permission(tool_name: &str) -> PendingPermission {
        PendingPermission {
            id: 1,
            interaction_sequence: 1,
            session_id: "s1".to_string(),
            tool_name: tool_name.to_string(),
            summary: String::new(),
            cwd: String::new(),
            suggestions: Vec::new(),
            waiter: Arc::new(PermissionWaiter {
                decision: Mutex::new(None),
                ready: Condvar::new(),
            }),
        }
    }

    fn build_choice(questions: Vec<ChoiceQuestion>, kind: ChoiceKind) -> PendingChoice {
        let len = questions.len();
        PendingChoice {
            id: 1,
            interaction_sequence: 1,
            session_id: "s1".to_string(),
            kind,
            title: "t".to_string(),
            detail: "d".to_string(),
            questions,
            selected: vec![Vec::new(); len],
            other_text: vec![String::new(); len],
            tool_input: json!({}),
            waiter: Arc::new(ChoiceWaiter {
                decision: Mutex::new(None),
                ready: Condvar::new(),
            }),
        }
    }

    #[test]
    fn parse_ask_user_questions_appends_other_option() {
        let input = json!({
            "questions": [{
                "header": "h",
                "question": "Q?",
                "multiSelect": false,
                "options": [
                    {"label": "A", "description": "a"},
                    {"label": "B", "description": "b"}
                ]
            }]
        });
        let questions = parse_ask_user_questions(&input);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].options.len(), 3);
        assert!(questions[0].options[2].is_other);
        assert_eq!(questions[0].options[2].label, "Other...");
    }

    #[test]
    fn parse_ask_user_questions_skips_when_options_empty() {
        let input = json!({
            "questions": [{
                "question": "Q?",
                "options": []
            }]
        });
        let questions = parse_ask_user_questions(&input);
        assert!(questions.is_empty());
    }

    #[test]
    fn exit_plan_questions_includes_other_feedback_option() {
        let questions = exit_plan_questions(&json!({}));
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].options.len(), 3);
        assert_eq!(questions[0].options[0].label, "Approve plan");
        assert_eq!(questions[0].options[1].label, "Keep planning");
        assert!(questions[0].options[2].is_other);
    }

    #[test]
    fn is_submittable_requires_other_text_when_other_selected() {
        let mut choice = build_choice(exit_plan_questions(&json!({})), ChoiceKind::ExitPlanMode);
        // Select Other (index 2)
        choice.selected[0] = vec![2];
        choice.other_text[0] = String::new();
        assert!(!choice.is_submittable());

        choice.other_text[0] = "  ".to_string();
        assert!(!choice.is_submittable());

        choice.other_text[0] = "do X instead".to_string();
        assert!(choice.is_submittable());
    }

    #[test]
    fn is_submittable_blocks_when_any_question_unanswered() {
        let questions = vec![
            ChoiceQuestion {
                header: String::new(),
                question: "Q1".to_string(),
                multi_select: false,
                options: vec![ChoiceOption {
                    label: "A".to_string(),
                    description: String::new(),
                    is_other: false,
                }],
            },
            ChoiceQuestion {
                header: String::new(),
                question: "Q2".to_string(),
                multi_select: false,
                options: vec![ChoiceOption {
                    label: "B".to_string(),
                    description: String::new(),
                    is_other: false,
                }],
            },
        ];
        let mut choice = build_choice(questions, ChoiceKind::AskUserQuestion);
        choice.selected[0] = vec![0];
        assert!(!choice.is_submittable());
        choice.selected[1] = vec![0];
        assert!(choice.is_submittable());
    }

    #[test]
    fn ask_user_question_uses_other_text_in_answer() {
        let input = json!({
            "questions": [{
                "question": "Pick one",
                "options": [
                    {"label": "A", "description": ""}
                ]
            }]
        });
        let parsed = parse_ask_user_questions(&input);
        let mut choice = build_choice(parsed, ChoiceKind::AskUserQuestion);
        choice.tool_input = input;
        // Select Other (index 1)
        choice.selected[0] = vec![1];
        choice.other_text[0] = "custom answer".to_string();

        let response = choice_response(
            &choice,
            ChoiceDecision::Submit {
                selected: choice.selected.clone(),
                other_text: choice.other_text.clone(),
            },
        );
        let answer = response
            .get("hookSpecificOutput")
            .and_then(|h| h.get("updatedInput"))
            .and_then(|i| i.get("answers"))
            .and_then(|a| a.get("Pick one"))
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(answer, "custom answer");
    }

    #[test]
    fn exit_plan_other_path_denies_with_feedback() {
        let mut choice = build_choice(exit_plan_questions(&json!({})), ChoiceKind::ExitPlanMode);
        choice.selected[0] = vec![2];
        choice.other_text[0] = "please address concern X".to_string();
        let response = choice_response(
            &choice,
            ChoiceDecision::Submit {
                selected: choice.selected.clone(),
                other_text: choice.other_text.clone(),
            },
        );
        let reason = response
            .get("hookSpecificOutput")
            .and_then(|h| h.get("permissionDecisionReason"))
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(
            response
                .get("hookSpecificOutput")
                .and_then(|h| h.get("permissionDecision"))
                .and_then(Value::as_str),
            Some("deny")
        );
        assert!(reason.contains("please address concern X"));
    }

    #[test]
    fn summarize_exit_plan_keeps_long_plan_text() {
        let plan = "x".repeat(800);
        let input = json!({"plan": plan.clone()});
        let detail = summarize_exit_plan(&input);
        assert!(detail.len() >= 800);
        assert!(detail.starts_with("xxxx"));
    }

    #[test]
    fn summarize_exit_plan_preserves_markdown_newlines() {
        let input = json!({"plan": "# Title\r\n\r\n## Step\n- item"});
        let detail = summarize_exit_plan(&input);
        assert_eq!(detail, "# Title\n\n## Step\n- item");
    }

    #[test]
    fn summarize_tool_input_wraps_command_in_code_block() {
        let input = json!({ "command": "cargo test", "description": "Run the suite" });
        let summary = summarize_tool_input(&input).unwrap();
        assert!(summary.starts_with("Run the suite\n\n```"));
        assert!(summary.contains("cargo test"));
    }

    #[test]
    fn summarize_tool_input_renders_edit_as_diff() {
        let input = json!({
            "file_path": "src/lib_x.rs",
            "old_string": "let a = 1;",
            "new_string": "let a = 2;",
        });
        let summary = summarize_tool_input(&input).unwrap();
        assert!(summary.starts_with("```diff"));
        // A file path with underscores survives verbatim inside the fence.
        assert!(summary.contains("src/lib_x.rs"));
        assert!(summary.contains("-let a = 1;"));
        assert!(summary.contains("+let a = 2;"));
    }

    #[test]
    fn summarize_tool_input_renders_write_as_added_lines() {
        let input = json!({ "file_path": "new.rs", "content": "fn main() {}\n" });
        let summary = summarize_tool_input(&input).unwrap();
        assert!(summary.starts_with("```diff"));
        assert!(summary.contains("+fn main() {}"));
    }

    #[test]
    fn summarize_tool_input_shows_read_target_verbatim() {
        let input = json!({ "file_path": "src/main_app.rs" });
        let summary = summarize_tool_input(&input).unwrap();
        assert_eq!(summary, "```\nsrc/main_app.rs\n```");
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn summarize_payload(value: &Value) -> String {
    if let Some(tool_input) = value.get("tool_input").or_else(|| value.get("toolInput")) {
        if let Some(summary) = summarize_tool_input(tool_input) {
            return summary;
        }
    }
    if let Some(message) = string_field(value, "message")
        .or_else(|| string_field(value, "reason"))
        .or_else(|| string_field(value, "notification"))
    {
        return shorten(&message, PAYLOAD_SUMMARY_MAX_CHARS);
    }
    shorten(&value.to_string(), PAYLOAD_SUMMARY_MAX_CHARS)
}

/// Build a markdown summary of a tool call for the permission popup. Commands
/// render as fenced code and edits as ```diff blocks so the user sees the
/// actual change being requested, not just the file name.
fn summarize_tool_input(tool_input: &Value) -> Option<String> {
    // Shell command (Bash and similar): optional description + the command.
    if let Some(command) = string_field(tool_input, "command") {
        let mut out = String::new();
        if let Some(description) = string_field(tool_input, "description") {
            let description = description.trim();
            if !description.is_empty() {
                out.push_str(&shorten(description, 200));
                out.push_str("\n\n");
            }
        }
        out.push_str(&fenced(
            "",
            &shorten_block(command.trim_end_matches('\n'), PAYLOAD_SUMMARY_MAX_CHARS),
        ));
        return Some(out);
    }

    let path = tool_file_path(tool_input);

    // MultiEdit: one diff per edit under a shared file header.
    if let Some(edits) = tool_input.get("edits").and_then(Value::as_array) {
        let mut body = String::new();
        if let Some(path) = &path {
            body.push_str(&format!(" {path}\n"));
        }
        for (index, edit) in edits.iter().take(MAX_MULTIEDIT_EDITS).enumerate() {
            if index > 0 {
                body.push_str(" @@\n");
            }
            let old = string_field(edit, "old_string").unwrap_or_default();
            let new = string_field(edit, "new_string").unwrap_or_default();
            body.push_str(&diff_lines_text(&old, &new));
            body.push('\n');
        }
        if edits.len() > MAX_MULTIEDIT_EDITS {
            body.push_str(&format!(
                " … {} more edit(s)\n",
                edits.len() - MAX_MULTIEDIT_EDITS
            ));
        }
        return Some(fenced("diff", body.trim_end_matches('\n')));
    }

    // Single edit.
    if tool_input.get("old_string").is_some() || tool_input.get("new_string").is_some() {
        let old = string_field(tool_input, "old_string").unwrap_or_default();
        let new = string_field(tool_input, "new_string").unwrap_or_default();
        let mut body = String::new();
        if let Some(path) = &path {
            body.push_str(&format!(" {path}\n"));
        }
        body.push_str(&diff_lines_text(&old, &new));
        return Some(fenced("diff", &body));
    }

    // Write / create file: show the new content as added lines.
    if let Some(content) = string_field(tool_input, "content") {
        let mut body = String::new();
        if let Some(path) = &path {
            body.push_str(&format!(" {path}\n"));
        }
        body.push_str(&added_lines(&content));
        return Some(fenced("diff", body.trim_end_matches('\n')));
    }

    // Read/search style tools: surface the target verbatim in a code chip.
    for key in ["file_path", "path", "pattern", "url"] {
        if let Some(text) = string_field(tool_input, key) {
            return Some(fenced("", &shorten_block(&text, PAYLOAD_SUMMARY_MAX_CHARS)));
        }
    }
    // Prose-style input (e.g. prompts) reads better as plain text.
    string_field(tool_input, "prompt").map(|text| shorten(&text, PAYLOAD_SUMMARY_MAX_CHARS))
}

fn fenced(lang: &str, body: &str) -> String {
    format!("```{lang}\n{body}\n```")
}

/// Render `content` as `+` added diff lines, capped for long files.
fn added_lines(content: &str) -> String {
    let trimmed = content.trim_end_matches('\n');
    let total = trimmed.split('\n').count();
    let mut out: Vec<String> = trimmed
        .split('\n')
        .take(WRITE_CONTENT_MAX_LINES)
        .map(|line| format!("+{line}"))
        .collect();
    if total > WRITE_CONTENT_MAX_LINES {
        out.push(format!(
            "+… {} more line(s)",
            total - WRITE_CONTENT_MAX_LINES
        ));
    }
    out.join("\n")
}

fn tool_file_path(tool_input: &Value) -> Option<String> {
    ["file_path", "path", "notebook_path", "filePath"]
        .into_iter()
        .find_map(|key| string_field(tool_input, key))
        .filter(|value| !value.trim().is_empty())
}
