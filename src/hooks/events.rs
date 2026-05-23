use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::app::{
    AppState, ChoiceDecision, ChoiceKind, ChoiceOption, ChoiceQuestion, ChoiceWaiter,
    PendingChoice, PendingPermission, PermissionDecision, PermissionWaiter, PetMood, SessionInfo,
};
use crate::config::PERMISSION_WAIT;
use crate::globals::APP_STATE;
use crate::util::shorten;

use super::quota::{scan_transcript_usage, update_quota_from_value};

pub(crate) fn process_hook(payload: Value, state: Arc<Mutex<AppState>>) -> Value {
    let event = string_field(&payload, "hook_event_name")
        .or_else(|| string_field(&payload, "hookEventName"))
        .unwrap_or_else(|| "unknown".to_string());
    let session_id = string_field(&payload, "session_id").unwrap_or_else(|| "default".to_string());
    let cwd = string_field(&payload, "cwd").unwrap_or_default();
    let tool_name = string_field(&payload, "tool_name")
        .or_else(|| string_field(&payload, "toolName"))
        .unwrap_or_default();

    if event == "PermissionRequest" {
        return handle_permission_request(payload, state, session_id, cwd, tool_name);
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
        if clears_all_pending_interactions(event.as_str()) {
            clear_all_interactions(&mut state);
        } else if clears_pending_interaction(event.as_str()) {
            clear_stale_interactions(&mut state, &session_id);
        }
        state.sessions.insert(
            session_id,
            SessionInfo {
                last_event: event.clone(),
                cwd: cwd.clone(),
                updated_at: Instant::now(),
            },
        );
        update_quota_from_value(&mut state.quota, &payload);
        if let Some(path) = transcript_path.as_deref() {
            state.quota.transcript_path = path.to_string();
        }

        apply_state_event(
            &mut state,
            event.as_str(),
            &payload,
            &tool_name,
            transcript_path.as_deref(),
        );

        let detail = if tool_name.is_empty() {
            summarize_payload(&payload)
        } else {
            tool_name
        };
        state.push_event(event.clone(), detail);
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
        }
    }

    json!({})
}

fn apply_state_event(
    state: &mut AppState,
    event: &str,
    payload: &Value,
    tool_name: &str,
    transcript_path: Option<&str>,
) {
    match event {
        "UserPromptSubmit" | "SessionStart" | "SessionResume" => {
            state.set_mood(PetMood::Thinking);
            state.show_speech(
                "Claude is thinking",
                "New work session",
                Duration::from_secs(4),
                3,
            );
        }
        "PreToolUse" => {
            let mood = mood_for_event(event, tool_name, payload);
            state.start_tool_mood(mood);
            if mood == PetMood::Subagent {
                state.active_subagents = state.active_subagents.saturating_add(1);
            }
        }
        "PostToolUse" => {
            let mood = mood_for_event("PreToolUse", tool_name, payload);
            state.finish_tool_mood(mood);
            if mood == PetMood::Subagent {
                state.active_subagents = state.active_subagents.saturating_sub(1);
            }
            state.set_mood(state.activity_mood().unwrap_or(PetMood::Happy));
        }
        "PostToolBatch" => {
            state.finish_all_tools();
            state.set_mood(state.activity_mood().unwrap_or(PetMood::Happy));
        }
        "PostToolUseFailure" | "StopFailure" | "PermissionDenied" => {
            state.finish_tool_mood(mood_for_event("PreToolUse", tool_name, payload));
            state.last_error = summarize_payload(payload);
            state.set_mood(PetMood::Error);
            state.show_speech(
                "Blocked",
                state.last_error.clone(),
                Duration::from_secs(5),
                6,
            );
        }
        "SubagentStart" | "TaskCreated" => {
            state.active_subagents = state.active_subagents.saturating_add(1);
            state.set_mood(PetMood::Subagent);
            state.show_speech(
                "Subagent started",
                "Parallel work is running",
                Duration::from_secs(4),
                4,
            );
        }
        "SubagentStop" | "TaskCompleted" => {
            state.active_subagents = state.active_subagents.saturating_sub(1);
            state.set_mood(state.activity_mood().unwrap_or(PetMood::Happy));
        }
        "Notification" | "Elicitation" => state.set_mood(PetMood::Thinking),
        "PreCompact" => state.set_mood(PetMood::Building),
        "PostCompact" => state.set_mood(PetMood::Happy),
        "Stop" | "SessionEnd" => {
            state.clear_activity();
            if event == "SessionEnd" && string_field(payload, "source").as_deref() == Some("clear")
            {
                state.set_mood(PetMood::Building);
                state.show_speech(
                    "Context cleared",
                    "Preparing a fresh session",
                    Duration::from_secs(3),
                    4,
                );
            } else {
                state.set_mood(PetMood::Happy);
                state.show_speech("Done", "Claude Code finished", Duration::from_secs(4), 4);
            }
            if let Some(path) = transcript_path {
                state.quota.transcript_path = path.to_string();
            }
        }
        _ => {}
    }
}

