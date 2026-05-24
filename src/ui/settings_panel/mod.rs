mod controls;
mod paint;

use controls::*;
use paint::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, HBRUSH, HDC, HFONT, InvalidateRect, ScreenToClient,
    SetWindowRgn,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    FindWindowW, GWLP_USERDATA, GetDlgCtrlID, GetSystemMetrics, GetWindowLongPtrW, HTCAPTION,
    HTCLIENT, ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_HAND, LoadCursorW, LoadIconW, RegisterClassW,
    SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOW, SendMessageW, SetCursor, SetForegroundWindow,
    SetTimer, SetWindowLongPtrW, ShowWindow, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN,
    WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND,
    WM_HSCROLL, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WM_SETICON, WM_TIMER, WNDCLASSW, WS_POPUP,
    WS_VISIBLE,
};

use crate::app::pomodoro::{PomodoroMode, PomodoroStatus, format_remaining};
use crate::app::{AppState, PetMood};
use crate::config::{
    PET_SCALE_MAX_PERCENT, PET_SCALE_MIN_PERCENT, POMODORO_MAX_MINUTES, POMODORO_MIN_MINUTES,
};
use crate::globals::APP_STATE;
use crate::settings::{
    AnimationSettings, LlmProfile, LlmProfileDb, UserSettings, apply_llm_profile_to_claude,
    current_claude_llm_profile, default_profile_id, ensure_claude_onboarding_complete,
    load_llm_profile_db, load_user_settings, mood_rows, save_llm_profile_db, save_user_settings,
};
use crate::ui::gif_animation::reload_animation_store;
use crate::ui::theme;
use crate::util::wide;

const CLASS_NAME: &str = "ClaudieSettingsWindow";
const ID_TAB_LLM: usize = 1900;
const ID_TAB_BASIC: usize = 1901;
const ID_TAB_POMODORO: usize = 1903;
const ID_CLOSE_SETTINGS: usize = 1904;
const ID_PROFILE_COMBO: usize = 2000;
const ID_PROFILE_ID: usize = 2001;
const ID_PROFILE_NAME: usize = 2002;
const ID_BASE_URL: usize = 2003;
const ID_AUTH_TOKEN: usize = 2004;
const ID_API_KEY: usize = 2005;
const ID_MODEL: usize = 2006;
const ID_OPUS_MODEL: usize = 2007;
const ID_SONNET_MODEL: usize = 2008;
const ID_HAIKU_MODEL: usize = 2009;
const ID_EXTRA_ENV: usize = 2010;
const ID_OPENAI_EXTRA_BODY: usize = 2011;
const ID_PET_SCALE: usize = 2090;
const ID_POMODORO_FOCUS: usize = 2091;
const ID_POMODORO_SHORT_BREAK: usize = 2092;
const ID_POMODORO_LONG_BREAK: usize = 2093;
const ID_SLEEP_AFTER: usize = 2094;
const ID_PET_DIR: usize = 2100;
const ID_GIF_DIR: usize = 2101;
const ID_ANIMATION_BASE: usize = 2200;
const ID_PROFILE_NEW: usize = 3001;
const ID_PROFILE_SAVE: usize = 3002;
const ID_PROFILE_USE: usize = 3003;
const ID_PROFILE_IMPORT: usize = 3004;
const ID_SAVE_PET: usize = 3005;
const ID_RESET_PET: usize = 3006;
const ID_PROFILE_DELETE: usize = 3007;
const ID_SAVE_POMODORO: usize = 3020;
const ID_START_POMODORO: usize = 3021;
const ID_PAUSE_RESUME_POMODORO: usize = 3022;
const ID_SKIP_POMODORO: usize = 3023;
const ID_STOP_POMODORO: usize = 3024;
const CB_ADDSTRING: u32 = 0x0143;
const CB_RESETCONTENT: u32 = 0x014B;
const CB_GETCURSEL: u32 = 0x0147;
const CB_SETCURSEL: u32 = 0x014E;
const CBN_SELCHANGE: u16 = 1;
const TBM_GETPOS: u32 = 0x0400;
const TBM_SETPOS: u32 = 0x0405;
const TBM_SETRANGE: u32 = 0x0406;
const TBM_SETTICFREQ: u32 = 0x0414;
const TB_THUMBTRACK: u16 = 5;
const TBS_AUTOTICKS: u32 = 0x0001;
const SETTINGS_WIDTH: i32 = 880;
const SETTINGS_HEIGHT: i32 = 700;
const SETTINGS_PANEL_PADDING: i32 = 16;
const SETTINGS_CONTAINER_PAD_X: i32 = 12;
const SETTINGS_CONTAINER_PAD_Y: i32 = 8;
const SETTINGS_GAP: i32 = 8;
const SETTINGS_PANEL_RADIUS: i32 = theme::RADIUS_WINDOW;
const SETTINGS_HEADER_HEIGHT: i32 = 118;
const SETTINGS_TAB_HEIGHT: i32 = 34;
const SETTINGS_TAB_TOP: i32 = 76;
const SETTINGS_DYNAMIC_TIMER_ID: usize = 1;

const COLOR_BG: u32 = theme::BG;
const COLOR_HEADER: u32 = theme::SURFACE_ALT;
const COLOR_CARD: u32 = theme::SURFACE;
const COLOR_FIELD: u32 = theme::FIELD;
const COLOR_BORDER: u32 = theme::HAIRLINE;
const COLOR_FIELD_BORDER: u32 = theme::FIELD_BORDER;
const COLOR_ACCENT: u32 = theme::ACCENT;
const COLOR_INK: u32 = theme::INK;
const COLOR_MUTED: u32 = theme::MUTED;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Basic,
    Llm,
    Pomodoro,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ButtonKind {
    Primary,
    Secondary,
}

pub(super) fn button_kind(id: usize) -> ButtonKind {
    match id {
        ID_PROFILE_USE | ID_SAVE_PET | ID_SAVE_POMODORO | ID_START_POMODORO => ButtonKind::Primary,
        _ => ButtonKind::Secondary,
    }
}

