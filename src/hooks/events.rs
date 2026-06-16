use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::net::TcpStream;
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
    Shrug,
    PermissionDenied,
    StartSubagent,
    FinishSubagent,
    Done,
    EndSession,
}

#[cfg(test)]
fn process_hook(payload: Value, state: Arc<Mutex<AppState>>) -> Value {
    process_hook_on_connection(payload, state, None)
}

pub(crate) fn process_hook_on_connection(
    payload: Value,
    state: Arc<Mutex<AppState>>,
    connection: Option<&TcpStream>,
) -> Value {
    let event = string_field(&payload, "hook_event_name")
        .or_else(|| string_field(&payload, "hookEventName"))
        .unwrap_or_else(|| "unknown".to_string());
    let explicit_session_id = explicit_session_id_from_payload(&payload);
    let session_id = explicit_session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let cwd = string_field(&payload, "cwd").unwrap_or_default();
    let tool_name = string_field(&payload, "tool_name")
        .or_else(|| string_field(&payload, "toolName"))
        .unwrap_or_default();

    // Claude Code shows its own terminal prompt while this blocking hook is
    // pending, so the popup and the terminal options stay usable side by side.
    // Plan approval (ExitPlanMode) and questions (AskUserQuestion) also arrive
    // through this event; intercepting them at PreToolUse would block Claude
    // Code's own UI entirely and ignore auto/bypass permission modes.
    if event == "PermissionRequest" {
        return handle_permission_request(payload, state, session_id, cwd, tool_name, connection);
    }

    let transcript_path = string_field(&payload, "transcript_path")
        .or_else(|| string_field(&payload, "transcriptPath"));

    {
        let mut state = state.lock().expect("state poisoned");
        let event_session_id =
            event_session_id_for_state(&state, event.as_str(), explicit_session_id.as_deref());
        state.note_session_event(
            &event_session_id,
            &cwd,
            event.as_str(),
            &tool_name,
            mood_for_tool_use(&tool_name, &payload),
        );
        if clears_pending_interaction(event.as_str()) {
            clear_stale_interactions(&mut state, &event_session_id);
        }
        sweep_terminal_answered_permissions(
            &mut state,
            event.as_str(),
            &event_session_id,
            &tool_name,
            &payload,
        );
        let capture_official = state.llm_profiles.official_profile_active();
        update_quota_from_value(&mut state.quota, &payload, capture_official);
        if let Some(path) = transcript_path.as_deref() {
            state.quota.transcript_path = path.to_string();
        }
        record_daily_stats(&mut state, event.as_str(), &tool_name);
        state.record_token_snapshot();

        if let Some(activity) = hook_activity(event.as_str(), &tool_name, &payload) {
            apply_hook_activity_for_session(&mut state, activity, &payload, &event_session_id);
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
    connection: Option<&TcpStream>,
) -> Value {
    // AskUserQuestion is a question, not a tool approval: render the parsed
    // options as a selectable choice popup and answer through `updatedInput`
    // instead of dumping the raw tool input behind Allow/Deny. Unparseable
    // input falls through to the generic permission popup.
    if tool_name == "AskUserQuestion" {
        let tool_input = payload
            .get("tool_input")
            .or_else(|| payload.get("toolInput"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let questions = parse_ask_user_questions(&tool_input);
        if !questions.is_empty() {
            return handle_question_request(
                &payload, state, session_id, tool_input, questions, connection,
            );
        }
    }

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
            tool_use_id: payload_tool_use_id(&payload).unwrap_or_default(),
            tool_input_fingerprint: tool_input_fingerprint(&payload).unwrap_or_default(),
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
            // Claude Code aborts this hook request once the user answers in
            // the terminal dialog; a closed connection means the decision was
            // already made elsewhere, so drop the popup without a decision.
            if connection.is_some_and(connection_closed) {
                break Some(PermissionDecision::Ignore);
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
        // A denied or externally dismissed permission interrupts the turn: the
        // tool never runs and no PostToolUse/Stop hook follows, so the
        // activity opened by this tool's PreToolUse must be released here or
        // the pet stays stuck in its working mood.
        if !matches!(
            decision,
            Some(PermissionDecision::AllowOnce) | Some(PermissionDecision::AllowAlways)
        ) {
            state.finish_session_work(&permission.session_id);
        }
        let deny_interrupts = matches!(decision, Some(PermissionDecision::Deny))
            && !deny_falls_back_to_terminal(&permission.tool_name);
        if state.pending_permissions.is_empty() && state.pending_choices.is_empty() {
            let mood = match decision {
                Some(PermissionDecision::Deny) if deny_interrupts => PetMood::Deny,
                Some(PermissionDecision::Ignore) => state.activity_mood().unwrap_or(PetMood::Idle),
                _ => state.activity_mood().unwrap_or(PetMood::Happy),
            };
            state.set_resting_mood(mood, matches!(mood, PetMood::Error | PetMood::Deny));
        }
        let (status, detail) = match decision {
            Some(PermissionDecision::Deny) if deny_interrupts => {
                (ClaudeSessionStatus::Denied, "Permission denied".to_string())
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

    permission_request_response(decision.unwrap_or(PermissionDecision::Ignore), &permission)
}

// ExitPlanMode and AskUserQuestion arrive through PermissionRequest but are
// interactive flows, not plain tool approvals: a deny must let Claude Code
// fall back to its own terminal UI (keep planning / ask in terminal) instead
// of aborting the whole turn.
fn deny_falls_back_to_terminal(tool_name: &str) -> bool {
    matches!(tool_name, "ExitPlanMode" | "AskUserQuestion")
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
        PermissionDecision::Deny if deny_falls_back_to_terminal(&permission.tool_name) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "deny",
                    "message": "Declined in claudie; continue in the terminal"
                }
            }
        }),
        // "interrupt": true is what makes the deny behave like the terminal
        // "No": Claude Code aborts the whole turn instead of feeding the
        // denial back to the model as retryable tool feedback.
        PermissionDecision::Deny => with_turn_stop(
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "deny",
                        "message": "User denied the request in claudie",
                        "interrupt": true
                    }
                }
            }),
            "User denied the request in claudie",
        ),
        PermissionDecision::Ignore => json!({}),
    }
}

