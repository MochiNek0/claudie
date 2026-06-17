use windows_sys::Win32::Foundation::{RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, Ellipse, FillRect, GetTextExtentPoint32W, HDC,
    LineTo, MoveToEx, PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    TextOutW,
};

use crate::app::fishing::{FishingPhase, FishingState};
use crate::app::pomodoro::{PomodoroMode, PomodoroState, PomodoroStatus, format_remaining};
use crate::app::{AppState, ClaudeSessionStatus, PetMood, SessionSwitcherItem};
use crate::config::*;
use crate::globals::PET_RENDERER;
use crate::settings::UserSettings;
use crate::ui::theme;
use crate::util::wide;

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

#[derive(Default)]
pub(super) struct RenderState {
    mood: PetMood,
    pub(super) settings: UserSettings,
    pomodoro: PomodoroState,
    fishing: FishingState,
    sessions: Vec<SessionSwitcherItem>,
}

pub(super) fn snapshot_state(state: &AppState) -> RenderState {
    RenderState {
        mood: state.mood,
        settings: state.settings.clone(),
        pomodoro: state.pomodoro.clone(),
        fishing: state.fishing.clone(),
        sessions: state.session_switcher_items(),
    }
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
        let name = if name.is_empty() {
            crate::i18n::strings().session_default_name
        } else {
            name
        };
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
        ClaudeSessionStatus::Error | ClaudeSessionStatus::Denied => rgb(232, 93, 87),
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
        ClaudeSessionStatus::Error | ClaudeSessionStatus::Denied => "x",
        ClaudeSessionStatus::Idle | ClaudeSessionStatus::Done => "-",
        ClaudeSessionStatus::Ended => ".",
    }
}

fn session_status_text(item: &SessionSwitcherItem) -> String {
    let detail = item.detail.trim();
    let s = crate::i18n::strings();
    match item.status {
        ClaudeSessionStatus::Idle | ClaudeSessionStatus::Done => s.session_ready.to_string(),
        ClaudeSessionStatus::Streaming => s.session_thinking.to_string(),
        ClaudeSessionStatus::Tool => {
            if detail.is_empty() {
                s.session_tool.to_string()
            } else {
                detail.to_string()
            }
        }
        ClaudeSessionStatus::WaitingPermission => s.session_permission.to_string(),
        ClaudeSessionStatus::WaitingChoice => s.session_choice.to_string(),
        ClaudeSessionStatus::Compacting => item.status.label().to_ascii_lowercase(),
        ClaudeSessionStatus::Error => {
            if detail.is_empty() {
                s.session_error.to_string()
            } else {
                detail.to_string()
            }
        }
        ClaudeSessionStatus::Denied => {
            if detail.is_empty() {
                s.session_denied.to_string()
            } else {
                detail.to_string()
            }
        }
        ClaudeSessionStatus::Ended => s.session_ended.to_string(),
    }
}

