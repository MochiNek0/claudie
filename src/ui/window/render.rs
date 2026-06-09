use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicI32, AtomicU64, Ordering},
};
use windows_sys::Win32::Foundation::{RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, Ellipse, FillRect, GetTextExtentPoint32W, HDC,
    LineTo, MoveToEx, PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    TextOutW,
};

use crate::app::fishing::{FishingPhase, FishingState};
use crate::app::pomodoro::{PomodoroMode, PomodoroState, PomodoroStatus, format_remaining};
use crate::app::{
    AppState, ClaudeSessionStatus, PendingChoice, PendingPermission, PetMood, SessionSwitcherItem,
};
use crate::config::*;
use crate::globals::PET_RENDERER;
use crate::settings::UserSettings;
use crate::ui::theme;
use crate::util::{compact_path, markdown_to_display_text, shorten, wide};

static PERMISSION_SCROLL_LINES: AtomicI32 = AtomicI32::new(0);
static PERMISSION_SCROLL_ID: AtomicU64 = AtomicU64::new(0);
static PERMISSION_DETAIL_CACHE: OnceLock<Mutex<PermissionDetailCache>> = OnceLock::new();
static CHOICE_SCROLL_LINES: AtomicI32 = AtomicI32::new(0);
static CHOICE_SCROLL_ID: AtomicU64 = AtomicU64::new(0);
const CHOICE_SCROLL_LINE_H: i32 = 17;

#[derive(Default)]
struct PermissionDetailCache {
    id: u64,
    width: i32,
    lines: Vec<PermissionDetailLine>,
}

#[derive(Clone)]
struct PermissionDetailLine {
    text: String,
    color: u32,
}

pub(super) fn scroll_permission_detail_lines(delta: i32) {
    let current = PERMISSION_SCROLL_LINES.load(Ordering::Relaxed);
    PERMISSION_SCROLL_LINES.store(current.saturating_add(delta).max(0), Ordering::Relaxed);
}

pub(super) fn scroll_choice_lines(delta: i32) {
    let current = CHOICE_SCROLL_LINES.load(Ordering::Relaxed);
    CHOICE_SCROLL_LINES.store(current.saturating_add(delta).max(0), Ordering::Relaxed);
}

pub(super) fn choice_option_at_point(
    choice: &PendingChoice,
    px: i32,
    py: i32,
) -> Option<(usize, usize)> {
    let viewport = choice_content_viewport();
    if !point_in_rect(px, py, viewport) {
        return None;
    }

    let (scroll_lines, _, _, _) = choice_scroll_metrics(choice);
    let viewport_top = viewport.1;
    let viewport_bottom = viewport.1 + viewport.3;
    let mut y = viewport_top - scroll_lines * CHOICE_SCROLL_LINE_H;
    if !choice.detail.trim().is_empty() {
        y += choice_detail_reserved_h();
    }

    for (question_index, question) in choice.questions.iter().enumerate() {
        if y > viewport_bottom {
            break;
        }
        y += 16 + 28 + 3;
        for (option_index, _) in question.options.iter().enumerate() {
            if y > viewport_bottom {
                break;
            }
            if y >= viewport_top
                && y + CHOICE_OPTION_H <= viewport_bottom
                && point_in_rect(
                    px,
                    py,
                    (CHOICE_OPTION_X, y, CHOICE_OPTION_W, CHOICE_OPTION_H),
                )
            {
                return Some((question_index, option_index));
            }
            y += CHOICE_OPTION_H + 4;
        }
        y += 6;
    }
    None
}

pub(super) fn session_switcher_session_at_point(
    state: &AppState,
    px: i32,
    py: i32,
    width: i32,
    height: i32,
) -> Option<String> {
    if !state.settings.show_session_switcher {
        return None;
    }
    let items = state.session_switcher_items();
    if items.len() <= 1 {
        return None;
    }
    if !point_in_rect(px, py, (0, 0, width, height)) {
        return None;
    }
    let visible_count = items.len().min(SESSION_SWITCHER_MAX_VISIBLE_ITEMS);
    for (index, item) in items.iter().take(visible_count).enumerate() {
        let row_y = SESSION_SWITCHER_VERTICAL_PADDING + index as i32 * (SESSION_BAR_HEIGHT + 1);
        if point_in_rect(px, py, (0, row_y, width, SESSION_BAR_HEIGHT)) {
            return Some(item.id.clone());
        }
    }
    None
}

