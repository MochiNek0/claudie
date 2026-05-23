mod render;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use render::{RenderState, fill_rect, render_permission_overlay, render_scene, snapshot_state};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EndPaint, InvalidateRect, PAINTSTRUCT, SRCCOPY, ScreenToClient, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, LASTINPUTINFO, MOD_CONTROL, MOD_SHIFT, RegisterHotKey, ReleaseCapture,
    UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
    GetClientRect, GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect,
    HTCAPTION, HWND_TOPMOST, IDC_ARROW, IDC_HAND, LWA_COLORKEY, LoadCursorW, MF_SEPARATOR,
    MF_STRING, MSG, PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW, SetCursor, SetForegroundWindow,
    SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, TrackPopupMenu, TranslateMessage, WM_CREATE,
    WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY, WM_LBUTTONDOWN, WM_NCLBUTTONDOWN, WM_PAINT, WM_RBUTTONUP,
    WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use crate::app::pomodoro::PomodoroStatus;
use crate::app::{AppState, PermissionDecision};
use crate::config::*;
use crate::globals::APP_STATE;
use crate::hooks::{
    decide_current_permission, deny_current_choice, submit_current_choice,
    toggle_current_choice_option,
};
use crate::settings::{UserSettings, WindowPosition, load_user_settings, save_user_settings};
use crate::ui::settings_panel::show_settings_panel;
use crate::util::wide;

static CONTEXT_MENU_OPEN: AtomicBool = AtomicBool::new(false);

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
    SetTimer(hwnd, 1, 33, None);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
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
            if let Some(state) = APP_STATE.get() {
                let mut state = state.lock().expect("state poisoned");
                let user_idle = user_idle_snapshot();
                state.tick_pomodoro();
                state.decay_mood(
                    user_idle.map(|snapshot| snapshot.0),
                    user_idle.map(|snapshot| snapshot.1),
                );
            }
            if !CONTEXT_MENU_OPEN.load(Ordering::Relaxed) {
                ensure_pet_topmost(hwnd);
            }
            sync_permission_overlay(overlay_hwnd(hwnd));
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
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
            0
        }
        WM_RBUTTONUP => {
            show_context_menu(hwnd);
            0
        }
        WM_SETCURSOR => DefWindowProcW(hwnd, msg, wparam, lparam),
        WM_DESTROY => {
            persist_pet_window_position(hwnd);
            let overlay = overlay_hwnd(hwnd);
            if !overlay.is_null() {
                DestroyWindow(overlay);
            }
            PostQuitMessage(0);
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
    clamp_window_position(position.x, position.y, WINDOW_WIDTH, WINDOW_HEIGHT)
}

unsafe fn clamp_window_position(x: i32, y: i32, width: i32, height: i32) -> (i32, i32) {
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
    let screen_w = screen_w.max(width);
    let screen_h = screen_h.max(height);
    let max_x = (min_x + screen_w - width).max(min_x);
    let max_y = (min_y + screen_h - height).max(min_y);
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
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

fn has_pending_overlay() -> bool {
    APP_STATE.get().is_some_and(|state| {
        let state = state.lock().expect("state poisoned");
        !state.pending_permissions.is_empty() || !state.pending_choices.is_empty()
    })
}

unsafe fn base_point_from_client(_hwnd: HWND, x: i32, y: i32) -> (i32, i32) {
    if x < 0 || y < 0 || x > WINDOW_WIDTH || y > WINDOW_HEIGHT {
        return (-1, -1);
    }
    (x, y)
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

unsafe fn sync_permission_overlay(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    if !has_pending_overlay() {
        ShowWindow(hwnd, SW_HIDE);
        return;
    }

    let screen_w = GetSystemMetrics(SM_CXSCREEN);
    let screen_h = GetSystemMetrics(SM_CYSCREEN);
    let x = (screen_w - PERMISSION_OVERLAY_WIDTH) / 2;
    let y = (screen_h - PERMISSION_OVERLAY_HEIGHT) / 2;
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        x,
        y,
        PERMISSION_OVERLAY_WIDTH,
        PERMISSION_OVERLAY_HEIGHT,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

unsafe fn show_context_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }

    let settings_label = wide("Settings...");
    AppendMenuW(menu, MF_STRING, MENU_SETTINGS_ID, settings_label.as_ptr());
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
        if command as usize == MENU_SETTINGS_ID {
            show_settings_panel(hwnd);
        } else if command as usize == MENU_POMODORO_START_ID {
            with_app_state(|state| state.start_pomodoro());
        } else if command as usize == MENU_POMODORO_STOP_ID {
            with_app_state(|state| state.stop_pomodoro());
        } else if command as usize == MENU_POMODORO_PAUSE_RESUME_ID {
            with_app_state(|state| {
                if state.pomodoro.status == PomodoroStatus::Paused {
                    state.resume_pomodoro();
                } else {
                    state.pause_pomodoro();
                }
            });
        } else if command as usize == MENU_POMODORO_SKIP_ID {
            with_app_state(|state| state.skip_pomodoro());
        } else if command as usize == MENU_EXIT_ID {
            DestroyWindow(hwnd);
        }
    }

    DestroyMenu(menu);
}

fn with_app_state(action: impl FnOnce(&mut AppState)) {
    if let Some(state) = APP_STATE.get() {
        action(&mut state.lock().expect("state poisoned"));
    }
}

fn has_pending_choice() -> bool {
    APP_STATE.get().is_some_and(|state| {
        !state
            .lock()
            .expect("state poisoned")
            .pending_choices
            .is_empty()
    })
}

fn choice_option_at(px: i32, py: i32) -> Option<(usize, usize)> {
    APP_STATE.get().and_then(|state| {
        let state = state.lock().expect("state poisoned");
        let choice = state.pending_choices.front()?;
        let mut y = CHOICE_CARD_Y + 50;
        if !choice.detail.trim().is_empty() {
            y += 5 * 17 + 8;
        }
        let limit_y = CHOICE_SUBMIT_BUTTON.1 - 12;
        for (question_index, question) in choice.questions.iter().enumerate() {
            if y + 47 > limit_y {
                break;
            }
            y += 16 + 28 + 3;
            for (option_index, _) in question.options.iter().enumerate() {
                if y + CHOICE_OPTION_H > limit_y {
                    break;
                }
                if point_in_tuple(
                    px,
                    py,
                    (CHOICE_OPTION_X, y, CHOICE_OPTION_W, CHOICE_OPTION_H),
                ) {
                    return Some((question_index, option_index));
                }
                y += CHOICE_OPTION_H + 4;
            }
            y += 6;
        }
        None
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
