use std::cell::RefCell;

use slint::{ComponentHandle, SharedString};

use crate::app::{PendingChoice, PendingPermission, PermissionDecision};
use crate::globals::APP_STATE;
use crate::hooks::{
    decide_current_permission, deny_current_choice, submit_current_choice,
    toggle_current_choice_option,
};
use crate::ui::slint_views::PromptWindow;
use crate::ui::window_icon::{apply_slint_window_icons, schedule_prompt_window_icon_refresh};

thread_local! {
    static PROMPT: RefCell<Option<PromptWindow>> = const { RefCell::new(None) };
    static PROMPT_OPTIONS: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn sync_prompt_popup() {
    let snapshot = prompt_snapshot();
    PROMPT.with(|slot| {
        if snapshot.is_none() {
            if let Some(window) = slot.borrow_mut().take() {
                let _ = window.hide();
            }
            PROMPT_OPTIONS.with(|options| options.borrow_mut().clear());
            return;
        }

        let snapshot = snapshot.expect("checked above");
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let Ok(window) = PromptWindow::new() else {
                return;
            };
            wire_prompt_callbacks(&window);
            *slot = Some(window);
        }
        if let Some(window) = slot.as_ref() {
            apply_prompt_snapshot(window, &snapshot);
            let _ = window.show();
            apply_slint_window_icons(window.window());
            schedule_prompt_window_icon_refresh(window.as_weak());
        }
    });
}

pub(crate) fn close_prompt_popup() {
    PROMPT.with(|slot| {
        if let Some(window) = slot.borrow_mut().take() {
            let _ = window.hide();
        }
    });
    PROMPT_OPTIONS.with(|options| options.borrow_mut().clear());
}
fn wire_prompt_callbacks(window: &PromptWindow) {
    window.on_allow_once(|| {
        decide_current_permission(PermissionDecision::AllowOnce);
        close_prompt_popup();
    });
    window.on_allow_always(|| {
        decide_current_permission(PermissionDecision::AllowAlways);
        close_prompt_popup();
    });
    window.on_deny(|| {
        decide_current_permission(PermissionDecision::Deny);
        close_prompt_popup();
    });
    window.on_submit_choice(|| {
        submit_current_choice();
        close_prompt_popup();
    });
    window.on_cancel_choice(|| {
        deny_current_choice();
        close_prompt_popup();
    });
    window.on_toggle_option0(|| toggle_prompt_option(0));
    window.on_toggle_option1(|| toggle_prompt_option(1));
    window.on_toggle_option2(|| toggle_prompt_option(2));
    window.on_toggle_option3(|| toggle_prompt_option(3));
    window.on_toggle_option4(|| toggle_prompt_option(4));
    window.on_toggle_option5(|| toggle_prompt_option(5));
    window.on_toggle_option6(|| toggle_prompt_option(6));
    window.on_toggle_option7(|| toggle_prompt_option(7));
    window.window().on_close_requested(|| {
        close_prompt_popup();
        slint::CloseRequestResponse::HideWindow
    });
}

fn toggle_prompt_option(index: usize) {
    let target = PROMPT_OPTIONS.with(|options| options.borrow().get(index).copied());
    if let Some((question_index, option_index)) = target {
        toggle_current_choice_option(question_index, option_index);
        sync_prompt_popup();
    }
}

struct PromptSnapshot {
    title: String,
    subtitle: String,
    detail: String,
    meta: String,
    is_choice: bool,
    submit_enabled: bool,
    options: Vec<String>,
    option_targets: Vec<(usize, usize)>,
}

fn prompt_snapshot() -> Option<PromptSnapshot> {
    let state = APP_STATE.get()?;
    let state = state.lock().expect("state poisoned");
    if let Some(choice) = state.pending_choices.front() {
        return Some(choice_snapshot(choice));
    }
    state.pending_permissions.front().map(permission_snapshot)
}