pub(super) fn render_pet_window(
    hdc: HDC,
    rect: &RECT,
    state: &RenderState,
    pet_offset_x: i32,
    pet_offset_y: i32,
) {
    fill_rect(hdc, rect, TRANSPARENT_KEY);
    let (pet_x, pet_y, pet_w, pet_h) = scaled_pet_rect(state.settings.pet_scale_percent());
    draw_pet(
        hdc,
        state.mood,
        pet_x + pet_offset_x,
        pet_y + pet_offset_y,
        pet_w,
        pet_h,
    );
}

pub(super) fn render_pomodoro_hud_window(hdc: HDC, rect: &RECT, state: &RenderState) {
    fill_rect(hdc, rect, TRANSPARENT_KEY);
    if state.pomodoro.status != PomodoroStatus::Stopped {
        draw_pomodoro_hud(
            hdc,
            state,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
        );
    }
}

pub(super) fn render_fishing_hud_window(hdc: HDC, rect: &RECT, state: &RenderState) {
    fill_rect(hdc, rect, TRANSPARENT_KEY);
    if state.fishing.is_active() {
        draw_fishing_hud(
            hdc,
            state,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
        );
    }
}

pub(super) fn render_session_switcher_window(hdc: HDC, rect: &RECT, state: &RenderState) {
    fill_rect(hdc, rect, TRANSPARENT_KEY);
    draw_session_switcher(
        hdc,
        state,
        0,
        0,
        rect.right - rect.left,
        rect.bottom - rect.top,
    );
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
    fishing: FishingState,
    sessions: Vec<SessionSwitcherItem>,
}

pub(super) fn snapshot_state(state: &AppState) -> RenderState {
    RenderState {
        mood: state.mood,
        pending_permission: state.current_pending_permission().cloned(),
        pending_choice: state.current_pending_choice().cloned(),
        settings: state.settings.clone(),
        pomodoro: state.pomodoro.clone(),
        fishing: state.fishing.clone(),
        sessions: state.session_switcher_items(),
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
    let panel_h = PERMISSION_DETAIL_PANEL_H;
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
    ensure_permission_scroll_id(permission.id);
    draw_scrollable_permission_text(hdc, permission, x, y, w, panel_h);
}

fn ensure_permission_scroll_id(id: u64) {
    if PERMISSION_SCROLL_ID.swap(id, Ordering::Relaxed) != id {
        PERMISSION_SCROLL_LINES.store(0, Ordering::Relaxed);
    }
}

fn draw_scrollable_permission_text(
    hdc: HDC,
    permission: &PendingPermission,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) {
    let line_height = 17;
    let visible_lines = ((h - 28) / line_height).max(1) as usize;
    let text_x = x + 16;
    let text_y = y + 14;
    let text_w = w - 40;
    let (visible_rows, total_lines, max_scroll, scroll) =
        visible_permission_detail_lines(hdc, permission, text_w, visible_lines);

    for (line_index, line) in visible_rows.iter().enumerate() {
        text(
            hdc,
            text_x,
            text_y + line_index as i32 * line_height,
            &line.text,
            line.color,
        );
    }

    if max_scroll > 0 {
        draw_scrollbar(
            hdc,
            x + w - 10,
            y + 12,
            h - 24,
            visible_lines,
            total_lines,
            scroll,
        );
    }
}

fn visible_permission_detail_lines(
    hdc: HDC,
    permission: &PendingPermission,
    text_w: i32,
    visible_lines: usize,
) -> (Vec<PermissionDetailLine>, usize, i32, i32) {
    let mut cache = permission_detail_cache()
        .lock()
        .expect("permission detail cache poisoned");
    if cache.id != permission.id || cache.width != text_w {
        cache.id = permission.id;
        cache.width = text_w;
        cache.lines = build_permission_detail_lines(hdc, permission, text_w);
    }

    let total_lines = cache.lines.len();
    let max_scroll = total_lines.saturating_sub(visible_lines) as i32;
    let scroll = PERMISSION_SCROLL_LINES
        .load(Ordering::Relaxed)
        .clamp(0, max_scroll);
    PERMISSION_SCROLL_LINES.store(scroll, Ordering::Relaxed);
    let visible_rows = cache
        .lines
        .iter()
        .skip(scroll as usize)
        .take(visible_lines)
        .cloned()
        .collect::<Vec<_>>();

    (visible_rows, total_lines, max_scroll, scroll)
}

fn permission_detail_cache() -> &'static Mutex<PermissionDetailCache> {
    PERMISSION_DETAIL_CACHE.get_or_init(|| Mutex::new(PermissionDetailCache::default()))
}

