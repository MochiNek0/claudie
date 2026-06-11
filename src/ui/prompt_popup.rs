use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use windows_sys::Win32::Foundation::HWND;

use crate::app::{ChoiceKind, PendingChoice, PendingPermission, PermissionDecision};
use crate::globals::APP_STATE;
use crate::hooks::{
    decide_current_permission, deny_current_choice, set_current_choice_other_text,
    submit_current_choice, toggle_current_choice_option,
};
use crate::ui::slint_views::{ChoiceOptionData, DiffLine, MarkdownBlockData, PromptWindow};
use crate::ui::window_icon::{apply_slint_window_icons, schedule_prompt_window_icon_refresh};
use crate::ui::window_position::center_window_on_screen;
use crate::util::{MarkdownBlock, MarkdownBlockKind, estimate_wrapped_lines, markdown_blocks};

// Text widths in logical px available for wrapping. Keep in sync with the
// PromptWindow geometry in slint_views.rs (window width is fixed at 640).
const DETAIL_TEXT_PX: f32 = 530.0;
const DETAIL_CODE_TEXT_PX: f32 = 510.0;
const DETAIL_DIFF_TEXT_PX: f32 = 500.0;
const OPTION_TEXT_PX: f32 = 500.0;
const HEADER_TEXT_PX: f32 = 540.0;
const PROMPT_WINDOW_LOGICAL_SIZE: (f32, f32) = (640.0, 640.0);

thread_local! {
    static PROMPT: RefCell<Option<PromptWindow>> = const { RefCell::new(None) };
    static PROMPT_OPTIONS: RefCell<Vec<OptionTarget>> = const { RefCell::new(Vec::new()) };
    static PROMPT_SNAPSHOT: RefCell<Option<PromptSnapshot>> = const { RefCell::new(None) };
    // (is_choice, id) of the interaction last handled by the popup. The
    // per-tick sync skips the (markdown-parsing) snapshot rebuild while this
    // key is unchanged; UI callbacks that mutate the current interaction go
    // through the forced path instead.
    static PROMPT_KEY: Cell<Option<(bool, u64)>> = const { Cell::new(None) };
}

#[derive(Clone, Copy)]
struct OptionTarget {
    question_index: usize,
    option_index: usize,
    is_header: bool,
    is_other: bool,
}

pub(crate) fn sync_prompt_popup() {
    sync_prompt_popup_impl(std::ptr::null_mut(), true);
}

pub(crate) fn sync_prompt_popup_for_parent(parent: HWND) {
    sync_prompt_popup_impl(parent, false);
}

fn sync_prompt_popup_impl(parent: HWND, force: bool) {
    let key = pending_prompt_key();
    if !force {
        let window_present = PROMPT.with(|slot| slot.borrow().is_some());
        if PROMPT_KEY.with(|last| last.get()) == key && window_present == key.is_some() {
            return;
        }
    }
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
                center_window_on_screen(window.window(), parent, PROMPT_WINDOW_LOGICAL_SIZE);
                let _ = window.show();
                center_window_on_screen(window.window(), parent, PROMPT_WINDOW_LOGICAL_SIZE);
                apply_slint_window_icons(window.window());
                schedule_prompt_window_icon_refresh(window.as_weak());
            }
        }
    });
    PROMPT_KEY.with(|last| last.set(key));
}

fn pending_prompt_key() -> Option<(bool, u64)> {
    let state = APP_STATE.get()?;
    let state = state.lock().expect("state poisoned");
    if let Some(choice) = state.current_pending_choice() {
        return Some((true, choice.id));
    }
    state
        .current_pending_permission()
        .map(|permission| (false, permission.id))
}

pub(crate) fn close_prompt_popup() {
    PROMPT.with(|slot| {
        if let Some(window) = slot.borrow_mut().take() {
            let _ = window.hide();
        }
    });
    PROMPT_OPTIONS.with(|options| options.borrow_mut().clear());
    PROMPT_SNAPSHOT.with(|last| last.borrow_mut().take());
    PROMPT_KEY.with(|last| last.set(None));
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
    detail: Vec<BlockView>,
    detail_dominant: bool,
    meta: String,
    is_choice: bool,
    submit_enabled: bool,
    submit_hint: String,
    options: Vec<OptionView>,
}

#[derive(Clone, PartialEq, Eq)]
struct BlockView {
    kind: u8,
    text: String,
    indent: u8,
    lines: u32,
    diff_lines: Vec<DiffLineView>,
}

#[derive(Clone, PartialEq, Eq)]
struct DiffLineView {
    text: String,
    tone: u8,
    lines: u32,
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
    label_lines: u32,
    desc_lines: u32,
}

fn detail_blocks(markdown: &str) -> Vec<BlockView> {
    markdown_blocks(markdown).iter().map(block_view).collect()
}

