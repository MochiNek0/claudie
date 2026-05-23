use windows_sys::Win32::Foundation::{HWND, LRESULT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetStockObject, HDC, HFONT, PAINTSTRUCT, SetBkColor, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetDlgCtrlID};

use crate::settings::mood_rows;

use super::controls::{draw_round_rect, draw_text, fill_rect};
use super::{
    COLOR_ACCENT, COLOR_ACCENT_SOFT, COLOR_BORDER, COLOR_CARD, COLOR_FIELD, COLOR_FIELD_BORDER,
    COLOR_HEADER, COLOR_INK, COLOR_MUTED, ID_CLOSE_SETTINGS, ID_TAB_BASIC, ID_TAB_LLM,
    ID_TAB_POMODORO, SETTINGS_CONTAINER_PAD_X, SETTINGS_GAP, SETTINGS_PANEL_PADDING,
    SETTINGS_PANEL_RADIUS, SettingsTab, panel,
};

pub(super) unsafe fn paint_settings(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rect = RECT::default();
    GetClientRect(hwnd, &mut rect);

    let active_tab = panel(hwnd)
        .map(|panel| panel.active_tab)
        .unwrap_or(SettingsTab::Llm);
    let fonts = panel(hwnd).map(|panel| (panel.font_title, panel.font_body));

    draw_settings_frame(hdc, &rect, active_tab, fonts);
    EndPaint(hwnd, &ps);
}

pub(super) unsafe fn draw_settings_frame(
    hdc: HDC,
    rect: &RECT,
    active_tab: SettingsTab,
    fonts: Option<(HFONT, HFONT)>,
) {
    fill_rect(hdc, rect, COLOR_CARD);
    fill_rect(
        hdc,
        &RECT {
            left: 0,
            top: 0,
            right: rect.right,
            bottom: 118,
        },
        COLOR_HEADER,
    );
    fill_rect(
        hdc,
        &RECT {
            left: 0,
            top: 118,
            right: rect.right,
            bottom: 119,
        },
        COLOR_BORDER,
    );
    draw_active_fields(hdc, active_tab);
    draw_tab_underline(
        hdc,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X,
        110,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110,
        active_tab == SettingsTab::Basic,
    );
    draw_tab_underline(
        hdc,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110 + SETTINGS_GAP,
        110,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110 + SETTINGS_GAP + 124,
        active_tab == SettingsTab::Pomodoro,
    );
    draw_tab_underline(
        hdc,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110 + SETTINGS_GAP + 124 + SETTINGS_GAP,
        110,
        SETTINGS_PANEL_PADDING
            + SETTINGS_CONTAINER_PAD_X
            + 110
            + SETTINGS_GAP
            + 124
            + SETTINGS_GAP
            + 140,
        active_tab == SettingsTab::Llm,
    );
    draw_round_rect(
        hdc,
        rect.right - 54,
        22,
        rect.right - 24,
        52,
        SETTINGS_PANEL_RADIUS * 2,
        COLOR_FIELD,
        COLOR_ACCENT_SOFT,
    );

    if let Some((title_font, body_font)) = fonts {
        draw_text(hdc, title_font, "claudie Settings", 32, 20, COLOR_INK);
        draw_text(
            hdc,
            body_font,
            "A compact control room for model profiles and pet moods.",
            32,
            52,
            COLOR_MUTED,
        );
    }
}

pub(super) unsafe fn draw_tab_underline(
    hdc: HDC,
    left: i32,
    bottom: i32,
    right: i32,
    active: bool,
) {
    if active {
        fill_rect(
            hdc,
            &RECT {
                left,
                top: bottom + 7,
                right,
                bottom: bottom + 9,
            },
            COLOR_ACCENT,
        );
    }
}

pub(super) unsafe fn draw_active_fields(hdc: HDC, active_tab: SettingsTab) {
    match active_tab {
        SettingsTab::Llm => {
            draw_field(hdc, 48, 276, 338, 36);
            draw_field(hdc, 394, 276, 386, 36);
            draw_field(hdc, 48, 334, 338, 36);
            draw_field(hdc, 394, 334, 386, 36);
            draw_field(hdc, 48, 392, 338, 36);
            draw_field(hdc, 394, 392, 386, 36);
            draw_field(hdc, 48, 450, 238, 36);
            draw_field(hdc, 294, 450, 238, 36);
            draw_field(hdc, 540, 450, 240, 36);
            draw_field(hdc, 48, 508, 732, 62);
        }
        SettingsTab::Basic => {
            draw_field(hdc, 48, 218, 384, 48);
            draw_field(hdc, 448, 218, 384, 48);
            draw_field(hdc, 48, 294, 384, 36);
            draw_field(hdc, 448, 294, 384, 36);
            for index in 0..mood_rows().len() {
                let col = index % 2;
                let row = index / 2;
                let x = if col == 0 { 48 } else { 448 };
                let y = 332 + row as i32 * 54;
                draw_field(hdc, x, y + 20, 384, 32);
            }
        }
        SettingsTab::Pomodoro => {
            draw_field(hdc, 48, 234, 120, 36);
            draw_field(hdc, 176, 234, 120, 36);
            draw_field(hdc, 304, 234, 120, 36);
            draw_round_rect(hdc, 48, 336, 738, 402, 18, COLOR_FIELD, COLOR_FIELD_BORDER);
        }
    }
}

pub(super) unsafe fn draw_field(hdc: HDC, x: i32, y: i32, w: i32, h: i32) {
    draw_round_rect(hdc, x, y, x + w, y + h, 16, COLOR_FIELD, COLOR_FIELD_BORDER);
}

pub(super) unsafe fn color_static(parent: HWND, child: HWND, hdc: HDC) -> LRESULT {
    if let Some(panel) = panel(parent) {
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, static_text_color(child, panel.active_tab));
        if child == panel.pet_scale_slider || child == panel.sleep_after_slider {
            SetBkColor(hdc, COLOR_FIELD);
            return panel.brush_field as LRESULT;
        }
        GetStockObject(STOCK_HOLLOW_BRUSH) as LRESULT
    } else {
        0
    }
}

pub(super) unsafe fn color_field(hwnd: HWND, hdc: HDC) -> LRESULT {
    if let Some(panel) = panel(hwnd) {
        SetBkColor(hdc, COLOR_FIELD);
        SetTextColor(hdc, COLOR_INK);
        panel.brush_field as LRESULT
    } else {
        0
    }
}

pub(super) unsafe fn color_button(hwnd: HWND, hdc: HDC) -> LRESULT {
    if let Some(panel) = panel(hwnd) {
        SetBkColor(hdc, COLOR_CARD);
        SetTextColor(hdc, COLOR_INK);
        panel.brush_card as LRESULT
    } else {
        0
    }
}

pub(super) unsafe fn static_text_color(child: HWND, active_tab: SettingsTab) -> u32 {
    match GetDlgCtrlID(child) as usize {
        ID_TAB_BASIC => tab_text_color(active_tab == SettingsTab::Basic),
        ID_TAB_POMODORO => tab_text_color(active_tab == SettingsTab::Pomodoro),
        ID_TAB_LLM => tab_text_color(active_tab == SettingsTab::Llm),
        ID_CLOSE_SETTINGS => COLOR_MUTED,
        _ => COLOR_INK,
    }
}

pub(super) fn tab_text_color(active: bool) -> u32 {
    if active { COLOR_ACCENT } else { COLOR_MUTED }
}

const STOCK_HOLLOW_BRUSH: i32 = 5;
