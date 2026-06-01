mod render;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::time::Duration;

use render::{
    RenderState, choice_option_at_point, fill_rect, render_permission_overlay, render_scene,
    scroll_choice_lines, scroll_permission_detail_lines, snapshot_state,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EndPaint, InvalidateRect, PAINTSTRUCT, SRCCOPY, ScreenToClient, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, LASTINPUTINFO, MOD_CONTROL, MOD_SHIFT, RegisterHotKey, ReleaseCapture,
    SetCapture, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, IDC_ARROW,
    IDC_HAND, LWA_COLORKEY, LoadCursorW, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    RegisterClassW, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SetCursor, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN,
    TrackPopupMenu, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    WS_VISIBLE,
};

use crate::app::pomodoro::PomodoroStatus;
use crate::app::{AppState, PermissionDecision, PetMood};
use crate::config::*;
use crate::globals::{APP_STATE, PET_RENDERER};
use crate::hooks::{
    decide_current_permission, deny_current_choice, submit_current_choice,
    toggle_current_choice_option,
};
use crate::settings::{
    LlmProfile, UserSettings, WindowPosition, apply_llm_profile_to_claude,
    ensure_claude_onboarding_complete, load_user_settings, save_llm_profile_db, save_user_settings,
};
use crate::ui::prompt_popup::{close_prompt_popup, sync_prompt_popup};
use crate::ui::settings_panel::{close_settings_panel, show_settings_panel};
use crate::util::wide;

static CONTEXT_MENU_OPEN: AtomicBool = AtomicBool::new(false);
static LEFT_BUTTON_CAPTURED: AtomicBool = AtomicBool::new(false);
static LEFT_BUTTON_DRAGGING: AtomicBool = AtomicBool::new(false);
static LEFT_BUTTON_SCREEN_X: AtomicI32 = AtomicI32::new(0);
static LEFT_BUTTON_SCREEN_Y: AtomicI32 = AtomicI32::new(0);
static DRAG_WINDOW_X: AtomicI32 = AtomicI32::new(0);
static DRAG_WINDOW_Y: AtomicI32 = AtomicI32::new(0);
static RIGHT_BUTTON_CAPTURED: AtomicBool = AtomicBool::new(false);
static CURRENT_TIMER_MS: AtomicU32 = AtomicU32::new(ACTIVE_TIMER_MS);
static LAST_TOPMOST_TICK: AtomicU32 = AtomicU32::new(0);
const CLICK_DRAG_THRESHOLD_PX: i32 = 6;
const ACTIVE_TIMER_MS: u32 = 33;
const IDLE_TIMER_MS: u32 = 120;
const SLEEP_TIMER_MS: u32 = 500;
const TOPMOST_REFRESH_MS: u32 = 1_000;

