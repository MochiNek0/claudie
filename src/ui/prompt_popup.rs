use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::{PendingChoice, PendingPermission, PermissionDecision};
use crate::globals::APP_STATE;
use crate::hooks::{
    decide_current_permission, deny_current_choice, set_current_choice_other_text,
    submit_current_choice, toggle_current_choice_option,
};
use crate::ui::slint_views::{ChoiceOptionData, PromptWindow};
use crate::ui::window_icon::{apply_slint_window_icons, schedule_prompt_window_icon_refresh};

thread_local! {
    static PROMPT: RefCell<Option<PromptWindow>> = const { RefCell::new(None) };
    static PROMPT_OPTIONS: RefCell<Vec<OptionTarget>> = const { RefCell::new(Vec::new()) };
    static PROMPT_SNAPSHOT: RefCell<Option<PromptSnapshot>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy)]
struct OptionTarget {
    question_index: usize,
    option_index: usize,
    is_header: bool,
    is_other: bool,
}

pub(crate) fn sync_prompt_popup() {
    let snapshot = prompt_snapshot();
    PROMPT.with(|slot| {
        if snapshot.is_none() {
            if let Some(window) = slot.borrow_mut().take() {
                let _ = window.hide();
            }
            PROMPT_OPTIONS.with(|options| options.borrow_mut().clear());
            PROMPT_SNAPSHOT.with(|last| last.borrow_mut().take());
            return;
        }

        let snapshot = snapshot.expect("checked above");
        let mut slot = slot.borrow_mut();
        let mut created = false;
        if slot.is_none() {
            let Ok(window) = PromptWindow::new() else {
                return;
            };
            wire_prompt_callbacks(&window);
            *slot = Some(window);
            created = true;
        }
        if let Some(window) = slot.as_ref() {
            let should_apply = PROMPT_SNAPSHOT.with(|last| {
                let mut last = last.borrow_mut();
                if created || last.as_ref() != Some(&snapshot) {
                    *last = Some(snapshot.clone());
                    true
                } else {
                    false
                }
            });
            if should_apply {
                apply_prompt_snapshot(window, &snapshot);
            }
            if created {
                let _ = window.show();
                apply_slint_window_icons(window.window());
                schedule_prompt_window_icon_refresh(window.as_weak());
            }
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
    PROMPT_SNAPSHOT.with(|last| last.borrow_mut().take());
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
    window.on_toggle_option(|index| {
        let target = PROMPT_OPTIONS.with(|options| options.borrow().get(index as usize).copied());
        if let Some(target) = target {
            if target.is_header {
                return;
            }
            toggle_current_choice_option(target.question_index, target.option_index);
            sync_prompt_popup();
        }
    });
    window.on_set_other_text(|index, text| {
        let target = PROMPT_OPTIONS.with(|options| options.borrow().get(index as usize).copied());
        if let Some(target) = target {
            if !target.is_other {
                return;
            }
            set_current_choice_other_text(target.question_index, text.to_string());
            refresh_submit_state();
            remember_current_snapshot();
        }
    });
    window.window().on_close_requested(|| {
        close_prompt_popup();
        slint::CloseRequestResponse::HideWindow
    });
}

#[derive(Clone, PartialEq, Eq)]
struct PromptSnapshot {
    title: String,
    subtitle: String,
    detail: String,
    meta: String,
    is_choice: bool,
    submit_enabled: bool,
    submit_hint: String,
    options: Vec<OptionView>,
}

#[derive(Clone, PartialEq, Eq)]
struct OptionView {
    question_index: usize,
    option_index: usize,
    label: String,
    description: String,
    selected: bool,
    is_other: bool,
    other_text: String,
    multi_select: bool,
    is_question_header: bool,
    header: String,
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
        submit_hint: String::new(),
        options: Vec::new(),
    }
}

fn choice_snapshot(choice: &PendingChoice) -> PromptSnapshot {
    let mut detail = choice.detail.trim().to_string();
    if detail.is_empty() {
        detail = choice
            .questions
            .iter()
            .map(|question| question.question.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
    }

    let mut options = Vec::new();
    for (question_index, question) in choice.questions.iter().enumerate() {
        let header_label = if question.header.trim().is_empty() {
            String::new()
        } else {
            question.header.clone()
        };
        options.push(OptionView {
            question_index,
            option_index: 0,
            label: String::new(),
            description: question.question.clone(),
            selected: false,
            is_other: false,
            other_text: String::new(),
            multi_select: question.multi_select,
            is_question_header: true,
            header: header_label,
        });
        for (option_index, option) in question.options.iter().enumerate() {
            let selected = choice
                .selected
                .get(question_index)
                .is_some_and(|items| items.contains(&option_index));
            let other_text = if option.is_other {
                choice
                    .other_text
                    .get(question_index)
                    .cloned()
                    .unwrap_or_default()
            } else {
                String::new()
            };
            options.push(OptionView {
                question_index,
                option_index,
                label: option.label.clone(),
                description: option.description.clone(),
                selected,
                is_other: option.is_other,
                other_text,
                multi_select: question.multi_select,
                is_question_header: false,
                header: String::new(),
            });
        }
    }

    let submit_enabled = choice.is_submittable();
    let submit_hint = if submit_enabled {
        String::new()
    } else {
        submit_hint_for(choice)
    };

    PromptSnapshot {
        title: choice.title.clone(),
        subtitle: "Claude Code is asking for input.".to_string(),
        detail,
        meta: prompt_meta(&choice.session_id, ""),
        is_choice: true,
        submit_enabled,
        submit_hint,
        options,
    }
}

fn submit_hint_for(choice: &PendingChoice) -> String {
    let mut needs_other_text = false;
    for (qi, question) in choice.questions.iter().enumerate() {
        let Some(selected) = choice.selected.get(qi) else {
            return "Please answer every question before submitting.".to_string();
        };
        if selected.is_empty() {
            return "Please answer every question before submitting.".to_string();
        }
        for &oi in selected {
            if question
                .options
                .get(oi)
                .is_some_and(|option| option.is_other)
                && choice
                    .other_text
                    .get(qi)
                    .map(|text| text.trim().is_empty())
                    .unwrap_or(true)
            {
                needs_other_text = true;
            }
        }
    }
    if needs_other_text {
        "Please fill in the 'Other' answer before submitting.".to_string()
    } else {
        String::new()
    }
}

fn apply_prompt_snapshot(window: &PromptWindow, snapshot: &PromptSnapshot) {
    window.set_is_choice(snapshot.is_choice);
    window.set_title_text(shared(&snapshot.title));
    window.set_subtitle_text(shared(&snapshot.subtitle));
    window.set_detail_text(shared(&snapshot.detail));
    window.set_meta_text(shared(&snapshot.meta));
    window.set_submit_enabled(snapshot.submit_enabled);
    window.set_submit_hint(shared(&snapshot.submit_hint));

    let model_data: Vec<ChoiceOptionData> = snapshot
        .options
        .iter()
        .map(|opt| ChoiceOptionData {
            question_index: opt.question_index as i32,
            option_index: opt.option_index as i32,
            label: shared(&opt.label),
            description: shared(&opt.description),
            selected: opt.selected,
            is_other: opt.is_other,
            other_text: shared(&opt.other_text),
            multi_select: opt.multi_select,
            is_question_header: opt.is_question_header,
            header: shared(&opt.header),
        })
        .collect();
    let model: ModelRc<ChoiceOptionData> = ModelRc::from(Rc::new(VecModel::from(model_data)));
    window.set_options_model(model);

    PROMPT_OPTIONS.with(|options| {
        *options.borrow_mut() = snapshot
            .options
            .iter()
            .map(|opt| OptionTarget {
                question_index: opt.question_index,
                option_index: opt.option_index,
                is_header: opt.is_question_header,
                is_other: opt.is_other,
            })
            .collect();
    });
}

fn refresh_submit_state() {
    PROMPT.with(|slot| {
        let slot = slot.borrow();
        let Some(window) = slot.as_ref() else {
            return;
        };
        let Some(state) = APP_STATE.get() else {
            return;
        };
        let state = state.lock().expect("state poisoned");
        let Some(choice) = state.pending_choices.front() else {
            return;
        };
        let enabled = choice.is_submittable();
        let hint = if enabled {
            String::new()
        } else {
            submit_hint_for(choice)
        };
        window.set_submit_enabled(enabled);
        window.set_submit_hint(shared(&hint));
    });
}

fn remember_current_snapshot() {
    PROMPT_SNAPSHOT.with(|last| {
        *last.borrow_mut() = prompt_snapshot();
    });
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
