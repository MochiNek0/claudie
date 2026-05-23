use windows_sys::Win32::Foundation::{HWND, LRESULT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetStockObject, GetTextExtentPoint32W, HDC, HFONT, PAINTSTRUCT,
    SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetDlgCtrlID};

use crate::settings::mood_rows;
use crate::ui::theme;
use crate::util::wide;

use super::controls::{draw_round_rect, draw_text, fill_rect};
use super::{
    ButtonKind, COLOR_ACCENT, COLOR_BORDER, COLOR_CARD, COLOR_FIELD,
    COLOR_FIELD_BORDER, COLOR_HEADER, COLOR_INK, COLOR_MUTED, ID_CLOSE_SETTINGS, ID_TAB_BASIC,
    ID_TAB_LLM, ID_TAB_POMODORO, SETTINGS_CONTAINER_PAD_X, SETTINGS_GAP, SETTINGS_HEADER_HEIGHT,
    SETTINGS_PANEL_PADDING, SETTINGS_TAB_HEIGHT, SETTINGS_TAB_TOP,
    SettingsPanel, SettingsTab, button_kind, panel,
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
    // Canvas: white sheet. The header band is subtly tinted so it reads as
    // a quieter chrome zone above the prominent content.
    fill_rect(hdc, rect, COLOR_CARD);
    fill_rect(
        hdc,
        &RECT {
            left: 0,
            top: 0,
            right: rect.right,
            bottom: SETTINGS_HEADER_HEIGHT,
        },
        COLOR_HEADER,
    );

    // Hairline + soft shadow to lift the header off the content.
    fill_rect(
        hdc,
        &RECT {
            left: 0,
            top: SETTINGS_HEADER_HEIGHT,
            right: rect.right,
            bottom: SETTINGS_HEADER_HEIGHT + 1,
        },
        COLOR_BORDER,
    );
    fill_rect(
        hdc,
        &RECT {
            left: 0,
            top: SETTINGS_HEADER_HEIGHT + 1,
            right: rect.right,
            bottom: SETTINGS_HEADER_HEIGHT + 2,
        },
        theme::SHADOW_SOFT,
    );

draw_active_fields(hdc, active_tab);
    draw_tab_underline(
        hdc,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X,
        SETTINGS_TAB_TOP + SETTINGS_TAB_HEIGHT,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110,
        active_tab == SettingsTab::Basic,
    );
    draw_tab_underline(
        hdc,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110 + SETTINGS_GAP,
        SETTINGS_TAB_TOP + SETTINGS_TAB_HEIGHT,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110 + SETTINGS_GAP + 124,
        active_tab == SettingsTab::Pomodoro,
    );
    draw_tab_underline(
        hdc,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110 + SETTINGS_GAP + 124 + SETTINGS_GAP,
        SETTINGS_TAB_TOP + SETTINGS_TAB_HEIGHT,
        SETTINGS_PANEL_PADDING
            + SETTINGS_CONTAINER_PAD_X
            + 110
            + SETTINGS_GAP
            + 124
            + SETTINGS_GAP
            + 140,
        active_tab == SettingsTab::Llm,
    );

    // Close button (a faint ghost circle that subtly inverts on hover would
    // require mouse tracking; we keep a calm resting state).
    draw_round_rect(
        hdc,
        rect.right - 56,
        20,
        rect.right - 24,
        52,
        16,
        theme::SURFACE,
        COLOR_BORDER,
    );

    if let Some((title_font, body_font)) = fonts {
        draw_text(hdc, title_font, "claudie Settings", 32, 22, COLOR_INK);
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
        // Inset the underline from both sides so it reads as a refined
        // indicator instead of a hard rule.
        fill_rect(
            hdc,
            &RECT {
                left: left + 4,
                top: bottom + 6,
                right: right - 4,
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
            draw_field(hdc, 48, 508, 338, 62);
            draw_field(hdc, 394, 508, 386, 62);
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
            draw_round_rect(
                hdc,
                48,
                336,
                738,
                402,
                theme::RADIUS_CARD,
                COLOR_FIELD,
                COLOR_FIELD_BORDER,
            );
        }
    }
}

pub(super) unsafe fn draw_field(hdc: HDC, x: i32, y: i32, w: i32, h: i32) {
    draw_round_rect(
        hdc,
        x,
        y,
        x + w,
        y + h,
        theme::RADIUS_FIELD,
        COLOR_FIELD,
        COLOR_FIELD_BORDER,
    );
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

pub(super) unsafe fn draw_button_item(panel: &SettingsPanel, draw: &DRAWITEMSTRUCT) {
    let id = draw.CtlID as usize;
    let pressed = (draw.itemState & ODS_SELECTED) != 0;
    let focused = (draw.itemState & ODS_FOCUS) != 0;
    let disabled = (draw.itemState & ODS_DISABLED) != 0;
    let kind = button_kind(id);
    let (fill, text_color, border) = button_palette(kind, pressed, disabled);
    let rect = draw.rcItem;

    draw_round_rect(
        draw.hDC,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        theme::RADIUS_BUTTON,
        fill,
        border,
    );
    if focused && !pressed {
        draw_round_rect(
            draw.hDC,
            rect.left + 2,
            rect.top + 2,
            rect.right - 2,
            rect.bottom - 2,
            (theme::RADIUS_BUTTON - 2).max(2),
            fill,
            theme::ACCENT,
        );
    }

    let label = button_label(draw.hwndItem);
    if label.is_empty() {
        return;
    }
    let label_w = text_extent(draw.hDC, panel.font_body, &label);
    let cell_w = rect.right - rect.left;
    let cell_h = rect.bottom - rect.top;
    let text_x = rect.left + ((cell_w - label_w).max(0)) / 2;
    // text height = ~font_body cap height; tweak by 1px for visual centering.
    let text_y = rect.top + (cell_h - 16) / 2;
    draw_text(
        draw.hDC,
        panel.font_body,
        &label,
        text_x,
        text_y,
        text_color,
    );
}

fn button_palette(kind: ButtonKind, pressed: bool, disabled: bool) -> (u32, u32, u32) {
    if disabled {
        return (theme::FIELD, theme::MUTED_SOFT, theme::FIELD_BORDER);
    }
    match (kind, pressed) {
        (ButtonKind::Primary, false) => (theme::ACCENT, theme::SURFACE, theme::ACCENT),
        (ButtonKind::Primary, true) => (theme::ACCENT_PRESS, theme::SURFACE, theme::ACCENT_PRESS),
        (ButtonKind::Secondary, false) => (theme::SURFACE, theme::INK, theme::FIELD_BORDER),
        (ButtonKind::Secondary, true) => (theme::FIELD_HOVER, theme::INK, theme::FIELD_BORDER),
    }
}

unsafe fn text_extent(hdc: HDC, font: HFONT, value: &str) -> i32 {
    if font.is_null() {
        return value.chars().count() as i32 * 7;
    }
    let text_wide = wide(value);
    let old_font = SelectObject(hdc, font as _);
    let mut size = SIZE { cx: 0, cy: 0 };
    let count = text_wide.len().saturating_sub(1) as i32;
    let width = if GetTextExtentPoint32W(hdc, text_wide.as_ptr(), count, &mut size) != 0 {
        size.cx
    } else {
        value.chars().count() as i32 * 7
    };
    SelectObject(hdc, old_font);
    width
}

unsafe fn button_label(hwnd: HWND) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buffer = vec![0_u16; len as usize + 1];
    let read = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..read as usize])
}

const STOCK_HOLLOW_BRUSH: i32 = 5;
