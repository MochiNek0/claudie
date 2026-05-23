use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject, FillRect,
    GetStockObject, HDC, HFONT, PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT, TextOutW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CBS_DROPDOWNLIST, CreateWindowExW, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE,
    GetWindowTextLengthW, GetWindowTextW, MB_ICONERROR, MB_OK, MessageBoxW, SendMessageW,
    SetWindowTextW, WM_SETFONT, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};

use crate::ui::theme;
use crate::util::wide;

use super::{
    COLOR_BG, COLOR_CARD, COLOR_FIELD, SettingsPanel, TBM_SETPOS, TBM_SETRANGE, TBM_SETTICFREQ,
    TBS_AUTOTICKS,
};

const SS_CENTER: u32 = 0x0001;
const SS_NOTIFY: u32 = 0x0100;
const SS_CENTERIMAGE: u32 = 0x0200;
const CB_SETITEMHEIGHT_LOCAL: u32 = 0x0153;
pub(super) unsafe fn add_llm_label(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_label);
    panel.llm_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_llm_heading(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_heading);
    panel.llm_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_llm_note(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_body);
    panel.llm_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_llm_edit(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = edit(parent, id, x, y, w, h, value);
    set_control_font(hwnd, panel.font_body);
    panel.llm_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_llm_multiline(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = multiline_edit(parent, id, x, y, w, h, value);
    set_control_font(hwnd, panel.font_body);
    panel.llm_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_llm_combo(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> HWND {
    let hwnd = combo(parent, id, x, y, w, h);
    set_control_font(hwnd, panel.font_body);
    panel.llm_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_llm_button(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
    default: bool,
) -> HWND {
    let hwnd = button(parent, id, x, y, w, h, value, default);
    set_control_font(hwnd, panel.font_body);
    panel.llm_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pet_label(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_label);
    panel.pet_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pet_heading(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_heading);
    panel.pet_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pet_note(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_body);
    panel.pet_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pet_edit(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = edit(parent, id, x, y, w, h, value);
    set_control_font(hwnd, panel.font_body);
    panel.pet_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pet_slider(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: u32,
    min: u32,
    max: u32,
    tick_freq: u32,
) -> HWND {
    let hwnd = slider(parent, id, x, y, w, h, value, min, max, tick_freq);
    set_control_font(hwnd, panel.font_body);
    panel.pet_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pet_button(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
    default: bool,
) -> HWND {
    let hwnd = button(parent, id, x, y, w, h, value, default);
    set_control_font(hwnd, panel.font_body);
    panel.pet_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pomodoro_label(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_label);
    panel.pomodoro_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pomodoro_heading(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_heading);
    panel.pomodoro_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pomodoro_note(
    panel: &mut SettingsPanel,
    parent: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = label(parent, x, y, w, h, value);
    set_control_font(hwnd, panel.font_body);
    panel.pomodoro_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pomodoro_edit(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let hwnd = edit(parent, id, x, y, w, h, value);
    set_control_font(hwnd, panel.font_body);
    panel.pomodoro_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn add_pomodoro_button(
    panel: &mut SettingsPanel,
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
    default: bool,
) -> HWND {
    let hwnd = button(parent, id, x, y, w, h, value, default);
    set_control_font(hwnd, panel.font_body);
    panel.pomodoro_controls.push(hwnd);
    hwnd
}

pub(super) unsafe fn label(parent: HWND, x: i32, y: i32, w: i32, h: i32, value: &str) -> HWND {
    create_control(parent, 0, "STATIC", 0, x, y, w, h, value, 0)
}

pub(super) unsafe fn clickable_label(
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    create_control(
        parent,
        0,
        "STATIC",
        WS_TABSTOP | SS_NOTIFY | SS_CENTER | SS_CENTERIMAGE,
        x,
        y,
        w,
        h,
        value,
        id,
    )
}

pub(super) unsafe fn combo(parent: HWND, id: usize, x: i32, y: i32, w: i32, h: i32) -> HWND {
    let hwnd = create_control(
        parent,
        0,
        "COMBOBOX",
        WS_TABSTOP | CBS_DROPDOWNLIST as u32,
        x,
        y,
        w,
        h,
        "",
        id,
    );
    if !hwnd.is_null() {
        SendMessageW(hwnd, CB_SETITEMHEIGHT_LOCAL, usize::MAX, 28);
        SendMessageW(hwnd, CB_SETITEMHEIGHT_LOCAL, 0, 28);
    }
    hwnd
}

pub(super) unsafe fn slider(
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: u32,
    min: u32,
    max: u32,
    tick_freq: u32,
) -> HWND {
    let hwnd = create_control(
        parent,
        0,
        "msctls_trackbar32",
        WS_TABSTOP | TBS_AUTOTICKS,
        x,
        y,
        w,
        h,
        "",
        id,
    );
    if !hwnd.is_null() {
        let range = ((max as isize) << 16) | min as isize;
        SendMessageW(hwnd, TBM_SETRANGE, 1, range);
        SendMessageW(hwnd, TBM_SETTICFREQ, tick_freq as usize, 0);
        SendMessageW(hwnd, TBM_SETPOS, 1, value.clamp(min, max) as isize);
    }
    hwnd
}

pub(super) unsafe fn edit(
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    let inner_h = (h - 12).max(20);
    let inner_y = y + (h - inner_h) / 2;
    create_control(
        parent,
        0,
        "EDIT",
        WS_TABSTOP | ES_AUTOHSCROLL as u32,
        x + 10,
        inner_y,
        w - 20,
        inner_h,
        value,
        id,
    )
}

pub(super) unsafe fn multiline_edit(
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
) -> HWND {
    create_control(
        parent,
        0,
        "EDIT",
        WS_TABSTOP | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32,
        x + 10,
        y + 6,
        w - 20,
        h - 12,
        value,
        id,
    )
}

pub(super) unsafe fn button(
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
    _default: bool,
) -> HWND {
    // All settings buttons are owner-drawn so primary / secondary chrome can
    // match the rest of the panel; the kind is resolved from the control id
    // when WM_DRAWITEM fires.
    create_control(
        parent,
        0,
        "BUTTON",
        WS_TABSTOP | BS_OWNERDRAW as u32,
        x,
        y,
        w,
        h,
        value,
        id,
    )
}

pub(super) unsafe fn create_control(
    parent: HWND,
    ex_style: u32,
    class: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
    id: usize,
) -> HWND {
    let class = wide(class);
    let value = wide(value);
    let hwnd = CreateWindowExW(
        ex_style,
        class.as_ptr(),
        value.as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        parent,
        id as isize as _,
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    if !hwnd.is_null() {
        SendMessageW(
            hwnd,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            1,
        );
    }
    hwnd
}

pub(super) unsafe fn create_style_resources(panel: &mut SettingsPanel) {
    panel.font_title = create_font(theme::FONT_TITLE_PX, theme::WEIGHT_SEMIBOLD);
    panel.font_heading = create_font(theme::FONT_HEADING_PX, theme::WEIGHT_SEMIBOLD);
    panel.font_body = create_font(theme::FONT_BODY_PX, theme::WEIGHT_REGULAR);
    panel.font_label = create_font(theme::FONT_LABEL_PX, theme::WEIGHT_SEMIBOLD);
    panel.brush_bg = CreateSolidBrush(COLOR_BG);
    panel.brush_card = CreateSolidBrush(COLOR_CARD);
    panel.brush_field = CreateSolidBrush(COLOR_FIELD);
}

pub(super) unsafe fn create_font(height: i32, weight: i32) -> HFONT {
    let face = wide(theme::FONT_FACE);
    CreateFontW(
        -height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        face.as_ptr(),
    )
}

pub(super) unsafe fn set_control_font(hwnd: HWND, font: HFONT) {
    if !hwnd.is_null() && !font.is_null() {
        SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
    }
}

pub(super) unsafe fn delete_object<T>(handle: *mut T) {
    if !handle.is_null() {
        DeleteObject(handle as _);
    }
}

pub(super) unsafe fn fill_rect(hdc: HDC, rect: &RECT, color: u32) {
    let brush = CreateSolidBrush(color);
    if !brush.is_null() {
        FillRect(hdc, rect, brush);
        DeleteObject(brush as _);
    }
}

pub(super) unsafe fn draw_round_rect(
    hdc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
    fill: u32,
    border: u32,
) {
    let brush = CreateSolidBrush(fill);
    let pen = CreatePen(PS_SOLID, 1, border);
    if brush.is_null() || pen.is_null() {
        delete_object(brush);
        delete_object(pen);
        return;
    }

    let old_brush = SelectObject(hdc, brush as _);
    let old_pen = SelectObject(hdc, pen as _);
    RoundRect(hdc, left, top, right, bottom, radius, radius);
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    DeleteObject(pen as _);
    DeleteObject(brush as _);
}

pub(super) unsafe fn draw_text(hdc: HDC, font: HFONT, text: &str, x: i32, y: i32, color: u32) {
    if font.is_null() {
        return;
    }
    let text_wide = wide(text);
    let old_font = SelectObject(hdc, font as _);
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(hdc, color);
    TextOutW(
        hdc,
        x,
        y,
        text_wide.as_ptr(),
        text_wide.len().saturating_sub(1) as i32,
    );
    SelectObject(hdc, old_font);
}

pub(super) unsafe fn text_value(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buffer = vec![0_u16; len as usize + 1];
    let read = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..read as usize])
        .trim()
        .to_string()
}

pub(super) unsafe fn set_text(hwnd: HWND, value: &str) {
    let value = wide(value);
    SetWindowTextW(hwnd, value.as_ptr());
}

pub(super) unsafe fn message(hwnd: HWND, title: &str, body: &str) {
    let title = wide(title);
    let body = wide(body);
    MessageBoxW(hwnd, body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
}