fn build_permission_detail_lines(
    hdc: HDC,
    permission: &PendingPermission,
    text_w: i32,
) -> Vec<PermissionDetailLine> {
    let detail_text = permission_detail_text(permission);
    let compact_cwd = compact_path(&permission.cwd);
    wrap_text_to_width(hdc, &detail_text, text_w)
        .into_iter()
        .map(|line| {
            let color = if line.starts_with("session ") {
                theme::MUTED
            } else if line == compact_cwd {
                theme::MUTED_SOFT
            } else {
                theme::INK
            };
            PermissionDetailLine { text: line, color }
        })
        .collect()
}

fn permission_detail_text(permission: &PendingPermission) -> String {
    let mut text = markdown_to_display_text(&permission.summary);
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(&format!("session {}", shorten(&permission.session_id, 8)));
    if !permission.cwd.is_empty() {
        text.push('\n');
        text.push_str(&compact_path(&permission.cwd));
    }
    text
}

fn draw_scrollbar(
    hdc: HDC,
    x: i32,
    y: i32,
    h: i32,
    visible_lines: usize,
    total_lines: usize,
    scroll: i32,
) {
    let track_h = h.max(24);
    filled_round_rect(
        hdc,
        x,
        y,
        4,
        track_h,
        4,
        theme::FIELD_BORDER,
        theme::FIELD_BORDER,
    );
    let thumb_h = ((track_h as f32 * visible_lines as f32 / total_lines as f32).round() as i32)
        .clamp(18, track_h);
    let max_scroll = total_lines.saturating_sub(visible_lines).max(1) as i32;
    let thumb_y = y + (track_h - thumb_h) * scroll / max_scroll;
    filled_round_rect(
        hdc,
        x,
        thumb_y,
        4,
        thumb_h,
        4,
        theme::MUTED_SOFT,
        theme::MUTED_SOFT,
    );
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
    let viewport = choice_content_viewport();
    let viewport_top = viewport.1;
    let viewport_bottom = viewport.1 + viewport.3;
    let (scroll_lines, max_scroll_lines, visible_lines, total_lines) =
        choice_scroll_metrics(choice);
    let mut next_y = viewport_top - scroll_lines * CHOICE_SCROLL_LINE_H;
    if !choice.detail.trim().is_empty() {
        let detail = markdown_to_display_text(&choice.detail);
        draw_wrapped_text_limited_in_viewport(
            hdc,
            text_x,
            next_y,
            text_w,
            &detail,
            theme::MUTED,
            17,
            5,
            viewport_top,
            viewport_bottom,
        );
        next_y += choice_detail_reserved_h();
    }

    for (question_index, question) in choice.questions.iter().enumerate() {
        if next_y > viewport_bottom {
            break;
        }
        let heading = if question.header.is_empty() {
            format!("Question {}", question_index + 1)
        } else {
            question.header.clone()
        };
        let question_y = next_y;
        text_fit_in_viewport(
            hdc,
            text_x,
            question_y,
            text_w,
            &heading,
            theme::MUTED,
            viewport_top,
            viewport_bottom,
        );
        draw_wrapped_text_limited_in_viewport(
            hdc,
            text_x,
            question_y + 16,
            text_w,
            &question.question,
            theme::INK,
            17,
            2,
            viewport_top,
            viewport_bottom,
        );
        next_y = question_y + 16 + 28 + 3;

        for (option_index, option) in question.options.iter().enumerate() {
            if next_y > viewport_bottom {
                break;
            }
            if next_y + CHOICE_OPTION_H <= viewport_top {
                next_y += CHOICE_OPTION_H + 4;
                continue;
            }
            if next_y + CHOICE_OPTION_H > viewport_bottom {
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

    if max_scroll_lines > 0 {
        draw_scrollbar(
            hdc,
            CHOICE_CARD_X + CHOICE_CARD_W - 16,
            viewport_top + 2,
            viewport.3 - 4,
            visible_lines,
            total_lines,
            scroll_lines,
        );
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

fn choice_content_viewport() -> (i32, i32, i32, i32) {
    let y = CHOICE_CARD_Y + 50;
    (
        CHOICE_CARD_X + 24,
        y,
        CHOICE_CARD_W - 48,
        CHOICE_SUBMIT_BUTTON.1 - 12 - y,
    )
}

fn choice_detail_reserved_h() -> i32 {
    5 * 17 + 8
}

fn ensure_choice_scroll_id(id: u64) {
    if CHOICE_SCROLL_ID.swap(id, Ordering::Relaxed) != id {
        CHOICE_SCROLL_LINES.store(0, Ordering::Relaxed);
    }
}

fn choice_scroll_metrics(choice: &PendingChoice) -> (i32, i32, usize, usize) {
    ensure_choice_scroll_id(choice.id);
    let viewport_h = choice_content_viewport().3.max(1);
    let content_h = choice_content_height(choice).max(1);
    let max_scroll_px = content_h.saturating_sub(viewport_h);
    let max_scroll_lines = if max_scroll_px == 0 {
        0
    } else {
        (max_scroll_px + CHOICE_SCROLL_LINE_H - 1) / CHOICE_SCROLL_LINE_H
    };
    let scroll_lines = CHOICE_SCROLL_LINES
        .load(Ordering::Relaxed)
        .clamp(0, max_scroll_lines);
    CHOICE_SCROLL_LINES.store(scroll_lines, Ordering::Relaxed);
    let visible_lines = (viewport_h / CHOICE_SCROLL_LINE_H).max(1) as usize;
    let total_lines = ((content_h + CHOICE_SCROLL_LINE_H - 1) / CHOICE_SCROLL_LINE_H)
        .max(visible_lines as i32) as usize;
    (scroll_lines, max_scroll_lines, visible_lines, total_lines)
}

fn choice_content_height(choice: &PendingChoice) -> i32 {
    let mut height = 0;
    if !choice.detail.trim().is_empty() {
        height += choice_detail_reserved_h();
    }
    for question in &choice.questions {
        height += 16 + 28 + 3;
        height += question.options.len() as i32 * (CHOICE_OPTION_H + 4);
        height += 6;
    }
    height
}

fn draw_session_switcher(hdc: HDC, state: &RenderState, x: i32, y: i32, w: i32, h: i32) {
    if !state.settings.show_session_switcher || state.sessions.len() <= 1 {
        return;
    }
    let panel = rgb(33, 32, 37);
    let panel_border = rgb(43, 42, 49);
    let focused_row = rgb(39, 37, 45);
    let divider = rgb(48, 47, 55);
    let visible_count = state.sessions.len().min(SESSION_SWITCHER_MAX_VISIBLE_ITEMS);

    filled_round_rect(hdc, x, y, w, h, theme::RADIUS_FIELD, panel, panel_border);

    for (index, item) in state.sessions.iter().take(visible_count).enumerate() {
        let row_y = y + SESSION_SWITCHER_VERTICAL_PADDING + index as i32 * (SESSION_BAR_HEIGHT + 1);
        let accent = session_accent_color(item, index);

        if item.focused {
            filled_rect(
                hdc,
                x + 7,
                row_y + 1,
                w - 14,
                SESSION_BAR_HEIGHT - 2,
                focused_row,
            );
        }

        filled_round_rect(
            hdc,
            x + 2,
            row_y + 3,
            5,
            SESSION_BAR_HEIGHT - 6,
            4,
            accent,
            accent,
        );

        let count_x = (x + (w * 40) / 100).max(x + 88).min(x + w - 96);
        let symbol_x = count_x + 30;
        let detail_x = symbol_x + 20;
        let name_x = x + 22;
        let text_y = row_y + 5;
        let name = item.display_name.trim();
        let name = if name.is_empty() { "Session" } else { name };
        let fitted_name = fit_text_to_width(hdc, name, (count_x - name_x - 8).max(1));
        text(hdc, name_x, text_y, &fitted_name, accent);

        let count = session_count_text(item);
        text_fit(
            hdc,
            count_x,
            text_y,
            (symbol_x - count_x - 8).max(1),
            &count,
            rgb(196, 199, 206),
        );

        text_fit(
            hdc,
            symbol_x,
            text_y,
            16,
            session_status_symbol(item.status),
            rgb(239, 156, 46),
        );
        text_fit(
            hdc,
            detail_x,
            text_y,
            (x + w - detail_x - 12).max(1),
            &session_status_text(item),
            rgb(237, 239, 244),
        );

        if index + 1 < visible_count {
            filled_rect(hdc, x + 7, row_y + SESSION_BAR_HEIGHT, w - 9, 1, divider);
        }
    }
}

fn session_accent_color(item: &SessionSwitcherItem, index: usize) -> u32 {
    match item.status {
        ClaudeSessionStatus::Error => rgb(232, 93, 87),
        ClaudeSessionStatus::WaitingPermission | ClaudeSessionStatus::WaitingChoice => {
            rgb(145, 95, 231)
        }
        _ if item.focused => rgb(125, 83, 224),
        _ if index % 2 == 0 => rgb(125, 83, 224),
        _ => rgb(32, 143, 242),
    }
}

fn session_count_text(item: &SessionSwitcherItem) -> String {
    item.pending_count.to_string()
}

fn session_status_symbol(status: ClaudeSessionStatus) -> &'static str {
    match status {
        ClaudeSessionStatus::Tool => ">",
        ClaudeSessionStatus::Streaming | ClaudeSessionStatus::Compacting => "*",
        ClaudeSessionStatus::WaitingPermission | ClaudeSessionStatus::WaitingChoice => "!",
        ClaudeSessionStatus::Error => "x",
        ClaudeSessionStatus::Idle | ClaudeSessionStatus::Done => "-",
        ClaudeSessionStatus::Ended => ".",
    }
}

fn session_status_text(item: &SessionSwitcherItem) -> String {
    let detail = item.detail.trim();
    match item.status {
        ClaudeSessionStatus::Idle | ClaudeSessionStatus::Done => "ready".to_string(),
        ClaudeSessionStatus::Streaming => "thinking".to_string(),
        ClaudeSessionStatus::Tool => {
            if detail.is_empty() {
                "tool".to_string()
            } else {
                detail.to_string()
            }
        }
        ClaudeSessionStatus::WaitingPermission => "permission".to_string(),
        ClaudeSessionStatus::WaitingChoice => "choice".to_string(),
        ClaudeSessionStatus::Compacting => item.status.label().to_ascii_lowercase(),
        ClaudeSessionStatus::Error => {
            if detail.is_empty() {
                "error".to_string()
            } else {
                detail.to_string()
            }
        }
        ClaudeSessionStatus::Ended => "ended".to_string(),
    }
}

fn draw_fishing_hud(hdc: HDC, state: &RenderState, x: i32, y: i32, hud_w: i32, hud_h: i32) {
    let panel = rgb(27, 43, 58);
    let panel_border = rgb(63, 92, 113);
    let ink_on_panel = rgb(248, 252, 255);
    let soft_on_panel = rgb(174, 202, 218);
    filled_round_rect(
        hdc,
        x,
        y,
        hud_w,
        hud_h,
        theme::RADIUS_FIELD,
        panel,
        panel_border,
    );
    filled_rect(hdc, x + 8, y + 8, 4, hud_h - 16, rgb(65, 154, 192));

    match state.fishing.phase {
        FishingPhase::Waiting => {
            draw_bobber_icon(hdc, x + 26, y + 23);
            text_fit(
                hdc,
                x + 58,
                y + 15,
                (hud_w - 72).max(1),
                "CASTING",
                ink_on_panel,
            );
            text_fit(
                hdc,
                x + 58,
                y + 35,
                (hud_w - 72).max(1),
                "Waiting for a bite",
                soft_on_panel,
            );
            draw_water_waves(hdc, x + 58, y + 58, (hud_w - 82).max(1), rgb(65, 154, 192));
        }
        FishingPhase::Reeling => {
            text_fit(hdc, x + 18, y + 10, 88, "FISH ON!", ink_on_panel);
            text_fit(
                hdc,
                x + 112,
                y + 10,
                (hud_w - 128).max(1),
                "TAP TO REEL",
                soft_on_panel,
            );
            draw_fishing_meter(hdc, &state.fishing, x + 18, y + 31, (hud_w - 36).max(1));
            draw_catch_progress(
                hdc,
                x + 18,
                y + 58,
                (hud_w - 36).max(1),
                state.fishing.progress,
            );
        }
        FishingPhase::Caught => {
            filled_rect(hdc, x + 8, y + 8, 4, hud_h - 16, rgb(72, 173, 121));
            draw_fish_icon(hdc, x + 24, y + 27, rgb(72, 173, 121));
            text_fit(
                hdc,
                x + 64,
                y + 18,
                (hud_w - 82).max(1),
                "CAUGHT!",
                ink_on_panel,
            );
            draw_catch_progress(hdc, x + 64, y + 47, (hud_w - 84).max(1), 1.0);
        }
        FishingPhase::Missed => {
            filled_rect(hdc, x + 8, y + 8, 4, hud_h - 16, rgb(222, 86, 80));
            draw_fish_icon(hdc, x + 24, y + 27, rgb(222, 86, 80));
            text_fit(
                hdc,
                x + 64,
                y + 18,
                (hud_w - 82).max(1),
                "ESCAPED",
                ink_on_panel,
            );
            text_fit(
                hdc,
                x + 64,
                y + 43,
                (hud_w - 82).max(1),
                "The line went slack",
                soft_on_panel,
            );
        }
        FishingPhase::Inactive => {}
    }
}

fn draw_fishing_meter(hdc: HDC, fishing: &FishingState, x: i32, y: i32, w: i32) {
    const BAR_H: i32 = 14;
    let track = rgb(16, 30, 43);
    let track_border = rgb(75, 103, 121);
    filled_round_rect(hdc, x, y, w, BAR_H, theme::RADIUS_CHIP, track, track_border);
    let (target_min, target_max) = fishing.target_range();
    let target_x = x + (w as f32 * target_min).round() as i32;
    let target_w = (w as f32 * (target_max - target_min)).round() as i32;
    filled_round_rect(
        hdc,
        target_x,
        y + 2,
        target_w.max(4),
        BAR_H - 4,
        theme::RADIUS_CHIP,
        rgb(96, 191, 123),
        rgb(96, 191, 123),
    );
    let marker_x = x + (w as f32 * fishing.tension).round() as i32;
    filled_rect(hdc, marker_x - 3, y - 4, 6, BAR_H + 8, rgb(255, 216, 106));
    line(
        hdc,
        marker_x - 7,
        y - 6,
        marker_x,
        y - 1,
        rgb(255, 216, 106),
    );
    line(
        hdc,
        marker_x + 7,
        y - 6,
        marker_x,
        y - 1,
        rgb(255, 216, 106),
    );
}

fn draw_catch_progress(hdc: HDC, x: i32, y: i32, w: i32, progress: f32) {
    let segments = 8;
    let gap = 3;
    let segment_w = ((w - gap * (segments - 1)) / segments).max(1);
    let filled = ((progress.clamp(0.0, 1.0) * segments as f32).ceil() as i32).clamp(0, segments);
    for index in 0..segments {
        let sx = x + index * (segment_w + gap);
        let color = if index < filled {
            rgb(10, 132, 255)
        } else {
            rgb(55, 75, 91)
        };
        filled_round_rect(hdc, sx, y, segment_w, 8, theme::RADIUS_CHIP, color, color);
    }
}

fn draw_bobber_icon(hdc: HDC, x: i32, y: i32) {
    line(hdc, x + 11, y - 15, x + 11, y - 2, rgb(200, 222, 234));
    filled_ellipse(hdc, x, y, 22, 22, rgb(248, 252, 255));
    filled_rect(hdc, x + 1, y + 11, 20, 10, rgb(222, 86, 80));
    filled_ellipse(hdc, x + 7, y + 7, 4, 4, rgb(248, 252, 255));
}

fn draw_water_waves(hdc: HDC, x: i32, y: i32, w: i32, color: u32) {
    let wave_w = 30;
    let mut next_x = x;
    while next_x + wave_w <= x + w {
        line(hdc, next_x, y + 5, next_x + 7, y, color);
        line(hdc, next_x + 7, y, next_x + 15, y + 5, color);
        line(hdc, next_x + 15, y + 5, next_x + 23, y, color);
        line(hdc, next_x + 23, y, next_x + 30, y + 5, color);
        next_x += wave_w + 8;
    }
}

fn draw_fish_icon(hdc: HDC, x: i32, y: i32, body: u32) {
    filled_ellipse(hdc, x, y + 4, 22, 14, body);
    line(hdc, x + 21, y + 11, x + 30, y + 4, body);
    line(hdc, x + 21, y + 11, x + 30, y + 18, body);
    line(hdc, x + 30, y + 4, x + 30, y + 18, body);
    filled_ellipse(hdc, x + 5, y + 8, 3, 3, theme::SURFACE);
}

fn draw_pomodoro_hud(hdc: HDC, state: &RenderState, x: i32, y: i32, w: i32, h: i32) {
    let body = match state.pomodoro.mode {
        PomodoroMode::Focus => rgb(224, 73, 61),
        PomodoroMode::ShortBreak => rgb(65, 154, 192),
        PomodoroMode::LongBreak => rgb(134, 105, 196),
    };
    let timer = format_remaining(state.pomodoro.remaining(&state.settings.pomodoro));
    filled_round_rect(
        hdc,
        x,
        y,
        w,
        h,
        theme::RADIUS_FIELD,
        theme::SURFACE,
        theme::HAIRLINE,
    );

    draw_tomato_icon(hdc, x + 6, y + 5, body);
    text_fit(hdc, x + 32, y + 7, w - 36, &timer, theme::INK);
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
    let (w, h) = scaled_pet_size_for_percent(scale_percent);
    (PET_X, PET_Y, w, h)
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
        PetMood::Fishing => rgb(65, 154, 192),
        PetMood::FishingReel => rgb(224, 155, 63),
        PetMood::FishingCaught => rgb(72, 173, 121),
        PetMood::FishingMissed => rgb(222, 86, 80),
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
        PetMood::Fishing => {
            line(hdc, px(102), py(42), px(142), py(24), shade);
            line(hdc, px(142), py(24), px(157), py(34), rgb(65, 154, 192));
            filled_ellipse(hdc, px(150), py(32), pw(14), ph(8), rgb(65, 154, 192));
        }
        PetMood::FishingReel => {
            line(hdc, px(100), py(42), px(150), py(64), shade);
            line(hdc, px(150), py(64), px(162), py(55), rgb(65, 154, 192));
            filled_ellipse(hdc, px(158), py(52), pw(18), ph(10), rgb(65, 154, 192));
            line(hdc, px(57), py(68), px(83), py(68), shade);
        }
        PetMood::FishingCaught => {
            line(hdc, px(61), py(68), px(70), py(76), shade);
            line(hdc, px(70), py(76), px(83), py(66), shade);
            filled_ellipse(hdc, px(112), py(28), pw(28), ph(16), rgb(72, 173, 121));
        }
        PetMood::FishingMissed => {
            line(hdc, px(57), py(68), px(83), py(68), shade);
            line(hdc, px(102), py(42), px(146), py(72), shade);
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

fn text_fit_in_viewport(
    hdc: HDC,
    x: i32,
    y: i32,
    max_width: i32,
    value: &str,
    color: u32,
    viewport_top: i32,
    viewport_bottom: i32,
) {
    if y < viewport_top || y + CHOICE_SCROLL_LINE_H > viewport_bottom {
        return;
    }
    text_fit(hdc, x, y, max_width, value, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_wrapped_text_limited_in_viewport(
    hdc: HDC,
    x: i32,
    y: i32,
    max_width: i32,
    value: &str,
    color: u32,
    line_height: i32,
    max_lines: usize,
    viewport_top: i32,
    viewport_bottom: i32,
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
        if next_y >= viewport_top && next_y + line_height <= viewport_bottom {
            text(hdc, x, next_y, &line, color);
        }
        next_y += line_height;
    }
    next_y
}

fn wrap_text_to_width(hdc: HDC, value: &str, max_width: i32) -> Vec<String> {
    if max_width <= 0 {
        return vec![value.to_string()];
    }

    let mut lines = Vec::new();
    for raw_line in value.lines() {
        let normalized = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            lines.push(String::new());
            continue;
        }
        wrap_single_line_to_width(hdc, &normalized, max_width, &mut lines);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn point_in_rect(px: i32, py: i32, rect: (i32, i32, i32, i32)) -> bool {
    let (x, y, w, h) = rect;
    px >= x && px < x + w && py >= y && py < y + h
}

fn wrap_single_line_to_width(hdc: HDC, line: &str, max_width: i32, lines: &mut Vec<String>) {
    let mut current = String::new();
    for word in line.split(' ') {
        if current.is_empty() {
            if text_width(hdc, word) <= max_width {
                current.push_str(word);
            } else {
                append_split_word(hdc, word, max_width, lines, &mut current);
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
                append_split_word(hdc, word, max_width, lines, &mut current);
            }
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
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