// AskUserQuestion arrives as a blocking PermissionRequest like any other
// tool, but waits on a PendingChoice so the popup can offer the actual
// options. Submitting answers via `updatedInput` mirrors what Claude Code's
// own dialog sends; cancelling falls back to the terminal flow exactly like
// a permission deny on this tool.
fn handle_question_request(
    payload: &Value,
    state: Arc<Mutex<AppState>>,
    session_id: String,
    tool_input: Value,
    questions: Vec<ChoiceQuestion>,
    connection: Option<&TcpStream>,
) -> Value {
    let transcript_path = string_field(payload, "transcript_path")
        .or_else(|| string_field(payload, "transcriptPath"));
    let transcript_start = transcript_path
        .as_deref()
        .and_then(transcript_len)
        .unwrap_or(0);

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
        let question_count = questions.len();
        let choice = PendingChoice {
            id,
            interaction_sequence,
            session_id,
            kind: ChoiceKind::AskUserQuestion,
            title: "Question from Claude".to_string(),
            detail: String::new(),
            questions,
            selected: vec![Vec::new(); question_count],
            other_text: vec![String::new(); question_count],
            tool_input,
            waiter: waiter.clone(),
        };
        state.pending_choices.push_back(choice.clone());
        let detail = choice
            .questions
            .first()
            .map(|question| question.question.clone())
            .unwrap_or_default();
        state.mark_session_waiting_choice(&choice.session_id, &detail, choice.interaction_sequence);
        state.start_choice_activity(choice.id, &choice.session_id, PetMood::Thinking);
        state.set_resting_mood(PetMood::Thinking, true);
        state.record_choice_stats();
        let capture_official = state.llm_profiles.official_profile_active();
        update_quota_from_value(&mut state.quota, payload, capture_official);
        state.record_token_snapshot();
        choice
    };

    let decision = {
        let mut guard = waiter.decision.lock().expect("choice waiter poisoned");
        loop {
            if guard.is_some() {
                break guard.clone();
            }
            // Claude Code aborts this hook request once the user answers in
            // the terminal dialog; a closed connection means the decision was
            // already made elsewhere, so drop the popup without a decision.
            if connection.is_some_and(connection_closed) {
                break Some(ChoiceDecision::Ignore);
            }
            if transcript_path
                .as_deref()
                .is_some_and(|path| transcript_has_terminal_denial(path, transcript_start))
            {
                break Some(ChoiceDecision::Ignore);
            }
            let (next_guard, _) = waiter
                .ready
                .wait_timeout(guard, TRANSCRIPT_DENIAL_POLL)
                .expect("choice waiter poisoned");
            guard = next_guard;
        }
    };

    {
        let mut state = state.lock().expect("state poisoned");
        state.finish_choice_activity(choice.id);
        state
            .pending_choices
            .retain(|pending| pending.id != choice.id);
        let submitted = matches!(decision, Some(ChoiceDecision::Submit { .. }));
        // Only a submitted answer lets the tool run; everything else leaves
        // the question to the terminal, so release this session's activity
        // like a denied permission would.
        if !submitted {
            state.finish_session_work(&choice.session_id);
        }
        if state.pending_permissions.is_empty() && state.pending_choices.is_empty() {
            let mood = if submitted {
                state.activity_mood().unwrap_or(PetMood::Happy)
            } else {
                state.activity_mood().unwrap_or(PetMood::Idle)
            };
            state.set_resting_mood(mood, false);
        }
        let (status, detail) = match decision {
            Some(ChoiceDecision::Submit { .. }) | Some(ChoiceDecision::Deny) => {
                (ClaudeSessionStatus::Streaming, "Streaming".to_string())
            }
            Some(ChoiceDecision::Ignore) | None => {
                (ClaudeSessionStatus::Done, "Question closed".to_string())
            }
        };
        state.mark_session_interaction_finished(
            &choice.session_id,
            choice.interaction_sequence,
            status,
            detail,
        );
    }

    question_request_response(&choice, decision.unwrap_or(ChoiceDecision::Ignore))
}

fn question_request_response(choice: &PendingChoice, decision: ChoiceDecision) -> Value {
    match decision {
        ChoiceDecision::Submit {
            selected,
            other_text,
        } => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "allow",
                    "updatedInput": answered_tool_input(choice, &selected, &other_text)
                }
            }
        }),
        ChoiceDecision::Deny => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {
                    "behavior": "deny",
                    "message": "Declined in claudie; continue in the terminal"
                }
            }
        }),
        ChoiceDecision::Ignore => json!({}),
    }
}

