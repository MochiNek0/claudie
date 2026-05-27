use windows_sys::Win32::Foundation::{RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, Ellipse, FillRect, GetTextExtentPoint32W, HDC,
    LineTo, MoveToEx, PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    TextOutW,
};

use crate::app::pomodoro::{PomodoroMode, PomodoroState, PomodoroStatus, format_remaining};
use crate::app::{AppState, PendingChoice, PendingPermission, PetMood};
use crate::config::*;
use crate::globals::PET_RENDERER;
use crate::settings::UserSettings;
use crate::ui::theme;
use crate::util::{compact_path, shorten, wide};

pub(super) fn render_scene(hdc: HDC, rect: &RECT, state: &RenderState) {
    fill_rect(hdc, rect, TRANSPARENT_KEY);
    let (pet_x, pet_y, pet_w, pet_h) = scaled_pet_rect(state.settings.pet_scale_percent());
    draw_pet(hdc, state.mood, pet_x, pet_y, pet_w, pet_h);
    draw_status_hud(hdc, state, pet_x, pet_y, pet_w, pet_h);
}

pub(super) fn render_permission_overlay(hdc: HDC, rect: &RECT, state: &RenderState) {
    fill_rect(hdc, rect, TRANSPARENT_KEY);
    if let Some(choice) = &state.pending_choice {
        draw_choice_request(hdc, choice);
    } else if let Some(permission) = &state.pending_permission {
        draw_permission_request(hdc, permission);
    }
}

#[derive(Default)]
pub(super) struct RenderState {
    mood: PetMood,
    pending_permission: Option<PendingPermission>,
    pending_choice: Option<PendingChoice>,
    pub(super) settings: UserSettings,
    pomodoro: PomodoroState,
}

pub(super) fn snapshot_state(state: &AppState) -> RenderState {
    RenderState {
        mood: state.mood,
        pending_permission: state.pending_permissions.front().cloned(),
        pending_choice: state.pending_choices.front().cloned(),
        settings: state.settings.clone(),
        pomodoro: state.pomodoro.clone(),
    }
}

fn draw_permission_request(hdc: HDC, permission: &PendingPermission) {
    let card_x = PERMISSION_BUBBLE_X;
    let card_y = PERMISSION_BUBBLE_Y;
    let card_w = PERMISSION_BUBBLE_W;
    let card_h = PERMISSION_BUBBLE_H;

    draw_permission_card(hdc, card_x, card_y, card_w, card_h);

    let text_x = card_x + 24;
    let text_w = card_w - 48;
    text_fit(
        hdc,
        text_x,
        card_y + 22,
        text_w,
        "Permission request",
        theme::INK,
    );
    text_fit(
        hdc,
        text_x,
        card_y + 47,
        text_w,
        &format!("{} wants access", permission.tool_name.trim()),
        theme::MUTED,
    );
    draw_permission_detail_panel(hdc, permission, text_x, card_y + 76, text_w);
    draw_overlay_button(hdc, ALLOW_BUTTON, "Allow", OverlayButtonKind::Primary);
    draw_overlay_button(hdc, ALWAYS_BUTTON, "Always", OverlayButtonKind::Secondary);
    draw_overlay_button(hdc, DENY_BUTTON, "Deny", OverlayButtonKind::Danger);
}

fn draw_permission_detail_panel(hdc: HDC, permission: &PendingPermission, x: i32, y: i32, w: i32) {
    let panel_h = 120;
    filled_round_rect(
        hdc,
        x,
        y,
        w,
        panel_h,
        theme::RADIUS_FIELD,
        theme::FIELD,
        theme::FIELD_BORDER,
    );
    let mut next_y = draw_wrapped_text_limited(
        hdc,
        x + 16,
        y + 14,
        w - 32,
        &permission.summary,
        theme::INK,
        17,
        3,
    );
    next_y += 4;
    text_fit(
        hdc,
        x + 16,
        next_y,
        w - 32,
        &format!("session {}", shorten(&permission.session_id, 8)),
        theme::MUTED,
    );
    next_y += 17;
    if !permission.cwd.is_empty() {
        draw_wrapped_text_limited(
            hdc,
            x + 16,
            next_y,
            w - 32,
            &compact_path(&permission.cwd),
            theme::MUTED_SOFT,
            17,
            2,
        );
    }
}