fn block_view(block: &MarkdownBlock) -> BlockView {
    if block.kind == MarkdownBlockKind::Diff {
        return diff_block_view(&block.text);
    }
    let (kind, font_px) = match block.kind {
        MarkdownBlockKind::Paragraph => (0, 13.0),
        MarkdownBlockKind::Heading(1) => (1, 17.0),
        MarkdownBlockKind::Heading(2) => (2, 15.0),
        MarkdownBlockKind::Heading(level) => (level, 14.0),
        MarkdownBlockKind::Bullet => (4, 13.0),
        MarkdownBlockKind::Code => (5, 12.0),
        MarkdownBlockKind::Quote => (6, 13.0),
        MarkdownBlockKind::Diff => unreachable!("handled above"),
    };
    let mono = block.kind == MarkdownBlockKind::Code;
    let avail = if mono {
        DETAIL_CODE_TEXT_PX
    } else {
        DETAIL_TEXT_PX - f32::from(block.indent) * 14.0
    };
    BlockView {
        kind,
        text: block.text.clone(),
        indent: block.indent,
        lines: estimate_wrapped_lines(&block.text, font_px, avail, mono),
        diff_lines: Vec::new(),
    }
}

fn diff_block_view(text: &str) -> BlockView {
    let mut total = 0u32;
    let diff_lines = text
        .split('\n')
        .map(|raw| {
            let tone = match raw.as_bytes().first() {
                Some(b'+') => 1,
                Some(b'-') => 2,
                _ => 0,
            };
            let lines = estimate_wrapped_lines(raw, 12.0, DETAIL_DIFF_TEXT_PX, true);
            total += lines;
            DiffLineView {
                text: raw.to_string(),
                tone,
                lines,
            }
        })
        .collect();
    BlockView {
        kind: 7,
        text: String::new(),
        indent: 0,
        lines: total,
        diff_lines,
    }
}

fn prompt_snapshot() -> Option<PromptSnapshot> {
    let state = APP_STATE.get()?;
    let state = state.lock().expect("state poisoned");
    if let Some(choice) = state.current_pending_choice() {
        return Some(choice_snapshot(choice));
    }
    state.current_pending_permission().map(permission_snapshot)
}

fn permission_snapshot(permission: &PendingPermission) -> PromptSnapshot {
    PromptSnapshot {
        title: "Permission request".to_string(),
        subtitle: format!("{} wants access", permission.tool_name.trim()),
        detail: detail_blocks(&permission.summary),
        detail_dominant: true,
        meta: prompt_meta(&permission.session_id, &permission.cwd),
        is_choice: false,
        submit_enabled: false,
        submit_hint: String::new(),
        options: Vec::new(),
    }
}

fn choice_snapshot(choice: &PendingChoice) -> PromptSnapshot {
    let mut detail = detail_blocks(choice.detail.trim());
    if detail.is_empty() {
        let questions = choice
            .questions
            .iter()
            .map(|question| question.question.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        detail = detail_blocks(&questions);
    }

    let mut options = Vec::new();
    for (question_index, question) in choice.questions.iter().enumerate() {
        let header_text = if question.header.trim().is_empty() {
            question.question.clone()
        } else {
            format!("{} — {}", question.header, question.question)
        };
        options.push(OptionView {
            question_index,
            option_index: 0,
            label: String::new(),
            desc_lines: estimate_wrapped_lines(&header_text, 12.0, HEADER_TEXT_PX, false),
            description: header_text,
            selected: false,
            is_other: false,
            other_text: String::new(),
            multi_select: question.multi_select,
            is_question_header: true,
            label_lines: 0,
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
                label_lines: estimate_wrapped_lines(&option.label, 13.0, OPTION_TEXT_PX, false),
                desc_lines: estimate_wrapped_lines(
                    &option.description,
                    12.0,
                    OPTION_TEXT_PX,
                    false,
                ),
                label: option.label.clone(),
                description: option.description.clone(),
                selected,
                is_other: option.is_other,
                other_text,
                multi_select: question.multi_select,
                is_question_header: false,
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
        detail_dominant: choice.kind == ChoiceKind::ExitPlanMode,
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
    window.set_detail_dominant(snapshot.detail_dominant);
    window.set_meta_text(shared(&snapshot.meta));
    window.set_submit_enabled(snapshot.submit_enabled);
    window.set_submit_hint(shared(&snapshot.submit_hint));

    let block_data: Vec<MarkdownBlockData> = snapshot
        .detail
        .iter()
        .map(|block| {
            let diff_lines: Vec<DiffLine> = block
                .diff_lines
                .iter()
                .map(|line| DiffLine {
                    text: shared(&line.text),
                    tone: i32::from(line.tone),
                    lines: line.lines as i32,
                })
                .collect();
            MarkdownBlockData {
                kind: i32::from(block.kind),
                text: shared(&block.text),
                indent: i32::from(block.indent),
                lines: block.lines as i32,
                diff_lines: ModelRc::from(Rc::new(VecModel::from(diff_lines))),
            }
        })
        .collect();
    let blocks: ModelRc<MarkdownBlockData> = ModelRc::from(Rc::new(VecModel::from(block_data)));
    window.set_detail_blocks(blocks);

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
            label_lines: opt.label_lines as i32,
            desc_lines: opt.desc_lines as i32,
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
        let Some(choice) = state.current_pending_choice() else {
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