pub(crate) unsafe fn show_settings_panel(_parent: HWND) {
    show_settings_panel_tab(SettingsTab::Basic);
}

unsafe fn show_settings_panel_tab(tab: SettingsTab) {
    register_class();
    init_common_controls();

    let class_name = wide(CLASS_NAME);
    let existing = FindWindowW(class_name.as_ptr(), std::ptr::null());
    if !existing.is_null() {
        switch_tab(existing, tab);
        ShowWindow(existing, SW_SHOW);
        SetForegroundWindow(existing);
        return;
    }

    let settings = Box::new(load_user_settings());
    let title = wide("claudie Settings");
    let x = (GetSystemMetrics(SM_CXSCREEN) - SETTINGS_WIDTH).max(0) / 2;
    let y = (GetSystemMetrics(SM_CYSCREEN) - SETTINGS_HEIGHT).max(0) / 2;
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_POPUP | WS_VISIBLE,
        x,
        y,
        SETTINGS_WIDTH,
        SETTINGS_HEIGHT,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        Box::into_raw(settings) as *mut _,
    );

    if !hwnd.is_null() {
        apply_settings_window_chrome(hwnd);
        switch_tab(hwnd, tab);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}

unsafe fn init_common_controls() {
    let mut controls = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_BAR_CLASSES,
    };
    InitCommonControlsEx(&mut controls);
}

unsafe fn register_class() {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        let class_name = wide(CLASS_NAME);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(settings_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hIcon: unsafe { load_app_icon() },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        unsafe {
            RegisterClassW(&wc);
        }
    });
}

unsafe fn load_app_icon() -> *mut std::ffi::c_void {
    LoadIconW(GetModuleHandleW(std::ptr::null()), 1_usize as *const u16)
}

unsafe fn apply_settings_window_chrome(hwnd: HWND) {
    let icon = load_app_icon();
    if !icon.is_null() {
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
    }

    let region = CreateRoundRectRgn(
        0,
        0,
        SETTINGS_WIDTH,
        SETTINGS_HEIGHT,
        SETTINGS_PANEL_RADIUS * 2,
        SETTINGS_PANEL_RADIUS * 2,
    );
    if !region.is_null() && SetWindowRgn(hwnd, region, 1) == 0 {
        DeleteObject(region as _);
    }
}

unsafe fn is_pointer_control(hwnd: HWND) -> bool {
    let id = GetDlgCtrlID(hwnd);
    matches!(
        id as usize,
        ID_TAB_LLM
            | ID_TAB_BASIC
            | ID_TAB_POMODORO
            | ID_CLOSE_SETTINGS
            | ID_PROFILE_COMBO
            | ID_PET_SCALE
            | ID_SLEEP_AFTER
            | ID_PROFILE_NEW
            | ID_PROFILE_SAVE
            | ID_PROFILE_USE
            | ID_PROFILE_IMPORT
            | ID_PROFILE_DELETE
            | ID_SAVE_PET
            | ID_RESET_PET
            | ID_SAVE_POMODORO
            | ID_START_POMODORO
            | ID_PAUSE_RESUME_POMODORO
            | ID_SKIP_POMODORO
            | ID_STOP_POMODORO
    )
}

unsafe fn is_in_drag_region(hwnd: HWND, lparam: LPARAM) -> bool {
    let mut point = POINT {
        x: signed_loword(lparam),
        y: signed_hiword(lparam),
    };
    if ScreenToClient(hwnd, &mut point) == 0 {
        return false;
    }
    point.y >= 0 && point.y < 72
}

fn signed_loword(value: LPARAM) -> i32 {
    (value as u32 & 0xffff) as i16 as i32
}

fn signed_hiword(value: LPARAM) -> i32 {
    ((value as u32 >> 16) & 0xffff) as i16 as i32
}