fn draw_choice_request(hdc: HDC, choice: &PendingChoice) {
    draw_permission_card(
        hdc,
        CHOICE_CARD_X,
        CHOICE_CARD_Y,
        CHOICE_CARD_W,
        CHOICE_CARD_H,
    );
    let text_x = CHOICE_CARD_X + 24;
    let text_w = CHOICE_CARD_W - 48;
    text_fit(
        hdc,
        text_x,
        CHOICE_CARD_Y + 22,
        text_w,
        &choice.title,
        theme::INK,
    );
    let mut next_y = CHOICE_CARD_Y + 50;
    if !choice.detail.trim().is_empty() {
        draw_wrapped_text_limited(
            hdc,
            text_x,
            next_y,
            text_w,
            &choice.detail,
            theme::MUTED,
            17,
            5,
        );
        next_y += 5 * 17 + 8;
    }

    let limit_y = CHOICE_SUBMIT_BUTTON.1 - 12;
    for (question_index, question) in choice.questions.iter().enumerate() {
        if next_y + 47 > limit_y {
            break;
        }
        let heading = if question.header.is_empty() {
            format!("Question {}", question_index + 1)
        } else {
            question.header.clone()
        };
        let question_y = next_y;
        text_fit(hdc, text_x, question_y, text_w, &heading, theme::MUTED);
        draw_wrapped_text_limited(
            hdc,
            text_x,
            question_y + 16,
            text_w,
            &question.question,
            theme::INK,
            17,
            2,
        );
        next_y = question_y + 16 + 28 + 3;

        for (option_index, option) in question.options.iter().enumerate() {
            if next_y + CHOICE_OPTION_H > limit_y {
                break;
            }
            let selected = choice
                .selected
                .get(question_index)
                .is_some_and(|items| items.contains(&option_index));
            let (fill, border) = if selected {
                (theme::ACCENT_SOFT, theme::ACCENT)
            } else {
                (theme::SURFACE, theme::FIELD_BORDER)
            };
            filled_round_rect(
                hdc,
                CHOICE_OPTION_X,
                next_y,
                CHOICE_OPTION_W,
                CHOICE_OPTION_H,
                theme::RADIUS_CHIP,
                fill,
                border,
            );
            // Marker glyph: filled check for selected, hollow circle for idle.
            let (marker, marker_color) = if selected {
                ("\u{2713}", theme::ACCENT)
            } else {
                ("\u{25cb}", theme::MUTED_SOFT)
            };
            text(hdc, CHOICE_OPTION_X + 12, next_y + 6, marker, marker_color);
            let label = if option.description.is_empty() {
                option.label.clone()
            } else {
                format!("{} - {}", option.label, option.description)
            };
            text_fit(
                hdc,
                CHOICE_OPTION_X + 36,
                next_y + 6,
                CHOICE_OPTION_W - 48,
                &label,
                theme::INK,
            );
            next_y += CHOICE_OPTION_H + 4;
        }
        next_y += 6;
    }

    let submit_kind = if choice.is_submittable() {
        OverlayButtonKind::Primary
    } else {
        OverlayButtonKind::PrimaryDisabled
    };
    draw_overlay_button(hdc, CHOICE_SUBMIT_BUTTON, "Submit", submit_kind);
    draw_overlay_button(
        hdc,
        CHOICE_DENY_BUTTON,
        "Cancel",
        OverlayButtonKind::Secondary,
    );
}

fn draw_status_hud(hdc: HDC, state: &RenderState, pet_x: i32, pet_y: i32, pet_w: i32, pet_h: i32) {
    if state.pomodoro.status != PomodoroStatus::Stopped {
        let tomato_color = match state.pomodoro.mode {
            PomodoroMode::Focus => rgb(224, 73, 61),
            PomodoroMode::ShortBreak => rgb(65, 154, 192),
            PomodoroMode::LongBreak => rgb(134, 105, 196),
        };
        let timer = format_remaining(state.pomodoro.remaining(&state.settings.pomodoro));
        draw_pomodoro_badge(hdc, pet_x, pet_y, pet_w, pet_h, &timer, tomato_color);
    }
}