fn handle_permission_request(
    payload: Value,
    state: Arc<Mutex<AppState>>,
    session_id: String,
    cwd: String,
    tool_name: String,
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
        let permission = PendingPermission {
            id,
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
        state.set_mood(PetMood::Permission);
        state.push_event("PermissionRequest", permission.tool_name.clone());
        state.show_speech(
            "Permission request",
            format!("{} wants access", permission.tool_name),
            Duration::from_secs(5),
            9,
        );
        update_quota_from_value(&mut state.quota, &payload);
        permission
    };

    #[cfg(not(windows))]
    {
        let mut state = state.lock().expect("state poisoned");
        state
            .pending_permissions
            .retain(|pending| pending.id != permission.id);
        state.last_error = "Permission UI is not available on this platform.".to_string();
        state.set_mood(PetMood::Error);
        return permission_response(PermissionDecision::Deny, &permission);
    }

    let decision = {
        let deadline = Instant::now() + PERMISSION_WAIT;
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
            let now = Instant::now();
            if now >= deadline {
                break None;
            }
            let wait_for = (deadline - now).min(Duration::from_millis(250));
            let (next_guard, _) = waiter
                .ready
                .wait_timeout(guard, wait_for)
                .expect("permission waiter poisoned");
            guard = next_guard;
        }
    };

    {
        let mut state = state.lock().expect("state poisoned");
        state
            .pending_permissions
            .retain(|pending| pending.id != permission.id);
        if state.pending_permissions.is_empty() {
            state.set_mood(match decision {
                Some(PermissionDecision::Deny) | None => PetMood::Error,
                Some(PermissionDecision::Ignore) => PetMood::Idle,
                _ => PetMood::Happy,
            });
        }
        match decision {
            Some(PermissionDecision::AllowAlways) => {
                state.show_speech(
                    "Always allowed",
                    permission.tool_name.clone(),
                    Duration::from_secs(4),
                    6,
                );
            }
            Some(PermissionDecision::AllowOnce) => {
                state.show_speech(
                    "Allowed",
                    permission.tool_name.clone(),
                    Duration::from_secs(3),
                    5,
                );
            }
            Some(PermissionDecision::Deny) | None => {
                state.show_speech(
                    "Denied",
                    permission.tool_name.clone(),
                    Duration::from_secs(4),
                    6,
                );
            }
            Some(PermissionDecision::Ignore) => {}
        }
    }

    permission_response(decision.unwrap_or(PermissionDecision::Ignore), &permission)
}