// Claude Code's own question dialog answers AskUserQuestion by adding an
// `answers` map (question text -> chosen label) to the tool input; replicate
// that shape so the model sees the selection exactly as if answered natively.
fn answered_tool_input(
    choice: &PendingChoice,
    selected: &[Vec<usize>],
    other_text: &[String],
) -> Value {
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
                            .map(|text| text.trim())
                            .unwrap_or("");
                        (!text.is_empty()).then(|| text.to_string())
                    } else {
                        Some(option.label.clone())
                    }
                })
                .collect();
            answers.insert(question.question.clone(), Value::String(parts.join(", ")));
        }
        obj.insert("answers".to_string(), Value::Object(answers));
    }
    updated_input
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

// Denials must stop the turn like the terminal "No" option; otherwise Claude
// retries immediately and the popup reappears before the user can react.
fn with_turn_stop(mut response: Value, stop_reason: &str) -> Value {
    if let Some(obj) = response.as_object_mut() {
        obj.insert("continue".to_string(), Value::Bool(false));
        obj.insert("stopReason".to_string(), json!(stop_reason));
    }
    response
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
    let compact_json = text
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();
    // Only the structural records Claude Code itself writes on a terminal
    // "No" / Esc count. Loose phrase matching used to dismiss popups whenever
    // code or chat text in the transcript merely mentioned a denial. The
    // quote-anchored prefixes cannot occur inside nested tool-result text
    // because there the quotes are JSON-escaped (\").
    const DENIAL_MARKERS: [&str; 4] = [
        "\"content\":\"theuserdoesn'twanttoproceedwiththistooluse",
        "\"text\":\"theuserdoesn'twanttoproceedwiththistooluse",
        "\"tooluseresult\":\"userrejected",
        "\"content\":\"[requestinterruptedbyuser",
    ];
    DENIAL_MARKERS
        .iter()
        .any(|marker| compact_json.contains(marker))
}