fn draw_pomodoro_badge(
    hdc: HDC,
    pet_x: i32,
    pet_y: i32,
    pet_w: i32,
    pet_h: i32,
    timer: &str,
    body: u32,
) {
    const BADGE_W: i32 = 82;
    const BADGE_H: i32 = 28;
    const VISIBLE_HEAD_Y_PERCENT: i32 = 35;
    const GAP_FROM_HEAD: i32 = 2;
    const SCREEN_PAD: i32 = 8;

    let x =
        (pet_x + pet_w / 2 - BADGE_W / 2).clamp(SCREEN_PAD, WINDOW_WIDTH - BADGE_W - SCREEN_PAD);
    let head_y = pet_y + pet_h * VISIBLE_HEAD_Y_PERCENT / 100;
    let y =
        (head_y - BADGE_H - GAP_FROM_HEAD).clamp(SCREEN_PAD, WINDOW_HEIGHT - BADGE_H - SCREEN_PAD);
    filled_round_rect(
        hdc,
        x,
        y,
        BADGE_W,
        BADGE_H,
        theme::RADIUS_FIELD,
        theme::SURFACE,
        theme::HAIRLINE,
    );

    draw_tomato_icon(hdc, x + 6, y + 5, body);
    text_fit(hdc, x + 32, y + 7, BADGE_W - 36, timer, theme::INK);
}

fn draw_tomato_icon(hdc: HDC, x: i32, y: i32, body: u32) {
    filled_ellipse(hdc, x, y + 4, 20, 18, body);
    filled_ellipse(hdc, x + 3, y + 3, 15, 16, body);
    filled_ellipse(hdc, x + 5, y + 7, 4, 4, rgb(255, 178, 169));
    filled_ellipse(hdc, x + 4, y, 8, 5, rgb(80, 154, 91));
    filled_ellipse(hdc, x + 10, y, 8, 5, rgb(80, 154, 91));
    line(hdc, x + 10, y + 5, x + 13, y + 1, rgb(67, 122, 73));
}

fn draw_pet(hdc: HDC, mood: PetMood, x: i32, y: i32, w: i32, h: i32) {
    if let Some(store) = PET_RENDERER.get() {
        if unsafe {
            store
                .lock()
                .expect("pet renderer poisoned")
                .draw(hdc, mood, x, y, w, h)
        } {
            return;
        }
    }
    draw_pet_fallback(hdc, mood, x, y, w, h);
}

fn scaled_pet_rect(scale_percent: u32) -> (i32, i32, i32, i32) {
    let scale = scale_percent as i32;
    let w = (PET_W * scale + 50) / 100;
    let h = (PET_H * scale + 50) / 100;
    let center_x = PET_X + PET_W / 2;
    let bottom_y = PET_Y + PET_H;
    (center_x - w / 2, bottom_y - h, w.max(1), h.max(1))
}