fn permission_response(decision: PermissionDecision, permission: &PendingPermission) -> Value {
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

fn handle_interactive_pre_tool_use(
    payload: &Value,
    state: Arc<Mutex<AppState>>,
    session_id: &str,
    _cwd: &str,
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
        let selected = vec![Vec::new(); questions.len()];
        let choice = PendingChoice {
            id,
            session_id,
            kind,
            title,
            detail,
            questions,
            selected,
            tool_input,
            waiter: waiter.clone(),
        };
        state.pending_choices.push_back(choice.clone());
        state.set_mood(PetMood::Permission);
        state.push_event(
            "PreToolUse",
            match kind {
                ChoiceKind::AskUserQuestion => "AskUserQuestion",
                ChoiceKind::ExitPlanMode => "ExitPlanMode",
            },
        );
        update_quota_from_value(&mut state.quota, &json!({}));
        choice
    };

    let decision = {
        let guard = waiter.decision.lock().expect("choice waiter poisoned");
        let (guard, _) = waiter
            .ready
            .wait_timeout_while(guard, PERMISSION_WAIT, |decision| decision.is_none())
            .expect("choice waiter poisoned");
        guard.clone()
    };

    {
        let mut state = state.lock().expect("state poisoned");
        state
            .pending_choices
            .retain(|pending| pending.id != choice.id);
        if state.pending_permissions.is_empty() && state.pending_choices.is_empty() {
            state.set_mood(match decision {
                Some(ChoiceDecision::Submit(_)) => PetMood::Happy,
                Some(ChoiceDecision::Deny) => PetMood::Thinking,
                Some(ChoiceDecision::Ignore) | None => PetMood::Idle,
            });
        }
    }

    choice_response(&choice, decision.unwrap_or(ChoiceDecision::Ignore))
}