fn permission_snapshot(permission: &PendingPermission) -> PromptSnapshot {
    PromptSnapshot {
        title: "Permission request".to_string(),
        subtitle: format!("{} wants access", permission.tool_name.trim()),
        detail: permission.summary.clone(),
        meta: prompt_meta(&permission.session_id, &permission.cwd),
        is_choice: false,
        submit_enabled: false,
        options: Vec::new(),
        option_targets: Vec::new(),
    }
}

fn choice_snapshot(choice: &PendingChoice) -> PromptSnapshot {
    let mut detail = choice.detail.trim().to_string();
    if detail.is_empty() {
        detail = choice
            .questions
            .first()
            .map(|question| question.question.clone())
            .unwrap_or_default();
    }

    let mut options = Vec::new();
    let mut option_targets = Vec::new();
    for (question_index, question) in choice.questions.iter().enumerate() {
        for (option_index, option) in question.options.iter().enumerate() {
            if options.len() >= 8 {
                break;
            }
            let selected = choice
                .selected
                .get(question_index)
                .is_some_and(|items| items.contains(&option_index));
            let marker = if selected { "[x]" } else { "[ ]" };
            let label = if question.header.trim().is_empty() {
                option.label.clone()
            } else {
                format!("{}: {}", question.header, option.label)
            };
            options.push(format!("{marker} {label}"));
            option_targets.push((question_index, option_index));
        }
    }

    PromptSnapshot {
        title: choice.title.clone(),
        subtitle: "Claude Code is asking for input.".to_string(),
        detail,
        meta: prompt_meta(&choice.session_id, ""),
        is_choice: true,
        submit_enabled: choice.selected.iter().all(|items| !items.is_empty()),
        options,
        option_targets,
    }
}

fn apply_prompt_snapshot(window: &PromptWindow, snapshot: &PromptSnapshot) {
    window.set_is_choice(snapshot.is_choice);
    window.set_title_text(shared(&snapshot.title));
    window.set_subtitle_text(shared(&snapshot.subtitle));
    window.set_detail_text(shared(&snapshot.detail));
    window.set_meta_text(shared(&snapshot.meta));
    window.set_submit_enabled(snapshot.submit_enabled);
    set_prompt_option(window, 0, snapshot.options.first());
    set_prompt_option(window, 1, snapshot.options.get(1));
    set_prompt_option(window, 2, snapshot.options.get(2));
    set_prompt_option(window, 3, snapshot.options.get(3));
    set_prompt_option(window, 4, snapshot.options.get(4));
    set_prompt_option(window, 5, snapshot.options.get(5));
    set_prompt_option(window, 6, snapshot.options.get(6));
    set_prompt_option(window, 7, snapshot.options.get(7));
    PROMPT_OPTIONS.with(|options| {
        *options.borrow_mut() = snapshot.option_targets.clone();
    });
}

fn set_prompt_option(window: &PromptWindow, index: usize, label: Option<&String>) {
    let visible = label.is_some();
    let label = shared(label.map(String::as_str).unwrap_or_default());
    match index {
        0 => {
            window.set_option0_visible(visible);
            window.set_option0_text(label);
        }
        1 => {
            window.set_option1_visible(visible);
            window.set_option1_text(label);
        }
        2 => {
            window.set_option2_visible(visible);
            window.set_option2_text(label);
        }
        3 => {
            window.set_option3_visible(visible);
            window.set_option3_text(label);
        }
        4 => {
            window.set_option4_visible(visible);
            window.set_option4_text(label);
        }
        5 => {
            window.set_option5_visible(visible);
            window.set_option5_text(label);
        }
        6 => {
            window.set_option6_visible(visible);
            window.set_option6_text(label);
        }
        7 => {
            window.set_option7_visible(visible);
            window.set_option7_text(label);
        }
        _ => {}
    }
}

fn prompt_meta(session_id: &str, cwd: &str) -> String {
    let session = if session_id.trim().is_empty() {
        "session unknown".to_string()
    } else {
        format!("session {}", session_id.chars().take(8).collect::<String>())
    };
    if cwd.trim().is_empty() {
        session
    } else {
        format!("{session}    {cwd}")
    }
}

fn shared(value: &str) -> SharedString {
    SharedString::from(value)
}