fn draw_pet_fallback(hdc: HDC, mood: PetMood, x: i32, y: i32, w: i32, h: i32) {
    let body = match mood {
        PetMood::Search => rgb(245, 174, 64),
        PetMood::Error => rgb(222, 86, 80),
        PetMood::Happy => rgb(72, 173, 121),
        PetMood::Building => rgb(84, 130, 200),
        PetMood::Typing => rgb(92, 178, 191),
        PetMood::Subagent => rgb(143, 108, 207),
        PetMood::Sleeping => rgb(116, 128, 142),
        _ => rgb(78, 163, 170),
    };
    let shade = match mood {
        PetMood::Sleeping => rgb(98, 108, 120),
        _ => rgb(42, 57, 70),
    };

    let px = |value: i32| x + value * w / PET_W;
    let py = |value: i32| y + value * h / PET_H;
    let pw = |value: i32| (value * w / PET_W).max(1);
    let ph = |value: i32| (value * h / PET_H).max(1);

    filled_ellipse(hdc, px(12), py(22), pw(122), ph(92), body);
    filled_ellipse(hdc, px(8), py(8), pw(48), ph(44), body);
    filled_ellipse(hdc, px(82), py(8), pw(48), ph(44), body);
    filled_ellipse(hdc, px(32), py(24), pw(76), ph(64), rgb(250, 253, 255));
    filled_ellipse(hdc, px(45), py(45), pw(10), ph(14), shade);
    filled_ellipse(hdc, px(84), py(45), pw(10), ph(14), shade);

    match mood {
        PetMood::Happy => {
            line(hdc, px(61), py(68), px(70), py(76), shade);
            line(hdc, px(70), py(76), px(83), py(66), shade);
        }
        PetMood::Error => {
            line(hdc, px(57), py(68), px(83), py(68), shade);
        }
        PetMood::Search => {
            line(hdc, px(53), py(58), px(74), py(58), shade);
            line(hdc, px(74), py(58), px(91), py(75), shade);
            filled_ellipse(hdc, px(46), py(45), pw(26), ph(26), rgb(250, 253, 255));
            line(hdc, px(50), py(58), px(67), py(58), shade);
        }
        PetMood::Sleeping => {
            line(hdc, px(44), py(51), px(55), py(51), shade);
            line(hdc, px(84), py(51), px(95), py(51), shade);
        }
        PetMood::Typing => {
            filled_rect(hdc, px(45), py(70), pw(50), ph(10), rgb(72, 87, 98));
        }
        PetMood::Building => {
            line(hdc, px(100), py(25), px(122), py(47), shade);
            filled_rect(hdc, px(114), py(26), pw(22), ph(10), rgb(72, 87, 98));
        }
        PetMood::Subagent => {
            filled_ellipse(hdc, px(100), py(8), pw(16), ph(16), rgb(129, 96, 190));
            filled_ellipse(hdc, px(119), py(26), pw(16), ph(16), rgb(129, 96, 190));
        }
        _ => {
            filled_rect(hdc, px(60), py(68), pw(20), ph(5), shade);
        }
    }
}

fn draw_permission_card(hdc: HDC, x: i32, y: i32, w: i32, h: i32) {
    let radius = theme::RADIUS_CARD;
    filled_round_rect(hdc, x, y, w, h, radius, theme::SURFACE, theme::HAIRLINE);
    // Header band: tinted surface; its bottom is masked by the hairline.
    filled_round_rect(
        hdc,
        x,
        y + 1,
        w,
        64,
        radius,
        theme::SURFACE_ALT,
        theme::SURFACE_ALT,
    );
    fill_rect(
        hdc,
        &RECT {
            left: x + 1,
            top: y + 64,
            right: x + w - 1,
            bottom: y + 65,
        },
        theme::HAIRLINE,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OverlayButtonKind {
    Primary,
    PrimaryDisabled,
    Secondary,
    Danger,
}

fn draw_overlay_button(hdc: HDC, rect: (i32, i32, i32, i32), label: &str, kind: OverlayButtonKind) {
    let (x, y, w, h) = rect;
    let (fill, border, text_color) = match kind {
        OverlayButtonKind::Primary => (theme::ACCENT, theme::ACCENT, theme::SURFACE),
        OverlayButtonKind::PrimaryDisabled => {
            (theme::FIELD, theme::FIELD_BORDER, theme::MUTED_SOFT)
        }
        OverlayButtonKind::Secondary => (theme::SURFACE, theme::FIELD_BORDER, theme::INK),
        OverlayButtonKind::Danger => (theme::SURFACE, theme::DANGER_SOFT, theme::DANGER),
    };
    filled_round_rect(hdc, x, y, w, h, theme::RADIUS_BUTTON, fill, border);
    let label_x = x + (w - text_width(hdc, label)).max(0) / 2;
    text(hdc, label_x, y + 8, label, text_color);
}

pub(super) fn fill_rect(hdc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(hdc, rect, brush);
        DeleteObject(brush);
    }
}

fn filled_rect(hdc: HDC, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let rect = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    fill_rect(hdc, &rect, color);
}

fn filled_round_rect(
    hdc: HDC,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    fill: u32,
    border: u32,
) {
    unsafe {
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(hdc, brush);
        let old_pen = SelectObject(hdc, pen);
        RoundRect(hdc, x, y, x + w, y + h, radius, radius);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

fn filled_ellipse(hdc: HDC, x: i32, y: i32, w: i32, h: i32, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID, 1, color);
        let old_brush = SelectObject(hdc, brush);
        let old_pen = SelectObject(hdc, pen);
        Ellipse(hdc, x, y, x + w, y + h);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

fn line(hdc: HDC, x1: i32, y1: i32, x2: i32, y2: i32, color: u32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, 2, color);
        let old_pen = SelectObject(hdc, pen);
        MoveToEx(hdc, x1, y1, std::ptr::null_mut());
        LineTo(hdc, x2, y2);
        SelectObject(hdc, old_pen);
        DeleteObject(pen);
    }
}

fn text_fit(hdc: HDC, x: i32, y: i32, max_width: i32, value: &str, color: u32) {
    let fitted = fit_text_to_width(hdc, value, max_width);
    text(hdc, x, y, &fitted, color);
}

fn draw_wrapped_text_limited(
    hdc: HDC,
    x: i32,
    y: i32,
    max_width: i32,
    value: &str,
    color: u32,
    line_height: i32,
    max_lines: usize,
) -> i32 {
    let mut lines = wrap_text_to_width(hdc, value, max_width);
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            *last = fit_text_to_width(hdc, &format!("{last}..."), max_width);
        }
    }

    let mut next_y = y;
    for line in lines {
        text(hdc, x, next_y, &line, color);
        next_y += line_height;
    }
    next_y
}