fn draw_fishing_hud(hdc: HDC, state: &RenderState, x: i32, y: i32, hud_w: i32, hud_h: i32) {
    let water = rgb(65, 154, 192);
    let caught = rgb(72, 173, 121);
    let missed = rgb(222, 86, 80);
    let accent = match state.fishing.phase {
        FishingPhase::Caught => caught,
        FishingPhase::Missed => missed,
        _ => water,
    };
    filled_round_rect(
        hdc,
        x,
        y,
        hud_w,
        hud_h,
        theme::RADIUS_FIELD,
        theme::SURFACE,
        theme::HAIRLINE,
    );

    let s = crate::i18n::strings();
    match state.fishing.phase {
        FishingPhase::Waiting => {
            draw_bobber_icon(hdc, x + 18, y + 20, water);
            text_fit(
                hdc,
                x + 46,
                y + 10,
                (hud_w - 58).max(1),
                s.fishing_casting,
                accent,
            );
            text_fit(
                hdc,
                x + 46,
                y + 30,
                (hud_w - 58).max(1),
                s.fishing_waiting_bite,
                theme::INK_MUTED,
            );
        }
        FishingPhase::Reeling => {
            text_fit(hdc, x + 16, y + 7, 70, s.fishing_fish_on, accent);
            text_fit(
                hdc,
                x + 88,
                y + 7,
                (hud_w - 100).max(1),
                s.fishing_tap_reel,
                theme::INK_MUTED,
            );
            draw_fishing_meter(hdc, &state.fishing, x + 16, y + 28, (hud_w - 32).max(1));
            draw_catch_progress(
                hdc,
                x + 16,
                y + 46,
                (hud_w - 32).max(1),
                state.fishing.progress,
            );
        }
        FishingPhase::Caught => {
            draw_fish_icon(hdc, x + 18, y + 22, caught);
            text_fit(
                hdc,
                x + 54,
                y + 11,
                (hud_w - 66).max(1),
                s.fishing_caught,
                accent,
            );
            draw_catch_progress(hdc, x + 54, y + 34, (hud_w - 68).max(1), 1.0);
        }
        FishingPhase::Missed => {
            draw_fish_icon(hdc, x + 18, y + 22, missed);
            text_fit(
                hdc,
                x + 54,
                y + 11,
                (hud_w - 66).max(1),
                s.fishing_escaped,
                accent,
            );
            text_fit(
                hdc,
                x + 54,
                y + 32,
                (hud_w - 66).max(1),
                s.fishing_line_slack,
                theme::INK_MUTED,
            );
        }
        FishingPhase::Inactive => {}
    }
}

fn draw_fishing_meter(hdc: HDC, fishing: &FishingState, x: i32, y: i32, w: i32) {
    const BAR_H: i32 = 12;
    filled_round_rect(
        hdc,
        x,
        y,
        w,
        BAR_H,
        theme::RADIUS_CHIP,
        theme::SURFACE_HOVER,
        theme::HAIRLINE,
    );
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
        rgb(72, 173, 121),
        rgb(72, 173, 121),
    );
    let marker = rgb(245, 174, 64);
    let marker_x = x + (w as f32 * fishing.tension).round() as i32;
    filled_rect(hdc, marker_x - 2, y - 3, 5, BAR_H + 6, marker);
    line(hdc, marker_x - 6, y - 5, marker_x, y - 1, marker);
    line(hdc, marker_x + 6, y - 5, marker_x, y - 1, marker);
}

fn draw_catch_progress(hdc: HDC, x: i32, y: i32, w: i32, progress: f32) {
    let segments = 8;
    let gap = 3;
    let segment_w = ((w - gap * (segments - 1)) / segments).max(1);
    let filled = ((progress.clamp(0.0, 1.0) * segments as f32).ceil() as i32).clamp(0, segments);
    for index in 0..segments {
        let sx = x + index * (segment_w + gap);
        let color = if index < filled {
            theme::ACCENT
        } else {
            rgb(206, 214, 226)
        };
        filled_round_rect(hdc, sx, y, segment_w, 8, theme::RADIUS_CHIP, color, color);
    }
}

fn draw_bobber_icon(hdc: HDC, x: i32, y: i32, water: u32) {
    line(hdc, x + 11, y - 15, x + 11, y - 1, theme::INK_MUTED);
    filled_ellipse(hdc, x, y, 22, 22, rgb(222, 86, 80));
    filled_rect(hdc, x + 1, y + 11, 20, 9, water);
    filled_ellipse(hdc, x + 6, y + 6, 5, 5, rgb(250, 252, 255));
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
        PetMood::Deny => rgb(222, 86, 80),
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
        PetMood::Error | PetMood::Deny => {
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

pub(super) fn filled_round_rect(
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

pub(super) fn filled_ellipse(hdc: HDC, x: i32, y: i32, w: i32, h: i32, color: u32) {
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

fn point_in_rect(px: i32, py: i32, rect: (i32, i32, i32, i32)) -> bool {
    let (x, y, w, h) = rect;
    px >= x && px < x + w && py >= y && py < y + h
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