fn choice_response(choice: &PendingChoice, decision: ChoiceDecision) -> Value {
    match (choice.kind, decision) {
        (ChoiceKind::AskUserQuestion, ChoiceDecision::Submit(selected)) => {
            let mut updated_input = choice.tool_input.clone();
            if let Some(obj) = updated_input.as_object_mut() {
                let mut answers = serde_json::Map::new();
                for (question_index, question) in choice.questions.iter().enumerate() {
                    let answer = selected
                        .get(question_index)
                        .into_iter()
                        .flat_map(|items| items.iter())
                        .filter_map(|option_index| question.options.get(*option_index))
                        .map(|option| option.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    answers.insert(question.question.clone(), Value::String(answer));
                }
                obj.insert("answers".to_string(), Value::Object(answers));
            }
            pre_tool_allow(updated_input, "Answered in claudie")
        }
        (ChoiceKind::ExitPlanMode, ChoiceDecision::Submit(selected)) => {
            if selected.first().is_some_and(|items| items.contains(&0)) {
                pre_tool_allow(choice.tool_input.clone(), "Plan approved in claudie")
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
            let options = question
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flat_map(|options| options.iter())
                .filter_map(|option| {
                    let label = string_field(option, "label")?;
                    let description = string_field(option, "description").unwrap_or_default();
                    Some(ChoiceOption { label, description })
                })
                .collect::<Vec<_>>();
            if options.is_empty() {
                return None;
            }
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
            },
            ChoiceOption {
                label: "Keep planning".to_string(),
                description: "Reject for now and continue refining the plan.".to_string(),
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
    let mut detail = if path.is_empty() {
        shorten(&plan, 360)
    } else {
        format!("{}\n{}", shorten(&plan, 360), path)
    };
    if let Some(prompts) = tool_input.get("allowedPrompts").and_then(Value::as_array) {
        for prompt in prompts.iter().take(4) {
            let tool = string_field(prompt, "tool").unwrap_or_else(|| "Tool".to_string());
            let text = string_field(prompt, "prompt").unwrap_or_default();
            detail.push_str(&format!("\nAllow {tool}: {}", shorten(&text, 54)));
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
            state.pending_permissions.pop_front()
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
        let Some(choice) = state.pending_choices.front_mut() else {
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
    decide_current_choice(ChoiceDecision::Submit(Vec::new()));
}

pub(crate) fn deny_current_choice() {
    decide_current_choice(ChoiceDecision::Deny);
}

fn decide_current_choice(decision: ChoiceDecision) {
    if let Some(state) = APP_STATE.get() {
        let pending = {
            let mut state = state.lock().expect("state poisoned");
            let Some(choice) = state.pending_choices.front() else {
                return;
            };
            let is_submit = matches!(&decision, ChoiceDecision::Submit(_));
            if is_submit && choice.selected.iter().any(Vec::is_empty) {
                return;
            }
            let selected = choice.selected.clone();
            let decision = match decision {
                ChoiceDecision::Submit(_) => ChoiceDecision::Submit(selected),
                other => other,
            };
            state
                .pending_choices
                .pop_front()
                .map(|pending| (pending, decision))
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

fn clear_all_interactions(state: &mut AppState) {
    let stale_permissions = state.pending_permissions.drain(..).collect::<Vec<_>>();
    for pending in stale_permissions {
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

    let stale_choices = state.pending_choices.drain(..).collect::<Vec<_>>();
    for pending in stale_choices {
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

fn clears_all_pending_interactions(event: &str) -> bool {
    matches!(
        event,
        "PermissionDenied" | "PostToolUseFailure" | "Stop" | "SessionEnd" | "StopFailure"
    )
}

fn clears_pending_interaction(event: &str) -> bool {
    matches!(event, "PreToolUse" | "PostToolUse" | "PostToolBatch")
}

fn mood_for_event(event: &str, tool_name: &str, payload: &Value) -> PetMood {
    if event == "PreToolUse" && tool_name == "Task" {
        return PetMood::Subagent;
    }
    if event == "SessionEnd" && string_field(payload, "source").as_deref() == Some("clear") {
        return PetMood::Building;
    }
    mood_for_tool(tool_name)
}

fn mood_for_tool(tool_name: &str) -> PetMood {
    match tool_name {
        "Task" => PetMood::Subagent,
        "Bash" | "Shell" => PetMood::Building,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => PetMood::Typing,
        "Read" | "Grep" | "Glob" | "LS" | "WebFetch" | "WebSearch" | "TodoRead" | "TodoWrite"
        | "AskUserQuestion" | "ExitPlanMode" => PetMood::Thinking,
        _ => PetMood::Thinking,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_map_to_work_moods() {
        assert_eq!(mood_for_tool("Task"), PetMood::Subagent);
        assert_eq!(mood_for_tool("Bash"), PetMood::Building);
        assert_eq!(mood_for_tool("Edit"), PetMood::Typing);
        assert_eq!(mood_for_tool("Read"), PetMood::Thinking);
        assert_eq!(mood_for_tool("AskUserQuestion"), PetMood::Thinking);
    }

    #[test]
    fn pre_tool_task_maps_to_subagent() {
        assert_eq!(
            mood_for_event("PreToolUse", "Task", &json!({})),
            PetMood::Subagent
        );
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
        assert_eq!(state.mood, PetMood::Thinking);
    }

    #[test]
    fn subagent_has_priority_over_regular_tools() {
        let mut state = AppState::new();

        state.start_tool_mood(PetMood::Building);
        state.active_subagents = 1;

        assert_eq!(state.activity_mood(), Some(PetMood::Subagent));
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
        if let Some(command) = string_field(tool_input, "command") {
            return shorten(&command, 90);
        }
        for key in ["file_path", "path", "pattern", "url", "prompt"] {
            if let Some(text) = string_field(tool_input, key) {
                return shorten(&text, 90);
            }
        }
    }
    if let Some(message) = string_field(value, "message")
        .or_else(|| string_field(value, "reason"))
        .or_else(|| string_field(value, "notification"))
    {
        return shorten(&message, 90);
    }
    shorten(&value.to_string(), 90)
}