fn wrap_text_to_width(hdc: HDC, value: &str, max_width: i32) -> Vec<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() || max_width <= 0 {
        return vec![normalized];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    for word in normalized.split(' ') {
        if current.is_empty() {
            if text_width(hdc, word) <= max_width {
                current.push_str(word);
            } else {
                append_split_word(hdc, word, max_width, &mut lines, &mut current);
            }
            continue;
        }

        let candidate = format!("{current} {word}");
        if text_width(hdc, &candidate) <= max_width {
            current = candidate;
        } else {
            lines.push(current);
            current = String::new();
            if text_width(hdc, word) <= max_width {
                current.push_str(word);
            } else {
                append_split_word(hdc, word, max_width, &mut lines, &mut current);
            }
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn append_split_word(
    hdc: HDC,
    word: &str,
    max_width: i32,
    lines: &mut Vec<String>,
    current: &mut String,
) {
    let mut piece = String::new();
    for ch in word.chars() {
        let candidate = format!("{piece}{ch}");
        if !piece.is_empty() && text_width(hdc, &candidate) > max_width {
            lines.push(piece);
            piece = String::new();
        }
        piece.push(ch);
    }
    *current = piece;
}

fn fit_text_to_width(hdc: HDC, value: &str, max_width: i32) -> String {
    if max_width <= 0 || text_width(hdc, value) <= max_width {
        return value.to_string();
    }

    let ellipsis = "...";
    let ellipsis_width = text_width(hdc, ellipsis);
    let available = max_width.saturating_sub(ellipsis_width);
    let mut fitted = String::new();
    for ch in value.chars() {
        let next = format!("{fitted}{ch}");
        if text_width(hdc, &next) > available {
            break;
        }
        fitted.push(ch);
    }
    fitted.push_str(ellipsis);
    fitted
}

fn text_width(hdc: HDC, value: &str) -> i32 {
    unsafe {
        let wide = wide(value);
        let mut size = SIZE { cx: 0, cy: 0 };
        if GetTextExtentPoint32W(hdc, wide.as_ptr(), (wide.len() - 1) as i32, &mut size) == 0 {
            return value.chars().count() as i32 * 7;
        }
        size.cx
    }
}

fn text(hdc: HDC, x: i32, y: i32, value: &str, color: u32) {
    unsafe {
        let wide = wide(value);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, color);
        TextOutW(hdc, x, y, wide.as_ptr(), (wide.len() - 1) as i32);
    }
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}
