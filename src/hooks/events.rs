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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HookActivity {
    Idle,
    Thinking,
    StartTool(PetMood),
    FinishTool(PetMood),
    FinishToolBatch,
    Error,
    StartSubagent,
    FinishSubagent,
    Notification,
    Done,
    EndSession,
}

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

        if let Some(activity) = hook_activity(event.as_str(), &tool_name, &payload) {
            apply_hook_activity(&mut state, activity, &payload);
        }

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
        state.set_resting_mood(
            permission_visual_mood(&payload, &permission.tool_name),
            true,
        );
        state.push_event("PermissionRequest", permission.tool_name.clone());
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
            let mood = match decision {
                Some(PermissionDecision::Deny) | None => PetMood::Error,
                Some(PermissionDecision::Ignore) => state.activity_mood().unwrap_or(PetMood::Idle),
                _ => state.activity_mood().unwrap_or(PetMood::Happy),
            };
            state.set_resting_mood(mood, matches!(mood, PetMood::Error));
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
        state.set_resting_mood(PetMood::Thinking, true);
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
            let mood = match decision {
                Some(ChoiceDecision::Submit(_)) => state.activity_mood().unwrap_or(PetMood::Happy),
                Some(ChoiceDecision::Deny) => PetMood::Thinking,
                Some(ChoiceDecision::Ignore) | None => {
                    state.activity_mood().unwrap_or(PetMood::Idle)
                }
            };
            state.set_resting_mood(mood, matches!(mood, PetMood::Thinking));
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

fn hook_activity(event: &str, tool_name: &str, payload: &Value) -> Option<HookActivity> {
    let tool_mood = mood_for_tool_use(tool_name, payload);
    match event {
        "SessionStart" | "SessionResume" => Some(HookActivity::Idle),
        "UserPromptSubmit" => Some(HookActivity::Thinking),
        "PreToolUse" => Some(HookActivity::StartTool(tool_mood)),
        "PostToolUse" => Some(HookActivity::FinishTool(tool_mood)),
        "PostToolBatch" => Some(HookActivity::FinishToolBatch),
        "PostToolUseFailure" | "StopFailure" | "PermissionDenied" => Some(HookActivity::Error),
        "SubagentStart" | "TaskCreated" => Some(HookActivity::StartSubagent),
        "SubagentStop" | "TaskCompleted" => Some(HookActivity::FinishSubagent),
        "PreCompact" => Some(HookActivity::Thinking),
        "PostCompact" => Some(HookActivity::Done),
        "Notification" | "Elicitation" => Some(HookActivity::Notification),
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
            state.finish_all_tools();
        }
        HookActivity::Error => {
            state.clear_activity();
            state.last_error = summarize_payload(payload);
            state.set_resting_mood(PetMood::Error, true);
        }
        HookActivity::StartSubagent => {
            state.start_subagent();
        }
        HookActivity::FinishSubagent => {
            state.finish_subagent();
        }
        HookActivity::Notification => {
            state.set_resting_mood(PetMood::Thinking, true);
        }
        HookActivity::Done => {
            state.clear_activity();
            state.set_resting_mood(PetMood::Happy, false);
        }
        HookActivity::EndSession => {
            state.clear_activity();
            state.set_resting_mood(PetMood::Idle, false);
        }
    }
}

fn mood_for_tool(tool_name: &str) -> PetMood {
    let normalized = tool_name.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "task" => PetMood::Thinking,
        "bash" | "shell" => PetMood::Building,
        "edit" | "multiedit" | "write" | "notebookedit" => PetMood::Typing,
        "read" | "grep" | "glob" | "ls" | "webfetch" | "websearch" | "todoread" | "todowrite"
        | "askuserquestion" | "exitplanmode" => PetMood::Thinking,
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
        _ => PetMood::Thinking,
    }
}

fn mood_for_tool_use(tool_name: &str, payload: &Value) -> PetMood {
    if !tool_name.trim().eq_ignore_ascii_case("Task") {
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
    PetMood::Thinking
}

fn permission_visual_mood(payload: &Value, fallback_tool_name: &str) -> PetMood {
    let tool_name = string_field(payload, "tool_name")
        .or_else(|| string_field(payload, "toolName"))
        .unwrap_or_else(|| fallback_tool_name.to_string());
    let mood = mood_for_tool_use(&tool_name, payload);
    match mood {
        PetMood::Typing | PetMood::Building => mood,
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
    let session_id = string_field(payload, "session_id").unwrap_or_else(|| "default".to_string());
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
        "\u{7814}\u{7a76}",
        "\u{5206}\u{6790}",
        "\u{641c}\u{7d22}",
        "\u{8c03}\u{67e5}",
        "\u{89c4}\u{5212}",
        "subagent",
        "agent",
        "delegate",
        "parallel",
        "research",
        "analyze",
        "analyse",
        "search",
        "investigate",
        "planning",
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
        assert_eq!(mood_for_tool("Bash"), PetMood::Building);
        assert_eq!(mood_for_tool("Edit"), PetMood::Typing);
        assert_eq!(
            mood_for_tool("mcp__filesystem__write_file"),
            PetMood::Typing
        );
        assert_eq!(mood_for_tool("shell_command"), PetMood::Building);
        assert_eq!(mood_for_tool("agent_worker"), PetMood::Thinking);
        assert_eq!(mood_for_tool("Read"), PetMood::Thinking);
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
            hook_activity("PostToolUse", "Task", &json!({})),
            Some(HookActivity::FinishTool(PetMood::Thinking))
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
            PetMood::Thinking
        );
        assert_eq!(
            permission_visual_mood(&json!({ "tool_name": "UnknownTool" }), ""),
            PetMood::Thinking
        );
    }

    #[test]
    fn pending_permission_does_not_force_permission_gif() {
        let mut state = AppState::new();
        state.pending_permissions.push_back(PendingPermission {
            id: 1,
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
    fn session_end_clears_activity_and_returns_idle() {
        let mut state = AppState::new();
        state.start_tool_mood(PetMood::Typing);
        state.start_subagent();
        assert_eq!(state.active_tools, 1);
        assert_eq!(state.active_subagents, 1);

        apply_hook_activity(&mut state, HookActivity::EndSession, &json!({}));

        assert_eq!(state.active_tools, 0);
        assert_eq!(state.active_subagents, 0);
        assert_eq!(state.resting_mood, PetMood::Idle);
        assert_ne!(state.mood, PetMood::Sleeping);
    }

    #[test]
    fn stop_still_sets_happy() {
        let mut state = AppState::new();

        apply_hook_activity(&mut state, HookActivity::Done, &json!({}));

        assert_eq!(state.resting_mood, PetMood::Happy);
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
    fn permission_interrupts_work_and_returns_to_active_tool() {
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

        state.set_mood(PetMood::Permission);
        assert_eq!(state.mood, PetMood::Permission);
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