// Non-destructive probe for "Claude Code gave up on this hook request". The
// stream is switched to non-blocking only for the peek so the final response
// write (on the same socket) keeps its blocking semantics.
fn connection_closed(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let mut probe = [0_u8; 1];
    let closed = match stream.peek(&mut probe) {
        Ok(0) => true,
        Ok(_) => false,
        Err(err) => err.kind() != std::io::ErrorKind::WouldBlock,
    };
    let _ = stream.set_nonblocking(false);
    closed
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

// Resolve (with Ignore) every pending permission the predicate matches: the
// popup closes and Claude Code receives an empty hook response, leaving its
// own native flow in charge.
fn resolve_pending_permissions(
    state: &mut AppState,
    mut should_resolve: impl FnMut(&PendingPermission) -> bool,
) {
    let mut resolved = Vec::new();
    state.pending_permissions.retain(|pending| {
        if should_resolve(pending) {
            resolved.push(pending.clone());
            false
        } else {
            true
        }
    });

    for pending in resolved {
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
}

// Choice-popup counterpart of resolve_pending_permissions: close the popup
// and answer the blocked hook with an empty response.
fn resolve_pending_choices(
    state: &mut AppState,
    mut should_resolve: impl FnMut(&PendingChoice) -> bool,
) {
    let mut resolved = Vec::new();
    state.pending_choices.retain(|pending| {
        if should_resolve(pending) {
            resolved.push(pending.clone());
            false
        } else {
            true
        }
    });

    for pending in resolved {
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

// Both the claudie popup and the Claude Code terminal prompt are live at the
// same time, so forward progress in a session can reveal that a pending
// permission was already answered in the terminal. Only precise tool_use_id
// matches and the always-turn-blocking plan/question tools are swept here;
// unrelated parallel tools finishing must never dismiss someone else's popup.
fn sweep_terminal_answered_permissions(
    state: &mut AppState,
    event: &str,
    session_id: &str,
    tool_name: &str,
    payload: &Value,
) {
    match event {
        "PostToolUse" | "PostToolUseFailure" => {
            let tool_use_id = payload_tool_use_id(payload);
            let fingerprint = tool_input_fingerprint(payload);
            resolve_pending_permissions(state, |pending| {
                if pending.session_id != session_id {
                    return false;
                }
                if let Some(id) = tool_use_id.as_deref() {
                    if !pending.tool_use_id.is_empty() && pending.tool_use_id == id {
                        return true;
                    }
                }
                // Fingerprint fallback: when the PermissionRequest carried no
                // tool_use_id we cannot match by id, so a follow-up event for
                // the same tool + identical input is treated as "answered in
                // the terminal". Restricted to entries without a tool_use_id so
                // a parallel tool that does have one is only ever matched by id.
                if pending.tool_use_id.is_empty() {
                    if let Some(fingerprint) = fingerprint.as_deref() {
                        if !pending.tool_input_fingerprint.is_empty()
                            && pending.tool_input_fingerprint == fingerprint
                            && pending.tool_name.eq_ignore_ascii_case(tool_name)
                        {
                            return true;
                        }
                    }
                }
                deny_falls_back_to_terminal(&pending.tool_name)
            });
            // Question popups are always turn-blocking, so like the
            // plan/question permissions above, forward progress in the same
            // session means they were answered in the terminal.
            resolve_pending_choices(state, |pending| pending.session_id == session_id);
        }
        // Plan approval in the terminal is followed by execution, not by a
        // PostToolUse for ExitPlanMode — any other tool starting means the
        // plan dialog was already resolved there.
        "PreToolUse" if tool_name != "ExitPlanMode" => {
            resolve_pending_permissions(state, |pending| {
                pending.session_id == session_id && pending.tool_name == "ExitPlanMode"
            });
        }
        _ => {}
    }
}

fn clear_stale_interactions(state: &mut AppState, session_id: &str) {
    resolve_pending_permissions(state, |pending| pending.session_id == session_id);
    resolve_pending_choices(state, |pending| pending.session_id == session_id);
}

fn clears_pending_interaction(event: &str) -> bool {
    // Tool events (PreToolUse/PostToolUse/...) must not clear pending
    // interactions: tools run in parallel, and another tool finishing while a
    // permission popup waits would dismiss it before the user can respond.
    matches!(
        event,
        "UserPromptSubmit" | "PermissionDenied" | "Stop" | "SessionEnd" | "StopFailure"
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
        // A single tool failing is a recoverable hiccup: shrug it off. A
        // turn-ending failure is a real error.
        "PostToolUseFailure" => Some(HookActivity::Shrug),
        "StopFailure" => Some(HookActivity::Error),
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

#[cfg(test)]
fn apply_hook_activity(state: &mut AppState, activity: HookActivity, payload: &Value) {
    let session_id = session_id_from_payload(payload);
    apply_hook_activity_for_session(state, activity, payload, &session_id);
}

fn apply_hook_activity_for_session(
    state: &mut AppState,
    activity: HookActivity,
    payload: &Value,
    session_id: &str,
) {
    match activity {
        HookActivity::Idle => {
            state.set_resting_mood(PetMood::Idle, false);
        }
        HookActivity::Thinking => {
            state.set_resting_mood(PetMood::Thinking, true);
        }
        HookActivity::StartTool(mood) => {
            state.start_tool_activity(
                tool_key_for_session(payload, session_id),
                tool_name_from_payload(payload),
                mood,
            );
        }
        HookActivity::FinishTool(mood) => {
            state.finish_tool_activity(
                &candidate_tool_keys_for_session(payload, session_id),
                session_id,
                &tool_name_from_payload(payload),
                mood,
            );
        }
        HookActivity::FinishToolBatch => {
            state.finish_session_tools(session_id);
        }
        HookActivity::Error => {
            state.finish_session_work(session_id);
            state.last_error = summarize_payload(payload);
            state.set_resting_mood(PetMood::Error, true);
        }
        HookActivity::Shrug => {
            // Release the failed tool's working span so the (low-priority)
            // shrug actually surfaces; interrupts_visual=true forces the switch
            // away from the just-cleared working mood. No last_error: this is a
            // recoverable hiccup, not a turn-ending error.
            state.finish_session_work(session_id);
            state.set_resting_mood(PetMood::Shrug, true);
        }
        HookActivity::PermissionDenied => {
            state.finish_session_work(session_id);
            if state.activity_mood().is_none() {
                state.set_resting_mood(PetMood::Idle, true);
            }
        }
        HookActivity::StartSubagent => {
            state.start_subagent_for_session(session_id);
        }
        HookActivity::FinishSubagent => {
            state.finish_subagent_for_session(session_id);
        }
        HookActivity::Done => {
            state.finish_session_work(session_id);
            if state.activity_mood().is_none() {
                state.set_resting_mood(PetMood::Happy, true);
            }
        }
        HookActivity::EndSession => {
            state.finish_session_work(session_id);
            if state.activity_mood().is_none() {
                state.set_resting_mood(PetMood::Idle, true);
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

fn tool_key_for_session(payload: &Value, session_id: &str) -> String {
    let tool_name = tool_name_from_payload(payload);
    if let Some(id) = payload_tool_use_id(payload) {
        return format!("{session_id}:id:{id}");
    }
    let fingerprint = tool_input_fingerprint(payload).unwrap_or_default();
    format!("{session_id}:tool:{tool_name}:{fingerprint}")
}

fn payload_tool_use_id(payload: &Value) -> Option<String> {
    string_field(payload, "tool_use_id")
        .or_else(|| string_field(payload, "toolUseId"))
        .or_else(|| string_field(payload, "toolUseID"))
        .filter(|key| !key.trim().is_empty())
}

#[cfg(test)]
fn session_id_from_payload(payload: &Value) -> String {
    explicit_session_id_from_payload(payload).unwrap_or_else(|| "default".to_string())
}

fn explicit_session_id_from_payload(payload: &Value) -> Option<String> {
    string_field(payload, "session_id")
        .or_else(|| string_field(payload, "sessionId"))
        .or_else(|| string_field(payload, "sessionID"))
        .filter(|session_id| !session_id.trim().is_empty())
        .map(|session_id| session_id.trim().to_string())
}

fn event_session_id_for_state(
    state: &AppState,
    event: &str,
    explicit_session_id: Option<&str>,
) -> String {
    if let Some(session_id) = explicit_session_id.filter(|session_id| !session_id.trim().is_empty())
    {
        return session_id.trim().to_string();
    }
    if event_uses_focused_session_when_missing_id(event) {
        if let Some(session_id) = state.focused_session_id.as_deref() {
            return session_id.to_string();
        }
    }
    "default".to_string()
}

fn event_uses_focused_session_when_missing_id(event: &str) -> bool {
    matches!(
        event,
        "PostToolUse"
            | "PostToolBatch"
            | "PermissionDenied"
            | "PostToolUseFailure"
            | "Stop"
            | "SessionEnd"
            | "StopFailure"
            | "SubagentStop"
            | "TaskCompleted"
            | "PostCompact"
    )
}

fn candidate_tool_keys_for_session(payload: &Value, session_id: &str) -> Vec<String> {
    vec![tool_key_for_session(payload, session_id)]
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
    use crate::app::{ChoiceKind, ChoiceOption, ChoiceQuestion, ChoiceWaiter, PendingChoice};
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
            tool_use_id: String::new(),
            tool_input_fingerprint: String::new(),
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
    fn parallel_tool_events_keep_pending_permission() {
        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            let mut permission = build_permission("Bash");
            permission.session_id = "s1".to_string();
            state.pending_permissions.push_back(permission);
        }

        process_hook(
            json!({
                "hook_event_name": "PreToolUse",
                "session_id": "s1",
                "tool_name": "Read",
                "tool_use_id": "read-1"
            }),
            state.clone(),
        );
        process_hook(
            json!({
                "hook_event_name": "PostToolUse",
                "session_id": "s1",
                "tool_name": "Read",
                "tool_use_id": "read-1"
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert_eq!(state.pending_permissions.len(), 1);
    }

    #[test]
    fn post_tool_use_fingerprint_resolves_idless_permission() {
        // A PermissionRequest that carried no tool_use_id cannot be matched by
        // id, so the follow-up PostToolUse for the same tool + identical input
        // (answered in the terminal) must dismiss it via the fingerprint path.
        let tool_input = json!({ "command": "cargo test" });
        let fingerprint = tool_input_fingerprint(&json!({ "tool_input": tool_input.clone() }))
            .expect("fingerprint");

        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            let mut permission = build_permission("Bash");
            permission.session_id = "s1".to_string();
            permission.tool_use_id = String::new();
            permission.tool_input_fingerprint = fingerprint;
            state.pending_permissions.push_back(permission);
        }

        process_hook(
            json!({
                "hook_event_name": "PostToolUse",
                "session_id": "s1",
                "tool_name": "Bash",
                "tool_input": tool_input,
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert!(state.pending_permissions.is_empty());
    }

    #[test]
    fn post_tool_use_fingerprint_ignores_other_sessions() {
        // The fingerprint fallback must stay session-scoped: an identical tool
        // call in a different session must not dismiss this popup.
        let tool_input = json!({ "command": "cargo test" });
        let fingerprint = tool_input_fingerprint(&json!({ "tool_input": tool_input.clone() }))
            .expect("fingerprint");

        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            let mut permission = build_permission("Bash");
            permission.session_id = "s1".to_string();
            permission.tool_use_id = String::new();
            permission.tool_input_fingerprint = fingerprint;
            state.pending_permissions.push_back(permission);
        }

        process_hook(
            json!({
                "hook_event_name": "PostToolUse",
                "session_id": "s2",
                "tool_name": "Bash",
                "tool_input": tool_input,
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert_eq!(state.pending_permissions.len(), 1);
    }

    #[test]
    fn transcript_denial_ignores_incidental_permission_text() {
        let path = std::env::temp_dir().join(format!(
            "claudie_test_transcript_{}.jsonl",
            std::process::id()
        ));
        let path_str = path.to_str().expect("temp path");

        fs::write(&path, "a diff mentioning the PermissionDenied hook event\n").unwrap();
        assert!(!transcript_has_terminal_denial(path_str, 0));

        // Loose prose about denials must not dismiss the popup anymore.
        fs::write(&path, "the request was denied by user\n").unwrap();
        assert!(!transcript_has_terminal_denial(path_str, 0));

        // Denial markers nested inside tool-result text arrive JSON-escaped
        // (e.g. while a Claude session reads claudie's own source) and must
        // not match either.
        fs::write(
            &path,
            "{\"content\":\"code: \\\"toolUseResult\\\": \\\"User rejected tool use\\\"\"}\n",
        )
        .unwrap();
        assert!(!transcript_has_terminal_denial(path_str, 0));

        // The actual record Claude Code writes when the user picks "No" in
        // the terminal permission dialog.
        let rejection = concat!(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","#,
            r#""content":"The user doesn't want to proceed with this tool use. "#,
            r#"The tool use was rejected.","is_error":true,"tool_use_id":"toolu_1"}]},"#,
            r#""toolUseResult":"User rejected tool use"}"#,
            "\n"
        );
        fs::write(&path, rejection).unwrap();
        assert!(transcript_has_terminal_denial(path_str, 0));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn terminal_denial_releases_started_tool_activity() {
        let path = std::env::temp_dir().join(format!(
            "claudie_test_denial_release_{}.jsonl",
            std::process::id()
        ));
        fs::write(&path, "{}\n").unwrap();
        let transcript = path.to_str().expect("temp path").to_string();

        let state = Arc::new(Mutex::new(AppState::new()));
        process_hook(
            json!({
                "hook_event_name": "PreToolUse",
                "session_id": "s1",
                "tool_name": "Bash",
                "tool_use_id": "bash-1",
                "tool_input": { "command": "cargo test" }
            }),
            state.clone(),
        );
        assert_eq!(state.lock().expect("state poisoned").active_tools, 1);

        let worker = {
            let state = state.clone();
            let transcript = transcript.clone();
            std::thread::spawn(move || {
                process_hook(
                    json!({
                        "hook_event_name": "PermissionRequest",
                        "session_id": "s1",
                        "tool_name": "Bash",
                        "tool_input": { "command": "cargo test" },
                        "transcript_path": transcript
                    }),
                    state,
                )
            })
        };

        // Let the popup register, then simulate the user answering "No" in
        // the Claude Code terminal dialog. The waiter captures the transcript
        // length before pushing the pending permission, so once the popup is
        // visible the rejection below is guaranteed to land past that offset.
        let deadline = Instant::now() + Duration::from_secs(5);
        while state
            .lock()
            .expect("state poisoned")
            .pending_permissions
            .is_empty()
        {
            assert!(Instant::now() < deadline, "permission never registered");
            std::thread::sleep(Duration::from_millis(10));
        }
        let rejection = concat!(
            "{}\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","#,
            r#""content":"The user doesn't want to proceed with this tool use. "#,
            r#"The tool use was rejected.","is_error":true,"tool_use_id":"bash-1"}]},"#,
            r#""toolUseResult":"User rejected tool use"}"#,
            "\n"
        );
        fs::write(&path, rejection).unwrap();
        worker.join().expect("permission hook thread");

        let state = state.lock().expect("state poisoned");
        assert!(state.pending_permissions.is_empty());
        assert_eq!(state.active_tools, 0);
        assert_eq!(state.resting_mood, PetMood::Idle);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn stop_without_session_id_finishes_focused_session() {
        let state = Arc::new(Mutex::new(AppState::new()));
        process_hook(
            json!({
                "hook_event_name": "UserPromptSubmit",
                "session_id": "s1"
            }),
            state.clone(),
        );

        process_hook(json!({ "hook_event_name": "Stop" }), state.clone());

        let state = state.lock().expect("state poisoned");
        let session = state.sessions.get("s1").expect("focused session");
        assert_eq!(session.status, ClaudeSessionStatus::Done);
        assert_eq!(state.resting_mood, PetMood::Happy);
        assert_eq!(state.mood, PetMood::Happy);
    }

    #[test]
    fn permission_denied_without_session_id_clears_focused_popup() {
        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            state.mark_session_waiting_permission("s1", "", "Edit", 1);
            let mut permission = build_permission("Edit");
            permission.session_id = "s1".to_string();
            state.pending_permissions.push_back(permission);
        }

        process_hook(
            json!({ "hook_event_name": "PermissionDenied" }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert!(state.pending_permissions.is_empty());
        let session = state.sessions.get("s1").expect("focused session");
        assert_eq!(session.status, ClaudeSessionStatus::Done);
        assert_eq!(state.resting_mood, PetMood::Idle);
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
    fn late_subagent_stop_does_not_revive_finished_session() {
        let state = Arc::new(Mutex::new(AppState::new()));
        process_hook(
            json!({ "hook_event_name": "UserPromptSubmit", "session_id": "s1" }),
            state.clone(),
        );
        process_hook(
            json!({ "hook_event_name": "Stop", "session_id": "s1" }),
            state.clone(),
        );

        process_hook(
            json!({ "hook_event_name": "SubagentStop", "session_id": "s1" }),
            state.clone(),
        );
        process_hook(
            json!({ "hook_event_name": "TaskCompleted", "session_id": "s1" }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        let session = state.sessions.get("s1").expect("session");
        assert_eq!(session.status, ClaudeSessionStatus::Done);
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
        assert_eq!(state.resting_mood, PetMood::Idle);
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
    fn tool_failure_shrugs_off_and_is_overridden_by_next_tool() {
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
        assert_eq!(state.mood, PetMood::Typing);

        // A single tool failing is a recoverable hiccup: shrug, not Error.
        apply_hook_activity(
            &mut state,
            hook_activity("PostToolUseFailure", "Write", &payload).unwrap(),
            &payload,
        );
        assert_eq!(state.mood, PetMood::Shrug);

        // The next tool's working mood immediately overrides the low-priority shrug.
        let next = json!({
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_use_id": "bash-1",
        });
        apply_hook_activity(
            &mut state,
            hook_activity("PreToolUse", "Bash", &next).unwrap(),
            &next,
        );
        assert_eq!(state.mood, PetMood::Building);
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
    fn post_tool_use_resolves_pending_permission_with_matching_tool_use_id() {
        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            let mut permission = build_permission("Bash");
            permission.tool_use_id = "bash-1".to_string();
            state.pending_permissions.push_back(permission);
        }

        process_hook(
            json!({
                "hook_event_name": "PostToolUse",
                "session_id": "s1",
                "tool_name": "Bash",
                "tool_use_id": "bash-1"
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert!(state.pending_permissions.is_empty());
    }

    #[test]
    fn post_tool_use_sweeps_pending_plan_and_question_popups() {
        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            let mut plan = build_permission("ExitPlanMode");
            plan.id = 1;
            let mut question = build_permission("AskUserQuestion");
            question.id = 2;
            state.pending_permissions.push_back(plan);
            state.pending_permissions.push_back(question);
        }

        // Forward progress in the same session means both blocking dialogs
        // were answered in the terminal.
        process_hook(
            json!({
                "hook_event_name": "PostToolUse",
                "session_id": "s1",
                "tool_name": "Read",
                "tool_use_id": "read-1"
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert!(state.pending_permissions.is_empty());
    }

    #[test]
    fn pre_tool_use_sweeps_pending_exit_plan_only() {
        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            let mut plan = build_permission("ExitPlanMode");
            plan.id = 1;
            let mut bash = build_permission("Bash");
            bash.id = 2;
            state.pending_permissions.push_back(plan);
            state.pending_permissions.push_back(bash);
        }

        process_hook(
            json!({
                "hook_event_name": "PreToolUse",
                "session_id": "s1",
                "tool_name": "Edit",
                "tool_use_id": "edit-1"
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert_eq!(state.pending_permissions.len(), 1);
        assert_eq!(state.pending_permissions[0].tool_name, "Bash");
    }

    #[test]
    fn closed_connection_dismisses_waiting_permission() {
        use std::net::{TcpListener, TcpStream as StdTcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client = StdTcpStream::connect(addr).expect("connect");
        let (server_side, _) = listener.accept().expect("accept");

        assert!(!connection_closed(&server_side));

        let state = Arc::new(Mutex::new(AppState::new()));
        let worker = {
            let state = state.clone();
            std::thread::spawn(move || {
                process_hook_on_connection(
                    json!({
                        "hook_event_name": "PermissionRequest",
                        "session_id": "s1",
                        "tool_name": "Bash",
                        "tool_input": { "command": "cargo test" }
                    }),
                    state,
                    Some(&server_side),
                )
            })
        };

        let deadline = Instant::now() + Duration::from_secs(5);
        while state
            .lock()
            .expect("state poisoned")
            .pending_permissions
            .is_empty()
        {
            assert!(Instant::now() < deadline, "permission never registered");
            std::thread::sleep(Duration::from_millis(10));
        }

        // Claude Code answering in the terminal aborts the hook request.
        drop(client);

        let response = worker.join().expect("permission hook thread");
        assert_eq!(response, json!({}));
        let state = state.lock().expect("state poisoned");
        assert!(state.pending_permissions.is_empty());
    }

    fn build_permission(tool_name: &str) -> PendingPermission {
        PendingPermission {
            id: 1,
            interaction_sequence: 1,
            session_id: "s1".to_string(),
            tool_name: tool_name.to_string(),
            tool_use_id: String::new(),
            tool_input_fingerprint: String::new(),
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
    fn is_submittable_requires_other_text_when_other_selected() {
        let questions = vec![ChoiceQuestion {
            header: String::new(),
            question: "Q".to_string(),
            multi_select: false,
            options: vec![
                ChoiceOption {
                    label: "A".to_string(),
                    description: String::new(),
                    is_other: false,
                },
                ChoiceOption {
                    label: "Other...".to_string(),
                    description: String::new(),
                    is_other: true,
                },
            ],
        }];
        let mut choice = build_choice(questions, ChoiceKind::ExitPlanMode);
        // Select Other (index 1)
        choice.selected[0] = vec![1];
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
    fn permission_deny_stops_turn() {
        let permission = build_permission("Bash");
        let response = permission_request_response(PermissionDecision::Deny, &permission);
        assert_eq!(
            response.get("continue").and_then(Value::as_bool),
            Some(false)
        );
        assert!(response.get("stopReason").and_then(Value::as_str).is_some());
        assert_eq!(
            response["hookSpecificOutput"]["decision"]["behavior"].as_str(),
            Some("deny")
        );
        assert_eq!(
            response["hookSpecificOutput"]["decision"]["interrupt"].as_bool(),
            Some(true)
        );
        assert!(
            response["hookSpecificOutput"]["decision"]["message"]
                .as_str()
                .is_some()
        );

        let allow = permission_request_response(PermissionDecision::AllowOnce, &permission);
        assert!(allow.get("continue").is_none());
    }

    #[test]
    fn plan_and_question_deny_falls_back_to_terminal_without_stopping_turn() {
        for tool in ["ExitPlanMode", "AskUserQuestion"] {
            let permission = build_permission(tool);
            let response = permission_request_response(PermissionDecision::Deny, &permission);
            assert!(response.get("continue").is_none(), "{tool} stopped turn");
            assert_eq!(
                response["hookSpecificOutput"]["decision"]["behavior"].as_str(),
                Some("deny")
            );
            assert!(
                response["hookSpecificOutput"]["decision"]
                    .get("interrupt")
                    .is_none(),
                "{tool} carried interrupt"
            );
        }
    }

    #[test]
    fn parse_ask_user_questions_extracts_options_and_appends_other() {
        let tool_input = json!({
            "questions": [{
                "header": "学习方向",
                "multiSelect": false,
                "question": "你最想从哪个方向开始学习 Agent?",
                "options": [
                    { "label": "从零构建 Agent", "description": "手写 Agent 循环" },
                    { "label": "使用 Agent 框架", "description": "LangChain/CrewAI" }
                ]
            }]
        });
        let questions = parse_ask_user_questions(&tool_input);
        assert_eq!(questions.len(), 1);
        let question = &questions[0];
        assert_eq!(question.header, "学习方向");
        assert!(!question.multi_select);
        assert_eq!(question.options.len(), 3);
        assert_eq!(question.options[0].label, "从零构建 Agent");
        assert!(!question.options[0].is_other);
        assert!(question.options[2].is_other);
    }

    #[test]
    fn parse_ask_user_questions_rejects_question_without_options() {
        let tool_input = json!({ "questions": [{ "question": "Q", "options": [] }] });
        assert!(parse_ask_user_questions(&tool_input).is_empty());
        assert!(parse_ask_user_questions(&json!({})).is_empty());
    }

    #[test]
    fn question_submit_answers_through_updated_input() {
        let tool_input = json!({ "questions": [{ "question": "Pick", "options": [] }] });
        let questions = vec![ChoiceQuestion {
            header: String::new(),
            question: "Pick".to_string(),
            multi_select: true,
            options: vec![
                ChoiceOption {
                    label: "A".to_string(),
                    description: String::new(),
                    is_other: false,
                },
                ChoiceOption {
                    label: "B".to_string(),
                    description: String::new(),
                    is_other: false,
                },
                ChoiceOption {
                    label: "Other...".to_string(),
                    description: String::new(),
                    is_other: true,
                },
            ],
        }];
        let mut choice = build_choice(questions, ChoiceKind::AskUserQuestion);
        choice.tool_input = tool_input.clone();

        let response = question_request_response(
            &choice,
            ChoiceDecision::Submit {
                selected: vec![vec![0, 2]],
                other_text: vec![" custom answer ".to_string()],
            },
        );

        let decision = &response["hookSpecificOutput"]["decision"];
        assert_eq!(
            response["hookSpecificOutput"]["hookEventName"].as_str(),
            Some("PermissionRequest")
        );
        assert_eq!(decision["behavior"].as_str(), Some("allow"));
        assert_eq!(
            decision["updatedInput"]["answers"]["Pick"].as_str(),
            Some("A, custom answer")
        );
        // The original questions survive in the updated input.
        assert_eq!(
            decision["updatedInput"]["questions"],
            tool_input["questions"]
        );
        assert!(response.get("continue").is_none());
    }

    #[test]
    fn question_deny_falls_back_to_terminal_without_stopping_turn() {
        let choice = build_choice(Vec::new(), ChoiceKind::AskUserQuestion);
        let response = question_request_response(&choice, ChoiceDecision::Deny);
        assert!(response.get("continue").is_none());
        assert_eq!(
            response["hookSpecificOutput"]["decision"]["behavior"].as_str(),
            Some("deny")
        );
        assert!(
            response["hookSpecificOutput"]["decision"]
                .get("interrupt")
                .is_none()
        );

        let ignored = question_request_response(&choice, ChoiceDecision::Ignore);
        assert_eq!(ignored, json!({}));
    }

    #[test]
    fn post_tool_use_sweeps_pending_choice_in_same_session() {
        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = state.lock().expect("state poisoned");
            let mut choice = build_choice(Vec::new(), ChoiceKind::AskUserQuestion);
            choice.session_id = "s1".to_string();
            state.pending_choices.push_back(choice);
            let mut other = build_choice(Vec::new(), ChoiceKind::AskUserQuestion);
            other.id = 2;
            other.session_id = "s2".to_string();
            state.pending_choices.push_back(other);
        }

        process_hook(
            json!({
                "hook_event_name": "PostToolUse",
                "session_id": "s1",
                "tool_name": "Read",
                "tool_use_id": "read-1"
            }),
            state.clone(),
        );

        let state = state.lock().expect("state poisoned");
        assert_eq!(state.pending_choices.len(), 1);
        assert_eq!(state.pending_choices[0].session_id, "s2");
    }

    #[test]
    fn question_permission_request_registers_choice_and_closed_connection_dismisses() {
        use std::net::{TcpListener, TcpStream as StdTcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client = StdTcpStream::connect(addr).expect("connect");
        let (server_side, _) = listener.accept().expect("accept");

        let state = Arc::new(Mutex::new(AppState::new()));
        let worker = {
            let state = state.clone();
            std::thread::spawn(move || {
                process_hook_on_connection(
                    json!({
                        "hook_event_name": "PermissionRequest",
                        "session_id": "s1",
                        "tool_name": "AskUserQuestion",
                        "tool_input": {
                            "questions": [{
                                "question": "Q",
                                "options": [{ "label": "A" }]
                            }]
                        }
                    }),
                    state,
                    Some(&server_side),
                )
            })
        };

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            {
                let state = state.lock().expect("state poisoned");
                if !state.pending_choices.is_empty() {
                    assert!(state.pending_permissions.is_empty());
                    break;
                }
            }
            assert!(Instant::now() < deadline, "choice never registered");
            std::thread::sleep(Duration::from_millis(10));
        }

        // Claude Code answering in the terminal aborts the hook request.
        drop(client);

        let response = worker.join().expect("question hook thread");
        assert_eq!(response, json!({}));
        let state = state.lock().expect("state poisoned");
        assert!(state.pending_choices.is_empty());
    }

    #[test]
    fn summarize_tool_input_renders_plan_as_markdown() {
        let input = json!({ "plan": "# Title\n\n- step one" });
        let summary = summarize_tool_input(&input).unwrap();
        assert_eq!(summary, "# Title\n\n- step one");
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

    // Plan approval (ExitPlanMode): the plan field is already markdown.
    if let Some(plan) = string_field(tool_input, "plan") {
        return Some(shorten_block(&plan, PAYLOAD_SUMMARY_MAX_CHARS));
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