unsafe extern "system" fn settings_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let createstruct = lparam as *const CREATESTRUCTW;
            let settings = Box::from_raw((*createstruct).lpCreateParams as *mut UserSettings);
            let mut panel = Box::new(SettingsPanel::new(*settings, load_llm_profile_db()));
            create_controls(hwnd, &mut panel);
            refresh_profile_combo(&panel);
            load_selected_profile(&panel);
            refresh_pomodoro_tab(&panel);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(panel) as isize);
            apply_settings_window_chrome(hwnd);
            SetTimer(hwnd, SETTINGS_DYNAMIC_TIMER_ID, 1000, None);
            0
        }
        WM_PAINT => {
            paint_settings(hwnd);
            0
        }
        WM_DRAWITEM => {
            let draw = lparam as *const DRAWITEMSTRUCT;
            if !draw.is_null()
                && let Some(panel) = panel(hwnd)
            {
                draw_button_item(panel, &*draw);
                return 1;
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_CTLCOLORSTATIC => color_static(hwnd, lparam as HWND, wparam as HDC),
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => color_field(hwnd, wparam as HDC),
        WM_CTLCOLORBTN => color_button(hwnd, wparam as HDC),
        WM_HSCROLL => {
            if let Some(panel) = panel(hwnd) {
                if lparam == panel.pet_scale_slider as isize {
                    update_pet_scale_from_slider(hwnd, panel, loword(wparam) != TB_THUMBTRACK);
                    InvalidateRect(
                        hwnd,
                        &RECT {
                            left: 48,
                            top: 218,
                            right: 432,
                            bottom: 266,
                        },
                        0,
                    );
                    InvalidateRect(panel.pet_scale_slider, std::ptr::null(), 1);
                    return 0;
                }
                if lparam == panel.sleep_after_slider as isize {
                    update_sleep_after_from_slider(hwnd, panel, loword(wparam) != TB_THUMBTRACK);
                    InvalidateRect(
                        hwnd,
                        &RECT {
                            left: 448,
                            top: 218,
                            right: 832,
                            bottom: 266,
                        },
                        0,
                    );
                    InvalidateRect(panel.sleep_after_slider, std::ptr::null(), 1);
                    return 0;
                }
            }
            0
        }
        WM_SETCURSOR => {
            if is_pointer_control(wparam as HWND) {
                SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            let hit = DefWindowProcW(hwnd, msg, wparam, lparam);
            if hit == HTCLIENT as isize && is_in_drag_region(hwnd, lparam) {
                return HTCAPTION as isize;
            }
            hit
        }
        WM_TIMER => {
            if wparam == SETTINGS_DYNAMIC_TIMER_ID {
                if let Some(panel) = panel(hwnd) {
                    if panel.active_tab == SettingsTab::Pomodoro {
                        refresh_pomodoro_tab(panel);
                    }
                }
            }
            0
        }
        WM_COMMAND => {
            let command = loword(wparam);
            let notify = hiword(wparam);
            if command == ID_PROFILE_COMBO as u16 && notify == CBN_SELCHANGE {
                if let Some(panel) = panel(hwnd) {
                    load_selected_profile(panel);
                }
                return 0;
            }
            if command == ID_TAB_LLM as u16 {
                switch_tab(hwnd, SettingsTab::Llm);
                return 0;
            }
            if command == ID_TAB_BASIC as u16 {
                switch_tab(hwnd, SettingsTab::Basic);
                return 0;
            }
            if command == ID_TAB_POMODORO as u16 {
                switch_tab(hwnd, SettingsTab::Pomodoro);
                return 0;
            }
            if command == ID_CLOSE_SETTINGS as u16 {
                DestroyWindow(hwnd);
                return 0;
            }
            if command == ID_PROFILE_NEW as u16 {
                new_profile(hwnd);
                return 0;
            }
            if command == ID_PROFILE_SAVE as u16 {
                save_profile_from_button(hwnd);
                return 0;
            }
            if command == ID_PROFILE_USE as u16 {
                use_profile(hwnd);
                return 0;
            }
            if command == ID_PROFILE_IMPORT as u16 {
                import_current_profile(hwnd);
                return 0;
            }
            if command == ID_PROFILE_DELETE as u16 {
                delete_profile(hwnd);
                return 0;
            }
            if command == ID_SAVE_PET as u16 {
                save_basic_settings(hwnd);
                return 0;
            }
            if command == ID_RESET_PET as u16 {
                reset_pet_fields(hwnd);
                return 0;
            }
            if command == ID_SAVE_POMODORO as u16 {
                save_pomodoro_settings(hwnd);
                return 0;
            }
            if command == ID_START_POMODORO as u16 {
                mutate_app_state(|state| state.start_pomodoro());
                refresh_settings_dynamic(hwnd);
                return 0;
            }
            if command == ID_PAUSE_RESUME_POMODORO as u16 {
                mutate_app_state(|state| {
                    if state.pomodoro.status == PomodoroStatus::Paused {
                        state.resume_pomodoro();
                    } else {
                        state.pause_pomodoro();
                    }
                });
                refresh_settings_dynamic(hwnd);
                return 0;
            }
            if command == ID_SKIP_POMODORO as u16 {
                mutate_app_state(|state| state.skip_pomodoro());
                refresh_settings_dynamic(hwnd);
                return 0;
            }
            if command == ID_STOP_POMODORO as u16 {
                mutate_app_state(|state| state.stop_pomodoro());
                refresh_settings_dynamic(hwnd);
                return 0;
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsPanel;
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

struct SettingsPanel {
    settings: UserSettings,
    llm_db: LlmProfileDb,
    active_tab: SettingsTab,
    font_title: HFONT,
    font_heading: HFONT,
    font_body: HFONT,
    font_label: HFONT,
    brush_bg: HBRUSH,
    brush_card: HBRUSH,
    brush_field: HBRUSH,
    tab_llm: HWND,
    tab_basic: HWND,
    tab_pomodoro: HWND,
    pet_scale_slider: HWND,
    pet_scale_value: HWND,
    sleep_after_slider: HWND,
    sleep_after_value: HWND,
    pomodoro_focus: HWND,
    pomodoro_short_break: HWND,
    pomodoro_long_break: HWND,
    pomodoro_status: HWND,
    pomodoro_pause_resume: HWND,
    profile_combo: HWND,
    profile_id: HWND,
    profile_name: HWND,
    base_url: HWND,
    auth_token: HWND,
    api_key: HWND,
    model: HWND,
    opus_model: HWND,
    sonnet_model: HWND,
    haiku_model: HWND,
    openai_extra_body: HWND,
    extra_env: HWND,
    pet_dir: HWND,
    gif_dir: HWND,
    animation_edits: Vec<(PetMood, HWND)>,
    llm_controls: Vec<HWND>,
    pet_controls: Vec<HWND>,
    pomodoro_controls: Vec<HWND>,
}

impl SettingsPanel {
    fn new(settings: UserSettings, llm_db: LlmProfileDb) -> Self {
        Self {
            settings,
            llm_db,
            active_tab: SettingsTab::Llm,
            font_title: std::ptr::null_mut(),
            font_heading: std::ptr::null_mut(),
            font_body: std::ptr::null_mut(),
            font_label: std::ptr::null_mut(),
            brush_bg: std::ptr::null_mut(),
            brush_card: std::ptr::null_mut(),
            brush_field: std::ptr::null_mut(),
            tab_llm: std::ptr::null_mut(),
            tab_basic: std::ptr::null_mut(),
            tab_pomodoro: std::ptr::null_mut(),
            pet_scale_slider: std::ptr::null_mut(),
            pet_scale_value: std::ptr::null_mut(),
            sleep_after_slider: std::ptr::null_mut(),
            sleep_after_value: std::ptr::null_mut(),
            pomodoro_focus: std::ptr::null_mut(),
            pomodoro_short_break: std::ptr::null_mut(),
            pomodoro_long_break: std::ptr::null_mut(),
            pomodoro_status: std::ptr::null_mut(),
            pomodoro_pause_resume: std::ptr::null_mut(),
            profile_combo: std::ptr::null_mut(),
            profile_id: std::ptr::null_mut(),
            profile_name: std::ptr::null_mut(),
            base_url: std::ptr::null_mut(),
            auth_token: std::ptr::null_mut(),
            api_key: std::ptr::null_mut(),
            model: std::ptr::null_mut(),
            opus_model: std::ptr::null_mut(),
            sonnet_model: std::ptr::null_mut(),
            haiku_model: std::ptr::null_mut(),
            openai_extra_body: std::ptr::null_mut(),
            extra_env: std::ptr::null_mut(),
            pet_dir: std::ptr::null_mut(),
            gif_dir: std::ptr::null_mut(),
            animation_edits: Vec::new(),
            llm_controls: Vec::new(),
            pet_controls: Vec::new(),
            pomodoro_controls: Vec::new(),
        }
    }
}

impl Drop for SettingsPanel {
    fn drop(&mut self) {
        unsafe {
            delete_object(self.font_title);
            delete_object(self.font_heading);
            delete_object(self.font_body);
            delete_object(self.font_label);
            delete_object(self.brush_bg);
            delete_object(self.brush_card);
            delete_object(self.brush_field);
        }
    }
}

unsafe fn create_controls(hwnd: HWND, panel: &mut SettingsPanel) {
    create_style_resources(panel);

    let close = clickable_label(
        hwnd,
        ID_CLOSE_SETTINGS,
        SETTINGS_WIDTH - 56,
        20,
        32,
        32,
        "\u{00d7}",
    );
    set_control_font(close, panel.font_heading);

    panel.tab_basic = clickable_label(
        hwnd,
        ID_TAB_BASIC,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X,
        SETTINGS_TAB_TOP,
        110,
        SETTINGS_TAB_HEIGHT,
        "Basic",
    );
    set_control_font(panel.tab_basic, panel.font_heading);
    panel.tab_pomodoro = clickable_label(
        hwnd,
        ID_TAB_POMODORO,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110 + SETTINGS_GAP,
        SETTINGS_TAB_TOP,
        124,
        SETTINGS_TAB_HEIGHT,
        "Pomodoro",
    );
    set_control_font(panel.tab_pomodoro, panel.font_heading);
    panel.tab_llm = clickable_label(
        hwnd,
        ID_TAB_LLM,
        SETTINGS_PANEL_PADDING + SETTINGS_CONTAINER_PAD_X + 110 + SETTINGS_GAP + 124 + SETTINGS_GAP,
        SETTINGS_TAB_TOP,
        140,
        SETTINGS_TAB_HEIGHT,
        "LLM Profiles",
    );
    set_control_font(panel.tab_llm, panel.font_heading);

    add_llm_heading(panel, hwnd, 48, 138, 240, 26, "Provider profiles");
    add_llm_note(
        panel,
        hwnd,
        48,
        164,
        620,
        20,
        "Keep Claude Code provider settings tidy without leaving the pet.",
    );

    add_llm_label(panel, hwnd, 48, 198, 88, 16, "Profile");
    panel.profile_combo = add_llm_combo(panel, hwnd, ID_PROFILE_COMBO, 48, 218, 288, 220);
    add_llm_button(panel, hwnd, ID_PROFILE_NEW, 352, 217, 76, 29, "New", false);
    add_llm_button(
        panel,
        hwnd,
        ID_PROFILE_IMPORT,
        436,
        217,
        132,
        29,
        "Import Current",
        false,
    );
    add_llm_button(
        panel,
        hwnd,
        ID_PROFILE_DELETE,
        576,
        217,
        84,
        29,
        "Delete",
        false,
    );

    add_llm_label(panel, hwnd, 48, 256, 118, 16, "Profile ID");
    panel.profile_id = add_llm_edit(panel, hwnd, ID_PROFILE_ID, 48, 276, 338, 36, "");
    add_llm_label(panel, hwnd, 394, 256, 118, 16, "Name");
    panel.profile_name = add_llm_edit(panel, hwnd, ID_PROFILE_NAME, 394, 276, 386, 36, "");

    add_llm_label(panel, hwnd, 48, 314, 118, 16, "Model");
    panel.model = add_llm_edit(panel, hwnd, ID_MODEL, 48, 334, 338, 36, "");
    add_llm_label(panel, hwnd, 394, 314, 118, 16, "Base URL");
    panel.base_url = add_llm_edit(panel, hwnd, ID_BASE_URL, 394, 334, 386, 36, "");

    add_llm_label(panel, hwnd, 48, 372, 118, 16, "API key");
    panel.api_key = add_llm_edit(panel, hwnd, ID_API_KEY, 48, 392, 338, 36, "");
    add_llm_label(panel, hwnd, 394, 372, 170, 16, "Auth token (proxy)");
    panel.auth_token = add_llm_edit(panel, hwnd, ID_AUTH_TOKEN, 394, 392, 386, 36, "");

    add_llm_label(panel, hwnd, 48, 430, 118, 16, "Opus");
    panel.opus_model = add_llm_edit(panel, hwnd, ID_OPUS_MODEL, 48, 450, 238, 36, "");
    add_llm_label(panel, hwnd, 294, 430, 118, 16, "Sonnet");
    panel.sonnet_model = add_llm_edit(panel, hwnd, ID_SONNET_MODEL, 294, 450, 238, 36, "");
    add_llm_label(panel, hwnd, 540, 430, 118, 16, "Haiku");
    panel.haiku_model = add_llm_edit(panel, hwnd, ID_HAIKU_MODEL, 540, 450, 240, 36, "");

    add_llm_label(panel, hwnd, 48, 488, 118, 16, "Extra env");
    panel.extra_env = add_llm_multiline(panel, hwnd, ID_EXTRA_ENV, 48, 508, 338, 62, "");
    add_llm_label(panel, hwnd, 394, 488, 170, 16, "OpenAI body");
    panel.openai_extra_body =
        add_llm_multiline(panel, hwnd, ID_OPENAI_EXTRA_BODY, 394, 508, 386, 62, "");

    add_llm_button(
        panel,
        hwnd,
        ID_PROFILE_SAVE,
        624,
        612,
        96,
        32,
        "Save",
        false,
    );
    add_llm_button(panel, hwnd, ID_PROFILE_USE, 730, 612, 96, 32, "Use", true);

    add_pet_heading(panel, hwnd, 48, 138, 240, 26, "Pet renderer");
    add_pet_note(
        panel,
        hwnd,
        48,
        164,
        620,
        20,
        "Tune the desktop pet size and map each mood to a GIF filename.",
    );
    add_pet_label(panel, hwnd, 48, 198, 150, 16, "Pet size");
    panel.pet_scale_slider = add_pet_slider(
        panel,
        hwnd,
        ID_PET_SCALE,
        60,
        224,
        260,
        36,
        panel.settings.pet_scale_percent(),
        PET_SCALE_MIN_PERCENT,
        PET_SCALE_MAX_PERCENT,
        10,
    );
    panel.pet_scale_value = add_pet_label(
        panel,
        hwnd,
        360,
        232,
        72,
        20,
        &format!("{}%", panel.settings.pet_scale_percent()),
    );
    add_pet_label(panel, hwnd, 448, 198, 150, 16, "Sleep after (s)");
    panel.sleep_after_slider = add_pet_slider(
        panel,
        hwnd,
        ID_SLEEP_AFTER,
        460,
        224,
        260,
        36,
        panel.settings.sleep_after_secs(),
        15,
        1800,
        60,
    );
    panel.sleep_after_value = add_pet_label(
        panel,
        hwnd,
        730,
        232,
        72,
        20,
        &format!("{}s", panel.settings.sleep_after_secs()),
    );
    add_pet_label(panel, hwnd, 48, 274, 150, 16, "Pet asset directory");
    panel.pet_dir = add_pet_edit(
        panel,
        hwnd,
        ID_PET_DIR,
        48,
        294,
        384,
        36,
        &panel.settings.pet_dir.clone(),
    );
    add_pet_label(panel, hwnd, 448, 274, 150, 16, "GIF directory");
    panel.gif_dir = add_pet_edit(
        panel,
        hwnd,
        ID_GIF_DIR,
        448,
        294,
        384,
        36,
        &panel.settings.gif_dir.clone(),
    );

    for (index, (mood, label_text)) in mood_rows().iter().enumerate() {
        let col = index % 2;
        let row = index / 2;
        let x = if col == 0 { 48 } else { 448 };
        let y = 332 + row as i32 * 54;
        let value = panel.settings.animation_value(*mood).to_string();
        add_pet_label(panel, hwnd, x, y, 118, 16, label_text);
        let edit = add_pet_edit(
            panel,
            hwnd,
            ID_ANIMATION_BASE + index,
            x,
            y + 20,
            384,
            32,
            &value,
        );
        panel.animation_edits.push((*mood, edit));
    }

    add_pet_button(panel, hwnd, ID_SAVE_PET, 624, 612, 96, 32, "Save", true);
    add_pet_button(panel, hwnd, ID_RESET_PET, 730, 612, 96, 32, "Reset", false);

    add_pomodoro_heading(panel, hwnd, 48, 138, 240, 26, "Pomodoro");
    add_pomodoro_note(
        panel,
        hwnd,
        48,
        164,
        620,
        20,
        "Set focus and break lengths, then control the active timer.",
    );
    add_pomodoro_label(panel, hwnd, 48, 214, 96, 16, "Focus min");
    panel.pomodoro_focus = add_pomodoro_edit(
        panel,
        hwnd,
        ID_POMODORO_FOCUS,
        48,
        234,
        120,
        36,
        &panel.settings.pomodoro.focus_minutes().to_string(),
    );
    add_pomodoro_label(panel, hwnd, 176, 214, 96, 16, "Short break");
    panel.pomodoro_short_break = add_pomodoro_edit(
        panel,
        hwnd,
        ID_POMODORO_SHORT_BREAK,
        176,
        234,
        120,
        36,
        &panel.settings.pomodoro.short_break_minutes().to_string(),
    );
    add_pomodoro_label(panel, hwnd, 304, 214, 96, 16, "Long break");
    panel.pomodoro_long_break = add_pomodoro_edit(
        panel,
        hwnd,
        ID_POMODORO_LONG_BREAK,
        304,
        234,
        120,
        36,
        &panel.settings.pomodoro.long_break_minutes().to_string(),
    );
    add_pomodoro_button(
        panel,
        hwnd,
        ID_SAVE_POMODORO,
        518,
        236,
        96,
        32,
        "Save",
        true,
    );
    panel.pomodoro_status = add_pomodoro_label(
        panel,
        hwnd,
        48 + SETTINGS_CONTAINER_PAD_X,
        336 + SETTINGS_CONTAINER_PAD_Y,
        620,
        44,
        "",
    );
    add_pomodoro_button(
        panel,
        hwnd,
        ID_START_POMODORO,
        48,
        430,
        96,
        32,
        "Start",
        true,
    );
    panel.pomodoro_pause_resume = add_pomodoro_button(
        panel,
        hwnd,
        ID_PAUSE_RESUME_POMODORO,
        154,
        430,
        112,
        32,
        "Pause",
        false,
    );
    add_pomodoro_button(
        panel,
        hwnd,
        ID_SKIP_POMODORO,
        276,
        430,
        96,
        32,
        "Skip",
        false,
    );
    add_pomodoro_button(
        panel,
        hwnd,
        ID_STOP_POMODORO,
        382,
        430,
        96,
        32,
        "Stop",
        false,
    );
    show_tab(panel, SettingsTab::Basic);
}

unsafe fn refresh_profile_combo(panel: &SettingsPanel) {
    SendMessageW(panel.profile_combo, CB_RESETCONTENT, 0, 0);
    for profile in &panel.llm_db.profiles {
        let mut label = profile.display_label();
        if label.trim().is_empty() {
            label = profile.id.clone();
        }
        let label = wide(&label);
        SendMessageW(
            panel.profile_combo,
            CB_ADDSTRING,
            0,
            label.as_ptr() as isize,
        );
    }
    let index = panel
        .llm_db
        .profiles
        .iter()
        .position(|profile| profile.id == panel.llm_db.active_profile_id)
        .unwrap_or(0);
    if !panel.llm_db.profiles.is_empty() {
        SendMessageW(panel.profile_combo, CB_SETCURSEL, index, 0);
    }
}

unsafe fn load_selected_profile(panel: &SettingsPanel) {
    let Some(index) = selected_profile_index(panel) else {
        clear_profile_fields(panel);
        return;
    };
    let Some(profile) = panel.llm_db.profiles.get(index) else {
        clear_profile_fields(panel);
        return;
    };
    set_profile_fields(panel, profile);
}

unsafe fn switch_tab(hwnd: HWND, tab: SettingsTab) {
    let Some(panel) = panel(hwnd) else {
        return;
    };
    show_tab(panel, tab);
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

unsafe fn show_tab(panel: &mut SettingsPanel, tab: SettingsTab) {
    panel.active_tab = tab;
    for control in &panel.llm_controls {
        ShowWindow(
            *control,
            if tab == SettingsTab::Llm {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
    }
    for control in &panel.pet_controls {
        ShowWindow(
            *control,
            if tab == SettingsTab::Basic {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
    }
    for control in &panel.pomodoro_controls {
        ShowWindow(
            *control,
            if tab == SettingsTab::Pomodoro {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
    }
    if tab == SettingsTab::Pomodoro {
        refresh_pomodoro_tab(panel);
    }
    set_text(panel.tab_llm, "LLM Profiles");
    set_text(panel.tab_basic, "Basic");
    set_text(panel.tab_pomodoro, "Pomodoro");
}

unsafe fn new_profile(hwnd: HWND) {
    let Some(panel) = panel(hwnd) else {
        return;
    };
    SendMessageW(panel.profile_combo, CB_SETCURSEL, usize::MAX, 0);
    clear_profile_fields(panel);
    set_text(panel.profile_name, "New Provider");
}

struct SavedProfile {
    profile: LlmProfile,
    was_active: bool,
}

unsafe fn save_profile_from_button(hwnd: HWND) {
    let Some(saved) = save_profile(hwnd, false) else {
        return;
    };
    if saved.was_active {
        apply_saved_profile(hwnd, &saved.profile, "applied saved LLM profile");
    }
}

unsafe fn save_profile(hwnd: HWND, activate_profile: bool) -> Option<SavedProfile> {
    let panel = panel(hwnd)?;
    let previous_id = selected_profile_index(panel)
        .and_then(|index| panel.llm_db.profiles.get(index))
        .map(|profile| profile.id.clone());
    let active_profile_id = panel.llm_db.active_profile_id.clone();
    let mut profile = current_profile_from_fields(panel);
    if profile.name.trim().is_empty() {
        message(
            hwnd,
            "Profile name required",
            "Please enter a profile name.",
        );
        return None;
    }
    if let Err(err) = profile.openai_extra_body_fields() {
        message(hwnd, "Invalid OpenAI body", &err);
        return None;
    }
    if profile.id.trim().is_empty() {
        profile.id = default_profile_id(&profile.name);
    }
    let was_active = previous_id
        .as_deref()
        .is_some_and(|id| id == active_profile_id)
        || profile.id == active_profile_id;

    if let Some(previous_id) = previous_id {
        if previous_id != profile.id {
            panel
                .llm_db
                .profiles
                .retain(|existing| existing.id != previous_id);
        }
    }
    panel.llm_db.upsert_profile(profile.clone());
    if activate_profile || was_active {
        panel.llm_db.active_profile_id = profile.id.clone();
    }

    if let Err(err) = save_llm_profile_db(&panel.llm_db) {
        message(hwnd, "Failed to save profile", &err);
        return None;
    }
    if let Err(err) = ensure_claude_onboarding_complete() {
        message(hwnd, "Failed to update Claude onboarding", &err);
    }
    sync_app_llm_profiles(&panel.llm_db, "saved LLM profile");
    refresh_profile_combo(panel);
    select_profile_by_id(panel, &profile.id);
    set_profile_fields(panel, &profile);
    Some(SavedProfile {
        profile,
        was_active,
    })
}

unsafe fn use_profile(hwnd: HWND) {
    let Some(saved) = save_profile(hwnd, true) else {
        return;
    };
    apply_saved_profile(hwnd, &saved.profile, "applied LLM profile");
}

unsafe fn apply_saved_profile(hwnd: HWND, profile: &LlmProfile, event: &str) {
    if let Err(err) = apply_llm_profile_to_claude(profile) {
        message(hwnd, "Failed to apply profile", &err);
        return;
    }
    if let Some(panel) = panel(hwnd) {
        sync_app_llm_profiles(&panel.llm_db, event);
    }
}

unsafe fn import_current_profile(hwnd: HWND) {
    let Some(panel) = panel(hwnd) else {
        return;
    };
    let Some(profile) = current_claude_llm_profile() else {
        message(
            hwnd,
            "Nothing to import",
            "Could not read an LLM profile from ~/.claude/settings.json.",
        );
        return;
    };
    set_profile_fields(panel, &profile);
}

unsafe fn delete_profile(hwnd: HWND) {
    let Some(panel) = panel(hwnd) else {
        return;
    };
    let Some(index) = selected_profile_index(panel) else {
        message(
            hwnd,
            "No profile selected",
            "Select a saved LLM profile before deleting.",
        );
        return;
    };
    let Some(profile) = panel.llm_db.profiles.get(index).cloned() else {
        message(
            hwnd,
            "Profile not found",
            "The selected LLM profile could not be found.",
        );
        return;
    };

    let label = profile_label_for_message(&profile);
    if !confirm(
        hwnd,
        "Delete profile",
        &format!("Delete \"{}\" from saved LLM profiles?", label),
    ) {
        return;
    }

    let mut next_db = panel.llm_db.clone();
    if next_db.remove_profile(&profile.id).is_none() {
        message(
            hwnd,
            "Profile not found",
            "The selected LLM profile could not be found.",
        );
        return;
    }
    let next_selection_id = next_db
        .profiles
        .get(index)
        .or_else(|| {
            index
                .checked_sub(1)
                .and_then(|index| next_db.profiles.get(index))
        })
        .map(|profile| profile.id.clone());

    if let Err(err) = save_llm_profile_db(&next_db) {
        message(hwnd, "Failed to delete profile", &err);
        return;
    }

    panel.llm_db = next_db;
    sync_app_llm_profiles(&panel.llm_db, "deleted LLM profile");
    refresh_profile_combo(panel);
    if let Some(next_selection_id) = next_selection_id {
        select_profile_by_id(panel, &next_selection_id);
        load_selected_profile(panel);
    } else {
        SendMessageW(panel.profile_combo, CB_SETCURSEL, usize::MAX, 0);
        clear_profile_fields(panel);
    }
}

unsafe fn save_basic_settings(hwnd: HWND) {
    let Some(panel) = panel(hwnd) else {
        return;
    };

    collect_basic_fields(panel);

    if let Err(err) = save_user_settings(&panel.settings) {
        message(hwnd, "Failed to save basic settings", &err);
        return;
    }

    sync_app_settings(&panel.settings, "saved basic settings");
    if let Err(err) = reload_animation_store() {
        message(hwnd, "Failed to reload pet renderer", &err);
    }
}

unsafe fn update_pet_scale_from_slider(hwnd: HWND, panel: &mut SettingsPanel, persist: bool) {
    collect_basic_fields(panel);
    update_pet_scale_label(panel);
    sync_app_settings(&panel.settings, "changed pet size");
    if persist {
        let _ = save_user_settings(&panel.settings).map_err(|err| {
            message(hwnd, "Failed to save pet size", &err);
        });
    }
}

unsafe fn update_sleep_after_from_slider(hwnd: HWND, panel: &mut SettingsPanel, persist: bool) {
    collect_basic_fields(panel);
    update_sleep_after_label(panel);
    sync_app_settings(&panel.settings, "changed sleep timeout");
    if persist {
        let _ = save_user_settings(&panel.settings).map_err(|err| {
            message(hwnd, "Failed to save sleep timeout", &err);
        });
    }
}

unsafe fn collect_basic_fields(panel: &mut SettingsPanel) {
    panel.settings.pet_scale_percent = slider_value_clamped(
        panel.pet_scale_slider,
        PET_SCALE_MIN_PERCENT,
        PET_SCALE_MAX_PERCENT,
    );
    panel.settings.sleep_after_secs = slider_value_clamped(panel.sleep_after_slider, 15, 1800);
    panel.settings.pet_dir = text_value(panel.pet_dir);
    panel.settings.gif_dir = text_value(panel.gif_dir);
    panel.settings.animations = AnimationSettings::default();
    for (mood, edit) in &panel.animation_edits {
        panel.settings.set_animation_value(*mood, text_value(*edit));
    }
}

unsafe fn slider_value_clamped(hwnd: HWND, min: u32, max: u32) -> u32 {
    let value = SendMessageW(hwnd, TBM_GETPOS, 0, 0);
    (value as u32).clamp(min, max)
}

unsafe fn minutes_value(hwnd: HWND, fallback: u32) -> u32 {
    text_value(hwnd)
        .parse::<u32>()
        .unwrap_or(fallback)
        .clamp(POMODORO_MIN_MINUTES, POMODORO_MAX_MINUTES)
}

unsafe fn update_pet_scale_label(panel: &SettingsPanel) {
    set_text(
        panel.pet_scale_value,
        &format!("{}%", panel.settings.pet_scale_percent()),
    );
}

unsafe fn update_sleep_after_label(panel: &SettingsPanel) {
    set_text(
        panel.sleep_after_value,
        &format!("{}s", panel.settings.sleep_after_secs()),
    );
}

unsafe fn update_pomodoro_fields(panel: &SettingsPanel) {
    set_text(
        panel.pomodoro_focus,
        &panel.settings.pomodoro.focus_minutes().to_string(),
    );
    set_text(
        panel.pomodoro_short_break,
        &panel.settings.pomodoro.short_break_minutes().to_string(),
    );
    set_text(
        panel.pomodoro_long_break,
        &panel.settings.pomodoro.long_break_minutes().to_string(),
    );
}

unsafe fn collect_pomodoro_fields(panel: &mut SettingsPanel) {
    panel.settings.pomodoro.focus_minutes = minutes_value(
        panel.pomodoro_focus,
        panel.settings.pomodoro.focus_minutes(),
    );
    panel.settings.pomodoro.short_break_minutes = minutes_value(
        panel.pomodoro_short_break,
        panel.settings.pomodoro.short_break_minutes(),
    );
    panel.settings.pomodoro.long_break_minutes = minutes_value(
        panel.pomodoro_long_break,
        panel.settings.pomodoro.long_break_minutes(),
    );
}

unsafe fn save_pomodoro_settings(hwnd: HWND) {
    let Some(panel) = panel(hwnd) else {
        return;
    };
    collect_pomodoro_fields(panel);
    if let Err(err) = save_user_settings(&panel.settings) {
        message(hwnd, "Failed to save pomodoro settings", &err);
        return;
    }
    update_pomodoro_fields(panel);
    sync_app_settings(&panel.settings, "saved pomodoro settings");
    refresh_pomodoro_tab(panel);
}

unsafe fn reset_pet_fields(hwnd: HWND) {
    let Some(panel) = panel(hwnd) else {
        return;
    };
    let defaults = UserSettings::default();
    panel.settings.pet_scale_percent = defaults.pet_scale_percent();
    panel.settings.sleep_after_secs = defaults.sleep_after_secs();
    SendMessageW(
        panel.pet_scale_slider,
        TBM_SETPOS,
        1,
        panel.settings.pet_scale_percent as isize,
    );
    SendMessageW(
        panel.sleep_after_slider,
        TBM_SETPOS,
        1,
        panel.settings.sleep_after_secs as isize,
    );
    update_pet_scale_label(panel);
    update_sleep_after_label(panel);
    sync_app_settings(&panel.settings, "reset pet settings");
    set_text(panel.pet_dir, &defaults.pet_dir);
    set_text(panel.gif_dir, &defaults.gif_dir);
    for (mood, edit) in &panel.animation_edits {
        set_text(*edit, defaults.animation_value(*mood));
    }
}

unsafe fn refresh_settings_dynamic(hwnd: HWND) {
    if let Some(panel) = panel(hwnd) {
        refresh_pomodoro_tab(panel);
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

unsafe fn refresh_pomodoro_tab(panel: &SettingsPanel) {
    let Some(state) = APP_STATE.get() else {
        set_text(panel.pomodoro_status, "Pomodoro data is not ready.");
        return;
    };
    let state = state.lock().expect("state poisoned");
    let mode = match state.pomodoro.mode {
        PomodoroMode::Focus => "Focus",
        PomodoroMode::ShortBreak => "Short break",
        PomodoroMode::LongBreak => "Long break",
    };
    let status = match state.pomodoro.status {
        PomodoroStatus::Stopped => "Stopped",
        PomodoroStatus::Running => "Running",
        PomodoroStatus::Paused => "Paused",
    };
    set_text(
        panel.pomodoro_status,
        &format!(
            "{}    {}    {}\nCompleted focus sessions: {}",
            mode,
            status,
            format_remaining(state.pomodoro.remaining(&state.settings.pomodoro)),
            state.pomodoro.completed_focus_count
        ),
    );
    set_text(
        panel.pomodoro_pause_resume,
        if state.pomodoro.status == PomodoroStatus::Paused {
            "Resume"
        } else {
            "Pause"
        },
    );
}

fn mutate_app_state(action: impl FnOnce(&mut AppState)) {
    if let Some(state) = APP_STATE.get() {
        action(&mut state.lock().expect("state poisoned"));
    }
}

fn sync_app_settings(settings: &UserSettings, event: &str) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        state.settings = settings.clone();
        state.push_event("settings", event);
        state.show_speech(
            "Settings saved",
            event,
            std::time::Duration::from_secs(3),
            4,
        );
    }
}

fn sync_app_llm_profiles(llm_db: &LlmProfileDb, event: &str) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        state.llm_profiles = llm_db.clone();
        state.push_event("settings", event);
        state.show_speech("LLM profile", event, std::time::Duration::from_secs(3), 4);
    }
}

unsafe fn selected_profile_index(panel: &SettingsPanel) -> Option<usize> {
    let index = SendMessageW(panel.profile_combo, CB_GETCURSEL, 0, 0);
    if index < 0 {
        None
    } else {
        Some(index as usize)
    }
}

unsafe fn select_profile_by_id(panel: &SettingsPanel, id: &str) {
    if let Some(index) = panel
        .llm_db
        .profiles
        .iter()
        .position(|profile| profile.id == id)
    {
        SendMessageW(panel.profile_combo, CB_SETCURSEL, index, 0);
    }
}

unsafe fn current_profile_from_fields(panel: &SettingsPanel) -> LlmProfile {
    LlmProfile {
        id: text_value(panel.profile_id),
        name: text_value(panel.profile_name),
        base_url: text_value(panel.base_url),
        auth_token: text_value(panel.auth_token),
        api_key: text_value(panel.api_key),
        model: text_value(panel.model),
        opus_model: text_value(panel.opus_model),
        sonnet_model: text_value(panel.sonnet_model),
        haiku_model: text_value(panel.haiku_model),
        openai_extra_body: text_value(panel.openai_extra_body),
        extra_env: text_value(panel.extra_env),
    }
}

fn profile_label_for_message(profile: &LlmProfile) -> String {
    let label = profile.display_label();
    if label.trim().is_empty() {
        profile.id.clone()
    } else {
        label
    }
}

unsafe fn set_profile_fields(panel: &SettingsPanel, profile: &LlmProfile) {
    set_text(panel.profile_id, &profile.id);
    set_text(panel.profile_name, &profile.name);
    set_text(panel.base_url, &profile.base_url);
    set_text(panel.auth_token, &profile.auth_token);
    set_text(panel.api_key, &profile.api_key);
    set_text(panel.model, &profile.model);
    set_text(panel.opus_model, &profile.opus_model);
    set_text(panel.sonnet_model, &profile.sonnet_model);
    set_text(panel.haiku_model, &profile.haiku_model);
    set_text(panel.openai_extra_body, &profile.openai_extra_body);
    set_text(panel.extra_env, &profile.extra_env);
}

unsafe fn clear_profile_fields(panel: &SettingsPanel) {
    let empty = LlmProfile::default();
    set_profile_fields(panel, &empty);
}

unsafe fn panel(hwnd: HWND) -> Option<&'static mut SettingsPanel> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsPanel;
    (!ptr.is_null()).then(|| &mut *ptr)
}

fn loword(value: usize) -> u16 {
    (value & 0xffff) as u16
}

fn hiword(value: usize) -> u16 {
    ((value >> 16) & 0xffff) as u16
}