pub(crate) unsafe fn run_window(port: u16) {
    let class_name = wide("ClaudieWindow");
    let overlay_class_name = wide("ClaudiePermissionOverlay");
    let title = wide(&format!("claudie :{port}"));
    let hinstance = GetModuleHandleW(std::ptr::null());
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: std::ptr::null_mut(),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    RegisterClassW(&wc);
    let overlay_wc = WNDCLASSW {
        lpfnWndProc: Some(permission_overlay_proc),
        lpszClassName: overlay_class_name.as_ptr(),
        ..wc
    };
    RegisterClassW(&overlay_wc);

    let settings = current_user_settings();
    let (x, y) = initial_pet_position(&settings);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_POPUP | WS_VISIBLE,
        x,
        y,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null_mut(),
    );

    if hwnd == std::ptr::null_mut() {
        eprintln!("Failed to create claudie window");
        return;
    }

    let overlay_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
        overlay_class_name.as_ptr(),
        title.as_ptr(),
        WS_POPUP,
        0,
        0,
        PERMISSION_OVERLAY_WIDTH,
        PERMISSION_OVERLAY_HEIGHT,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null_mut(),
    );
    if !overlay_hwnd.is_null() {
        SetLayeredWindowAttributes(overlay_hwnd, TRANSPARENT_KEY, 255, LWA_COLORKEY);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, overlay_hwnd as isize);
    }

    SetLayeredWindowAttributes(hwnd, TRANSPARENT_KEY, 255, LWA_COLORKEY);
    ShowWindow(hwnd, SW_SHOW);
    ensure_pet_topmost(hwnd);
    RegisterHotKey(hwnd, 1, MOD_CONTROL | MOD_SHIFT, 'Y' as u32);
    RegisterHotKey(hwnd, 2, MOD_CONTROL | MOD_SHIFT, 'N' as u32);
    SetTimer(hwnd, 1, ACTIVE_TIMER_MS, None);

    if let Err(err) = slint::run_event_loop_until_quit() {
        eprintln!("Slint event loop failed: {err}");
    }

    UnregisterHotKey(hwnd, 1);
    UnregisterHotKey(hwnd, 2);
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let _createstruct = lparam as *const CREATESTRUCTW;
            0
        }
        WM_PAINT => {
            paint_window(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_TIMER => {
            let mut timer_ms = IDLE_TIMER_MS;
            if let Some(state) = APP_STATE.get() {
                let mut state = state.lock().expect("state poisoned");
                let user_idle = user_idle_snapshot();
                state.tick_pomodoro();
                state.tick_fishing();
                state.decay_mood(
                    user_idle.map(|snapshot| snapshot.0),
                    user_idle.map(|snapshot| snapshot.1),
                );
                timer_ms = desired_timer_interval(&state);
            }
            update_window_timer(hwnd, timer_ms);
            let overlay = overlay_hwnd(hwnd);
            if !overlay.is_null() {
                ShowWindow(overlay, SW_HIDE);
            }
            sync_prompt_popup();
            if !CONTEXT_MENU_OPEN.load(Ordering::Relaxed) && should_refresh_topmost() {
                ensure_pet_topmost(hwnd);
            }
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_HOTKEY => {
            match wparam {
                1 => decide_current_permission(PermissionDecision::AllowOnce),
                2 => decide_current_permission(PermissionDecision::Deny),
                _ => {}
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = base_point_from_client(
                hwnd,
                loword(lparam as u32) as i32,
                hiword(lparam as u32) as i32,
            );
            if x < 0 || y < 0 {
                return 0;
            }
            let mut cursor = POINT { x: 0, y: 0 };
            let mut rect = RECT::default();
            if GetCursorPos(&mut cursor) == 0 || GetWindowRect(hwnd, &mut rect) == 0 {
                return 0;
            }
            LEFT_BUTTON_CAPTURED.store(true, Ordering::Relaxed);
            LEFT_BUTTON_DRAGGING.store(false, Ordering::Relaxed);
            LEFT_BUTTON_SCREEN_X.store(cursor.x, Ordering::Relaxed);
            LEFT_BUTTON_SCREEN_Y.store(cursor.y, Ordering::Relaxed);
            DRAG_WINDOW_X.store(rect.left, Ordering::Relaxed);
            DRAG_WINDOW_Y.store(rect.top, Ordering::Relaxed);
            SetCapture(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            if !LEFT_BUTTON_CAPTURED.load(Ordering::Relaxed) {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let mut cursor = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut cursor) == 0 {
                return 0;
            }
            let dx = cursor.x - LEFT_BUTTON_SCREEN_X.load(Ordering::Relaxed);
            let dy = cursor.y - LEFT_BUTTON_SCREEN_Y.load(Ordering::Relaxed);
            if !LEFT_BUTTON_DRAGGING.load(Ordering::Relaxed)
                && dx.abs().max(dy.abs()) >= CLICK_DRAG_THRESHOLD_PX
            {
                LEFT_BUTTON_DRAGGING.store(true, Ordering::Relaxed);
            }
            if LEFT_BUTTON_DRAGGING.load(Ordering::Relaxed) {
                let (x, y) = clamp_window_position(
                    DRAG_WINDOW_X.load(Ordering::Relaxed) + dx,
                    DRAG_WINDOW_Y.load(Ordering::Relaxed) + dy,
                    current_pet_rect_in_window(),
                );
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            0
        }
        WM_LBUTTONUP => {
            let was_captured = LEFT_BUTTON_CAPTURED.swap(false, Ordering::Relaxed);
            let was_dragging = LEFT_BUTTON_DRAGGING.swap(false, Ordering::Relaxed);
            ReleaseCapture();
            if was_captured && !was_dragging {
                with_app_state(|state| state.interact_with_pet());
                InvalidateRect(hwnd, std::ptr::null(), 0);
            } else if was_dragging {
                persist_pet_window_position(hwnd);
            }
            0
        }
        WM_RBUTTONDOWN => {
            RIGHT_BUTTON_CAPTURED.store(true, Ordering::Relaxed);
            SetCapture(hwnd);
            0
        }
        WM_RBUTTONUP => {
            let was_captured = RIGHT_BUTTON_CAPTURED.swap(false, Ordering::Relaxed);
            if was_captured {
                ReleaseCapture();
            }
            show_context_menu(hwnd);
            0
        }
        WM_SETCURSOR => {
            if cursor_over_pet(hwnd) {
                SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            persist_pet_window_position(hwnd);
            flush_app_stats_now();
            let overlay = overlay_hwnd(hwnd);
            if !overlay.is_null() {
                DestroyWindow(overlay);
            }
            close_settings_panel();
            close_prompt_popup();
            let _ = slint::quit_event_loop();
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn current_user_settings() -> UserSettings {
    APP_STATE
        .get()
        .map(|state| state.lock().expect("state poisoned").settings.clone())
        .unwrap_or_else(load_user_settings)
}

unsafe fn initial_pet_position(settings: &UserSettings) -> (i32, i32) {
    let Some(position) = settings.window_position else {
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    };
    clamp_window_position(
        position.x,
        position.y,
        visible_pet_rect_in_window(settings, PetMood::Idle),
    )
}

unsafe fn clamp_window_position(x: i32, y: i32, visible_rect: (i32, i32, i32, i32)) -> (i32, i32) {
    let mut min_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let mut min_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let mut screen_w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let mut screen_h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    if screen_w <= 0 || screen_h <= 0 {
        min_x = 0;
        min_y = 0;
        screen_w = GetSystemMetrics(SM_CXSCREEN);
        screen_h = GetSystemMetrics(SM_CYSCREEN);
    }
    let (visible_x, visible_y, visible_w, visible_h) = visible_rect;
    let screen_w = screen_w.max(visible_w);
    let screen_h = screen_h.max(visible_h);
    let min_window_x = min_x - visible_x;
    let min_window_y = min_y - visible_y;
    let max_window_x = (min_x + screen_w - visible_x - visible_w).max(min_window_x);
    let max_window_y = (min_y + screen_h - visible_y - visible_h).max(min_window_y);
    (
        x.clamp(min_window_x, max_window_x),
        y.clamp(min_window_y, max_window_y),
    )
}

fn current_pet_rect_in_window() -> (i32, i32, i32, i32) {
    let (settings, mood) = current_pet_settings_and_mood();
    visible_pet_rect_in_window(&settings, mood)
}

fn current_pet_settings_and_mood() -> (UserSettings, PetMood) {
    APP_STATE
        .get()
        .map(|state| {
            let state = state.lock().expect("state poisoned");
            (state.settings.clone(), state.mood)
        })
        .unwrap_or_else(|| (load_user_settings(), PetMood::Idle))
}

fn nominal_pet_rect_in_window(settings: &UserSettings) -> (i32, i32, i32, i32) {
    let scale = settings.pet_scale_percent() as i32;
    let w = (PET_W * scale + 50) / 100;
    let h = (PET_H * scale + 50) / 100;
    let center_x = PET_X + PET_W / 2;
    let bottom_y = PET_Y + PET_H;
    (center_x - w / 2, bottom_y - h, w.max(1), h.max(1))
}

fn visible_pet_rect_in_window(settings: &UserSettings, mood: PetMood) -> (i32, i32, i32, i32) {
    let nominal_rect = nominal_pet_rect_in_window(settings);
    let Some(bounds) = PET_RENDERER.get().and_then(|store| {
        store
            .lock()
            .expect("pet renderer poisoned")
            .visible_bounds(mood)
    }) else {
        return nominal_rect;
    };

    let (pet_x, pet_y, pet_w, pet_h) = nominal_rect;
    let (draw_w, draw_h) = fit_source_into(bounds.source_width, bounds.source_height, pet_w, pet_h);
    let draw_x = pet_x + (pet_w - draw_w) / 2;
    let draw_y = pet_y + (pet_h - draw_h);
    let source_w = bounds.source_width.max(1) as i64;
    let source_h = bounds.source_height.max(1) as i64;
    let draw_w = draw_w as i64;
    let draw_h = draw_h as i64;

    let left = draw_x + ((bounds.x as i64 * draw_w) / source_w) as i32;
    let top = draw_y + ((bounds.y as i64 * draw_h) / source_h) as i32;
    let right =
        draw_x + (((bounds.x + bounds.width) as i64 * draw_w + source_w - 1) / source_w) as i32;
    let bottom =
        draw_y + (((bounds.y + bounds.height) as i64 * draw_h + source_h - 1) / source_h) as i32;
    (left, top, (right - left).max(1), (bottom - top).max(1))
}

fn fit_source_into(src_w: u32, src_h: u32, max_w: i32, max_h: i32) -> (i32, i32) {
    if src_w == 0 || src_h == 0 || max_w <= 0 || max_h <= 0 {
        return (max_w.max(1), max_h.max(1));
    }
    let scale_w = max_w as f32 / src_w as f32;
    let scale_h = max_h as f32 / src_h as f32;
    let scale = scale_w.min(scale_h).max(0.01);
    let w = ((src_w as f32) * scale).round() as i32;
    let h = ((src_h as f32) * scale).round() as i32;
    (w.max(1), h.max(1))
}

unsafe fn ensure_pet_topmost(hwnd: HWND) {
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
}

unsafe fn persist_pet_window_position(hwnd: HWND) {
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return;
    }

    let mut settings = current_user_settings();
    settings.window_position = Some(WindowPosition {
        x: rect.left,
        y: rect.top,
    });
    if save_user_settings(&settings).is_ok()
        && let Some(state) = APP_STATE.get()
    {
        state.lock().expect("state poisoned").settings = settings;
    }
}

fn flush_app_stats_now() {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        if let Err(err) = state.flush_stats_now() {
            state.last_error = format!("Stats save failed: {err}");
        }
    }
}

fn has_pending_overlay() -> bool {
    APP_STATE.get().is_some_and(|state| {
        let state = state.lock().expect("state poisoned");
        !state.pending_permissions.is_empty() || !state.pending_choices.is_empty()
    })
}

fn desired_timer_interval(state: &AppState) -> u32 {
    let pending = !state.pending_permissions.is_empty() || !state.pending_choices.is_empty();
    if pending
        || state.mood.is_active_work()
        || state.fishing.is_active()
        || matches!(
            state.mood,
            PetMood::Happy
                | PetMood::Error
                | PetMood::Pomodoro
                | PetMood::Wave
                | PetMood::Stretch
                | PetMood::Fishing
                | PetMood::FishingReel
                | PetMood::FishingCaught
                | PetMood::FishingMissed
        )
    {
        ACTIVE_TIMER_MS
    } else if state.mood == PetMood::Sleeping {
        SLEEP_TIMER_MS
    } else {
        IDLE_TIMER_MS
    }
}

unsafe fn update_window_timer(hwnd: HWND, interval_ms: u32) {
    let previous = CURRENT_TIMER_MS.swap(interval_ms, Ordering::Relaxed);
    if previous != interval_ms {
        SetTimer(hwnd, 1, interval_ms, None);
    }
}

fn should_refresh_topmost() -> bool {
    let now = unsafe { GetTickCount() };
    let mut last = LAST_TOPMOST_TICK.load(Ordering::Relaxed);
    loop {
        if last != 0 && now.wrapping_sub(last) < TOPMOST_REFRESH_MS {
            return false;
        }
        match LAST_TOPMOST_TICK.compare_exchange_weak(
            last,
            now,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(next) => last = next,
        }
    }
}

unsafe fn base_point_from_client(_hwnd: HWND, x: i32, y: i32) -> (i32, i32) {
    if x < 0 || y < 0 || x > WINDOW_WIDTH || y > WINDOW_HEIGHT {
        return (-1, -1);
    }
    (x, y)
}

unsafe fn cursor_over_pet(hwnd: HWND) -> bool {
    let mut point = POINT { x: 0, y: 0 };
    if GetCursorPos(&mut point) == 0 || ScreenToClient(hwnd, &mut point) == 0 {
        return false;
    }
    let (settings, mood) = current_pet_settings_and_mood();
    let (pet_x, pet_y, pet_w, pet_h) = visible_pet_rect_in_window(&settings, mood);
    point.x >= pet_x && point.x <= pet_x + pet_w && point.y >= pet_y && point.y <= pet_y + pet_h
}

unsafe extern "system" fn permission_overlay_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_permission_overlay(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_LBUTTONDOWN => {
            let x = loword(lparam as u32) as i32;
            let y = hiword(lparam as u32) as i32;
            if let Some((question_index, option_index)) = choice_option_at(x, y) {
                toggle_current_choice_option(question_index, option_index);
                InvalidateRect(hwnd, std::ptr::null(), 0);
                return 0;
            }
            if has_pending_choice() {
                if point_in_tuple(x, y, CHOICE_SUBMIT_BUTTON) {
                    submit_current_choice();
                    return 0;
                }
                if point_in_tuple(x, y, CHOICE_DENY_BUTTON) {
                    deny_current_choice();
                    return 0;
                }
                return 0;
            }
            if point_in_tuple(x, y, ALLOW_BUTTON) {
                decide_current_permission(PermissionDecision::AllowOnce);
                return 0;
            }
            if point_in_tuple(x, y, ALWAYS_BUTTON) {
                decide_current_permission(PermissionDecision::AllowAlways);
                return 0;
            }
            if point_in_tuple(x, y, DENY_BUTTON) {
                decide_current_permission(PermissionDecision::Deny);
                return 0;
            }
            0
        }
        WM_MOUSEWHEEL => {
            let wheel_delta = hiword(wparam as u32) as i32;
            let line_delta = if wheel_delta > 0 { -3 } else { 3 };
            if choice_content_can_scroll_at(hwnd, lparam) {
                scroll_choice_lines(line_delta);
                InvalidateRect(hwnd, std::ptr::null(), 0);
                return 0;
            }
            if permission_detail_can_scroll_at(hwnd, lparam) {
                scroll_permission_detail_lines(line_delta);
                InvalidateRect(hwnd, std::ptr::null(), 0);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SETCURSOR => {
            if cursor_over_permission_button(hwnd) {
                SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn permission_detail_can_scroll_at(hwnd: HWND, lparam: LPARAM) -> bool {
    if !has_pending_permission() || has_pending_choice() {
        return false;
    }

    let mut point = POINT {
        x: loword(lparam as u32) as i32,
        y: hiword(lparam as u32) as i32,
    };
    if ScreenToClient(hwnd, &mut point) == 0 {
        return false;
    }

    point_in_tuple(point.x, point.y, permission_detail_panel_rect())
}

unsafe fn choice_content_can_scroll_at(hwnd: HWND, lparam: LPARAM) -> bool {
    if !has_pending_choice() {
        return false;
    }

    let mut point = POINT {
        x: loword(lparam as u32) as i32,
        y: hiword(lparam as u32) as i32,
    };
    if ScreenToClient(hwnd, &mut point) == 0 {
        return false;
    }

    point_in_tuple(point.x, point.y, choice_content_rect())
}

fn permission_detail_panel_rect() -> (i32, i32, i32, i32) {
    let x = PERMISSION_BUBBLE_X + 24;
    let y = PERMISSION_BUBBLE_Y + 76;
    let w = PERMISSION_BUBBLE_W - 48;
    (x, y, w, PERMISSION_DETAIL_PANEL_H)
}

fn choice_content_rect() -> (i32, i32, i32, i32) {
    let y = CHOICE_CARD_Y + 50;
    (
        CHOICE_CARD_X + 24,
        y,
        CHOICE_CARD_W - 48,
        CHOICE_SUBMIT_BUTTON.1 - 12 - y,
    )
}

unsafe fn cursor_over_permission_button(hwnd: HWND) -> bool {
    if !has_pending_overlay() {
        return false;
    }

    let mut point = POINT { x: 0, y: 0 };
    if GetCursorPos(&mut point) == 0 || ScreenToClient(hwnd, &mut point) == 0 {
        return false;
    }
    choice_option_at(point.x, point.y).is_some()
        || point_in_tuple(point.x, point.y, CHOICE_SUBMIT_BUTTON)
        || point_in_tuple(point.x, point.y, CHOICE_DENY_BUTTON)
        || point_in_tuple(point.x, point.y, ALLOW_BUTTON)
        || point_in_tuple(point.x, point.y, ALWAYS_BUTTON)
        || point_in_tuple(point.x, point.y, DENY_BUTTON)
}

unsafe fn overlay_hwnd(hwnd: HWND) -> HWND {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HWND
}

unsafe fn show_context_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }

    let settings_label = wide("Settings...");
    AppendMenuW(menu, MF_STRING, MENU_SETTINGS_ID, settings_label.as_ptr());
    append_llm_profile_menu(menu);
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    let pomodoro_label = wide("Start Pomodoro");
    AppendMenuW(
        menu,
        MF_STRING,
        MENU_POMODORO_START_ID,
        pomodoro_label.as_ptr(),
    );
    let stop_pomodoro_label = wide("Stop Pomodoro");
    AppendMenuW(
        menu,
        MF_STRING,
        MENU_POMODORO_STOP_ID,
        stop_pomodoro_label.as_ptr(),
    );
    let pause_resume_label = wide("Pause/Resume Pomodoro");
    AppendMenuW(
        menu,
        MF_STRING,
        MENU_POMODORO_PAUSE_RESUME_ID,
        pause_resume_label.as_ptr(),
    );
    let skip_label = wide("Skip Pomodoro");
    AppendMenuW(menu, MF_STRING, MENU_POMODORO_SKIP_ID, skip_label.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    let fishing_label = wide("Start Fishing");
    AppendMenuW(
        menu,
        MF_STRING,
        MENU_FISHING_START_ID,
        fishing_label.as_ptr(),
    );
    let stop_fishing_label = wide("Stop Fishing");
    AppendMenuW(
        menu,
        MF_STRING,
        MENU_FISHING_STOP_ID,
        stop_fishing_label.as_ptr(),
    );
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    let exit_label = wide("Exit");
    AppendMenuW(menu, MF_STRING, MENU_EXIT_ID, exit_label.as_ptr());

    let mut point = POINT { x: 0, y: 0 };
    if GetCursorPos(&mut point) != 0 {
        SetForegroundWindow(hwnd);
        CONTEXT_MENU_OPEN.store(true, Ordering::Relaxed);
        let command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_TOPALIGN | TPM_RETURNCMD,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        CONTEXT_MENU_OPEN.store(false, Ordering::Relaxed);
        ensure_pet_topmost(hwnd);
        let command = command as usize;
        if command == MENU_SETTINGS_ID {
            show_settings_panel(hwnd);
        } else if let Some(index) = llm_profile_index_from_command(command) {
            activate_llm_profile(index);
        } else if command == MENU_POMODORO_START_ID {
            with_app_state(|state| state.start_pomodoro());
        } else if command == MENU_POMODORO_STOP_ID {
            with_app_state(|state| state.stop_pomodoro());
        } else if command == MENU_POMODORO_PAUSE_RESUME_ID {
            with_app_state(|state| {
                if state.pomodoro.status == PomodoroStatus::Paused {
                    state.resume_pomodoro();
                } else {
                    state.pause_pomodoro();
                }
            });
        } else if command == MENU_POMODORO_SKIP_ID {
            with_app_state(|state| state.skip_pomodoro());
        } else if command == MENU_FISHING_START_ID {
            with_app_state(|state| state.start_fishing());
        } else if command == MENU_FISHING_STOP_ID {
            with_app_state(|state| state.stop_fishing());
        } else if command == MENU_EXIT_ID {
            DestroyWindow(hwnd);
        }
    }

    DestroyMenu(menu);
}

unsafe fn append_llm_profile_menu(menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU) {
    let entries = llm_profile_menu_entries();
    if entries.is_empty() {
        return;
    }
    let profile_menu = CreatePopupMenu();
    if profile_menu.is_null() {
        return;
    }

    for (index, label, checked) in entries {
        let label = wide(&label);
        let flags = if checked {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        AppendMenuW(
            profile_menu,
            flags,
            MENU_LLM_PROFILE_BASE_ID + index,
            label.as_ptr(),
        );
    }

    let profile_label = wide("LLM Profile");
    AppendMenuW(
        menu,
        MF_POPUP,
        profile_menu as usize,
        profile_label.as_ptr(),
    );
}

fn llm_profile_menu_entries() -> Vec<(usize, String, bool)> {
    let Some(state) = APP_STATE.get() else {
        return Vec::new();
    };
    let mut db = state.lock().expect("state poisoned").llm_profiles.clone();
    db.normalize();
    db.profiles
        .iter()
        .take(MENU_LLM_PROFILE_MAX_ITEMS)
        .enumerate()
        .map(|(index, profile)| {
            let label = profile_menu_label(profile);
            (index, label, profile.id == db.active_profile_id)
        })
        .collect()
}

fn profile_menu_label(profile: &LlmProfile) -> String {
    let label = profile.display_label();
    if label.trim().is_empty() {
        profile.id.clone()
    } else {
        label
    }
}

fn llm_profile_index_from_command(command: usize) -> Option<usize> {
    let end = MENU_LLM_PROFILE_BASE_ID + MENU_LLM_PROFILE_MAX_ITEMS;
    (MENU_LLM_PROFILE_BASE_ID..end)
        .contains(&command)
        .then_some(command - MENU_LLM_PROFILE_BASE_ID)
}

fn activate_llm_profile(index: usize) {
    let Some(state_handle) = APP_STATE.get() else {
        return;
    };
    let (db, profile) = {
        let state = state_handle.lock().expect("state poisoned");
        let mut db = state.llm_profiles.clone();
        db.normalize();
        let Some(profile) = db.profiles.get(index).cloned() else {
            return;
        };
        db.active_profile_id = profile.id.clone();
        (db, profile)
    };

    if let Err(err) = save_llm_profile_db(&db) {
        record_profile_activation_error(format!("Failed to save LLM profile: {err}"));
        return;
    }
    if let Err(err) = ensure_claude_onboarding_complete() {
        record_profile_activation_error(format!("Claude onboarding failed: {err}"));
        return;
    }
    if let Err(err) = apply_llm_profile_to_claude(&profile) {
        record_profile_activation_error(format!("Failed to apply LLM profile: {err}"));
        return;
    }

    let mut state = state_handle.lock().expect("state poisoned");
    state.llm_profiles = db;
    state.set_mood(PetMood::Happy);
}

fn record_profile_activation_error(message: String) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        state.last_error = message;
        state.set_mood(PetMood::Error);
    }
}

fn with_app_state(action: impl FnOnce(&mut AppState)) {
    if let Some(state) = APP_STATE.get() {
        action(&mut state.lock().expect("state poisoned"));
    }
}

fn has_pending_choice() -> bool {
    APP_STATE.get().is_some_and(|state| {
        state
            .lock()
            .expect("state poisoned")
            .current_pending_choice()
            .is_some()
    })
}

fn has_pending_permission() -> bool {
    APP_STATE.get().is_some_and(|state| {
        state
            .lock()
            .expect("state poisoned")
            .current_pending_permission()
            .is_some()
    })
}

fn choice_option_at(px: i32, py: i32) -> Option<(usize, usize)> {
    APP_STATE.get().and_then(|state| {
        let state = state.lock().expect("state poisoned");
        let choice = state.current_pending_choice()?;
        choice_option_at_point(choice, px, py)
    })
}

unsafe fn user_idle_snapshot() -> Option<(Duration, u32)> {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if GetLastInputInfo(&mut info) == 0 {
        return None;
    }
    let idle_ms = GetTickCount().wrapping_sub(info.dwTime);
    Some((Duration::from_millis(idle_ms as u64), info.dwTime))
}

unsafe fn paint_window(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rect = RECT::default();
    GetClientRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    let state = APP_STATE
        .get()
        .map(|state| snapshot_state(&state.lock().expect("state poisoned")))
        .unwrap_or_else(RenderState::default);

    let frame_dc = CreateCompatibleDC(hdc);
    let frame_bitmap = CreateCompatibleBitmap(hdc, width, height);
    if !frame_dc.is_null() && !frame_bitmap.is_null() {
        let old_frame_bitmap = SelectObject(frame_dc, frame_bitmap);
        fill_rect(frame_dc, &rect, TRANSPARENT_KEY);

        let base_rect = RECT {
            left: 0,
            top: 0,
            right: WINDOW_WIDTH,
            bottom: WINDOW_HEIGHT,
        };
        render_scene(frame_dc, &base_rect, &state);
        BitBlt(hdc, 0, 0, width, height, frame_dc, 0, 0, SRCCOPY);

        SelectObject(frame_dc, old_frame_bitmap);
        DeleteObject(frame_bitmap);
        DeleteDC(frame_dc);
    } else {
        render_scene(hdc, &rect, &state);
        if !frame_bitmap.is_null() {
            DeleteObject(frame_bitmap);
        }
        if !frame_dc.is_null() {
            DeleteDC(frame_dc);
        }
    }

    EndPaint(hwnd, &ps);
}

unsafe fn paint_permission_overlay(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rect = RECT::default();
    GetClientRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    let state = APP_STATE
        .get()
        .map(|state| snapshot_state(&state.lock().expect("state poisoned")))
        .unwrap_or_else(RenderState::default);

    let frame_dc = CreateCompatibleDC(hdc);
    let frame_bitmap = CreateCompatibleBitmap(hdc, width, height);
    if !frame_dc.is_null() && !frame_bitmap.is_null() {
        let old_frame_bitmap = SelectObject(frame_dc, frame_bitmap);
        fill_rect(frame_dc, &rect, TRANSPARENT_KEY);

        let base_rect = RECT {
            left: 0,
            top: 0,
            right: PERMISSION_OVERLAY_WIDTH,
            bottom: PERMISSION_OVERLAY_HEIGHT,
        };
        render_permission_overlay(frame_dc, &base_rect, &state);
        BitBlt(hdc, 0, 0, width, height, frame_dc, 0, 0, SRCCOPY);

        SelectObject(frame_dc, old_frame_bitmap);
        DeleteObject(frame_bitmap);
        DeleteDC(frame_dc);
    } else {
        render_permission_overlay(hdc, &rect, &state);
        if !frame_bitmap.is_null() {
            DeleteObject(frame_bitmap);
        }
        if !frame_dc.is_null() {
            DeleteDC(frame_dc);
        }
    }

    EndPaint(hwnd, &ps);
}

fn loword(value: u32) -> i16 {
    (value & 0xffff) as i16
}

fn hiword(value: u32) -> i16 {
    ((value >> 16) & 0xffff) as i16
}

fn point_in(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> bool {
    px >= x && px <= x + w && py >= y && py <= y + h
}

fn point_in_tuple(px: i32, py: i32, rect: (i32, i32, i32, i32)) -> bool {
    let (x, y, w, h) = rect;
    point_in(px, py, x, y, w, h)
}
