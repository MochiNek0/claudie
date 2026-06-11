mod render;

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use render::{
    RenderState, fill_rect, render_fishing_hud_window, render_pet_window,
    render_pomodoro_hud_window, render_session_switcher_window, session_switcher_session_at_point,
    snapshot_state,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, COLOR_MENU, COLOR_MENUHILIGHT, CreateCompatibleBitmap, CreateCompatibleDC,
    DFC_MENU, DFCS_MENUCHECK, DeleteDC, DeleteObject, DrawFrameControl, EndPaint, FillRect,
    GetSysColorBrush, GetTextExtentPoint32W, InvalidateRect, PAINTSTRUCT, SRCCOPY, ScreenToClient,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_CHECKED, ODS_SELECTED, ODT_MENU,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, LASTINPUTINFO, MOD_CONTROL, MOD_SHIFT, RegisterHotKey, ReleaseCapture,
    SetCapture, UnregisterHotKey,
};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, IDC_ARROW,
    IDC_HAND, IsWindowVisible, LWA_COLORKEY, LoadCursorW, MF_CHECKED, MF_OWNERDRAW, MF_POPUP,
    MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassW, RegisterWindowMessageW, SM_CXSCREEN,
    SM_CXSMICON, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYSMICON, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SetCursor, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN,
    TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_CREATE, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND,
    WM_HOTKEY, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MEASUREITEM, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NULL,
    WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use crate::app::fishing::FishingPhase;
use crate::app::pomodoro::{PomodoroMode, PomodoroStatus};
use crate::app::{AppState, PermissionDecision, PetMood, SessionSwitcherItem};
use crate::config::*;
use crate::globals::{APP_STATE, PET_RENDERER};
use crate::hooks::decide_current_permission;
use crate::settings::{
    LlmProfile, UserSettings, WindowPosition, apply_llm_profile_to_claude,
    ensure_claude_onboarding_complete, load_user_settings, save_llm_profile_db, save_user_settings,
};
use crate::ui::prompt_popup::{close_prompt_popup, sync_prompt_popup_for_parent};
use crate::ui::settings_panel::{close_settings_panel, show_settings_panel};
use crate::ui::window_icon::load_sized_app_icon;
use crate::usage_display::provider_usage_display;
use crate::util::{shorten, wide};

static CONTEXT_MENU_OPEN: AtomicBool = AtomicBool::new(false);
static MENU_PROFILE_DRAW_ITEMS: OnceLock<Mutex<HashMap<usize, MenuProfileDrawItem>>> =
    OnceLock::new();
static LEFT_BUTTON_CAPTURED: AtomicBool = AtomicBool::new(false);
static LEFT_BUTTON_DRAGGING: AtomicBool = AtomicBool::new(false);
static LEFT_BUTTON_SCREEN_X: AtomicI32 = AtomicI32::new(0);
static LEFT_BUTTON_SCREEN_Y: AtomicI32 = AtomicI32::new(0);
static DRAG_WINDOW_X: AtomicI32 = AtomicI32::new(0);
static DRAG_WINDOW_Y: AtomicI32 = AtomicI32::new(0);
static RIGHT_BUTTON_CAPTURED: AtomicBool = AtomicBool::new(false);
static CURRENT_TIMER_MS: AtomicU32 = AtomicU32::new(ACTIVE_TIMER_MS);
static LAST_TOPMOST_TICK: AtomicU32 = AtomicU32::new(0);
static TASKBAR_CREATED_MESSAGE: AtomicU32 = AtomicU32::new(0);
static PET_NOMINAL_ORIGIN: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();
const CLICK_DRAG_THRESHOLD_PX: i32 = 6;
const ACTIVE_TIMER_MS: u32 = 33;
const IDLE_TIMER_MS: u32 = 120;
const SLEEP_TIMER_MS: u32 = 500;
const TOPMOST_REFRESH_MS: u32 = 1_000;
const TRAY_ICON_ID: u32 = 1;
const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 1;

fn context_menu_open() -> bool {
    CONTEXT_MENU_OPEN.load(Ordering::Relaxed)
}

#[derive(Clone)]
struct MenuProfileDrawItem {
    segments: Vec<MenuProfileDrawSegment>,
}

#[derive(Clone)]
struct MenuProfileDrawSegment {
    text: String,
    color: u32,
}

struct AuxWindows {
    pomodoro_hud: HWND,
    fishing_hud: HWND,
    session_switcher: HWND,
}

/// Cheap per-window content keys; a window is only invalidated on a timer
/// tick when its key differs from the previous tick.
#[derive(Default, PartialEq)]
struct RepaintKeys {
    pet: Option<(PetMood, usize, u64, u32)>,
    pomodoro: Option<(PomodoroMode, PomodoroStatus, u64)>,
    fishing: Option<(FishingPhase, u32, u32, u32, u32)>,
    session: Option<Vec<SessionSwitcherItem>>,
}

thread_local! {
    static LAST_REPAINT_KEYS: RefCell<RepaintKeys> = RefCell::new(RepaintKeys::default());
}

struct WindowLayout {
    pet_visible_rect: PetVisibleRect,
    pomodoro_visible: bool,
    pomodoro_width: i32,
    pomodoro_height: i32,
    fishing_visible: bool,
    fishing_width: i32,
    fishing_height: i32,
    session_visible: bool,
    session_width: i32,
    session_height: i32,
}

#[derive(Clone, Copy)]
struct PetVisibleRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

pub(crate) unsafe fn run_window(port: u16) {
    let class_name = wide("ClaudieWindow");
    let pomodoro_class_name = wide("ClaudiePomodoroHud");
    let fishing_class_name = wide("ClaudieFishingHud");
    let session_class_name = wide("ClaudieSessionSwitcher");
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
    let pomodoro_wc = WNDCLASSW {
        lpfnWndProc: Some(pomodoro_hud_proc),
        lpszClassName: pomodoro_class_name.as_ptr(),
        ..wc
    };
    RegisterClassW(&pomodoro_wc);
    let fishing_wc = WNDCLASSW {
        lpfnWndProc: Some(fishing_hud_proc),
        lpszClassName: fishing_class_name.as_ptr(),
        ..wc
    };
    RegisterClassW(&fishing_wc);
    let session_wc = WNDCLASSW {
        lpfnWndProc: Some(session_switcher_proc),
        lpszClassName: session_class_name.as_ptr(),
        ..wc
    };
    RegisterClassW(&session_wc);

    let settings = current_user_settings();
    let (x, y) = initial_pet_position(&settings);
    let initial_visible_rect = visible_pet_rect_in_window(&settings, PetMood::Idle);
    let (window_w, window_h) = (initial_visible_rect.width, initial_visible_rect.height);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_POPUP | WS_VISIBLE,
        x,
        y,
        window_w,
        window_h,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null_mut(),
    );

    if hwnd == std::ptr::null_mut() {
        eprintln!("Failed to create claudie window");
        return;
    }
    remember_pet_nominal_origin_from_window(hwnd, initial_visible_rect);

    let pomodoro_hwnd = create_aux_window(
        pomodoro_class_name.as_ptr(),
        title.as_ptr(),
        POMODORO_HUD_WIDTH,
        POMODORO_HUD_HEIGHT,
        hinstance,
        hwnd,
    );
    let fishing_hwnd = create_aux_window(
        fishing_class_name.as_ptr(),
        title.as_ptr(),
        FISHING_HUD_WIDTH,
        FISHING_HUD_HEIGHT,
        hinstance,
        hwnd,
    );
    let (session_width, session_height) = session_window_size_for_settings(&settings);
    let session_hwnd = create_aux_window(
        session_class_name.as_ptr(),
        title.as_ptr(),
        session_width,
        session_height,
        hinstance,
        hwnd,
    );

    store_aux_windows(
        hwnd,
        AuxWindows {
            pomodoro_hud: pomodoro_hwnd,
            fishing_hud: fishing_hwnd,
            session_switcher: session_hwnd,
        },
    );

    register_taskbar_created_message();
    SetLayeredWindowAttributes(hwnd, TRANSPARENT_KEY, 255, LWA_COLORKEY);
    ShowWindow(hwnd, SW_SHOW);
    ensure_pet_topmost(hwnd);
    install_tray_icon(hwnd);
    RegisterHotKey(hwnd, 1, MOD_CONTROL | MOD_SHIFT, 'Y' as u32);
    RegisterHotKey(hwnd, 2, MOD_CONTROL | MOD_SHIFT, 'N' as u32);
    SetTimer(hwnd, 1, ACTIVE_TIMER_MS, None);

    if let Err(err) = slint::run_event_loop_until_quit() {
        eprintln!("Slint event loop failed: {err}");
    }

    UnregisterHotKey(hwnd, 1);
    UnregisterHotKey(hwnd, 2);
}

unsafe fn create_aux_window(
    class_name: *const u16,
    title: *const u16,
    width: i32,
    height: i32,
    hinstance: windows_sys::Win32::Foundation::HINSTANCE,
    owner: HWND,
) -> HWND {
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
        class_name,
        title,
        WS_POPUP,
        0,
        0,
        width,
        height,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null_mut(),
    );
    if !hwnd.is_null() {
        SetLayeredWindowAttributes(hwnd, TRANSPARENT_KEY, 255, LWA_COLORKEY);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, owner as isize);
    }
    hwnd
}

unsafe fn register_taskbar_created_message() {
    let message = TASKBAR_CREATED_MESSAGE.load(Ordering::Relaxed);
    if message != 0 {
        return;
    }
    let name = wide("TaskbarCreated");
    let message = RegisterWindowMessageW(name.as_ptr());
    if message != 0 {
        TASKBAR_CREATED_MESSAGE.store(message, Ordering::Relaxed);
    }
}

unsafe fn install_tray_icon(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    let mut data = tray_icon_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = TRAY_CALLBACK_MESSAGE;
    data.hIcon = load_sized_app_icon(GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
    copy_wide_truncated(&mut data.szTip, "claudie");
    Shell_NotifyIconW(NIM_ADD, &data);
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    let data = tray_icon_data(hwnd);
    Shell_NotifyIconW(NIM_DELETE, &data);
}

fn tray_icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        ..Default::default()
    }
}

fn copy_wide_truncated(target: &mut [u16], text: &str) {
    if target.is_empty() {
        return;
    }
    let text = wide(text);
    let len = text.len().saturating_sub(1).min(target.len() - 1);
    target[..len].copy_from_slice(&text[..len]);
    target[len] = 0;
}

unsafe fn handle_tray_callback(hwnd: HWND, lparam: LPARAM) {
    match lparam as u32 {
        WM_CONTEXTMENU | WM_RBUTTONUP => show_context_menu(hwnd),
        _ => {}
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let taskbar_created = TASKBAR_CREATED_MESSAGE.load(Ordering::Relaxed);
    if taskbar_created != 0 && msg == taskbar_created {
        install_tray_icon(hwnd);
        return 0;
    }

    match msg {
        WM_CREATE => {
            let _createstruct = lparam as *const CREATESTRUCTW;
            0
        }
        WM_MEASUREITEM => {
            if measure_profile_menu_item(lparam) {
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DRAWITEM => {
            if draw_profile_menu_item(lparam) {
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_PAINT => {
            paint_window(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_TIMER => {
            let mut timer_ms = IDLE_TIMER_MS;
            let mut target_layout = None;
            let mut repaint = RepaintKeys::default();
            let mut pet_mood = PetMood::Idle;
            let mut pet_scale = 100_u32;
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
                target_layout = Some(window_layout_for_state(&state));
                pet_mood = state.mood;
                pet_scale = state.settings.pet_scale_percent();
                repaint.pomodoro = (state.pomodoro.status != PomodoroStatus::Stopped).then(|| {
                    (
                        state.pomodoro.mode,
                        state.pomodoro.status,
                        state.pomodoro.remaining(&state.settings.pomodoro).as_secs(),
                    )
                });
                repaint.fishing = state.fishing.is_active().then(|| {
                    (
                        state.fishing.phase,
                        state.fishing.tension.to_bits(),
                        state.fishing.progress.to_bits(),
                        state.fishing.target_center.to_bits(),
                        state.fishing.target_half_width.to_bits(),
                    )
                });
                repaint.session =
                    session_switcher_visible(&state).then(|| state.session_switcher_items());
            }
            repaint.pet = Some(pet_frame_key(pet_mood, pet_scale));
            update_window_timer(hwnd, timer_ms);
            let menu_open = context_menu_open();
            if !menu_open {
                if let Some(layout) = target_layout {
                    sync_window_layout(hwnd, &layout);
                }
                sync_prompt_popup_for_parent(hwnd);
                if should_refresh_topmost() {
                    ensure_pet_topmost(hwnd);
                    ensure_aux_topmost(hwnd);
                }
            }
            invalidate_changed_windows(hwnd, repaint);
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
        TRAY_CALLBACK_MESSAGE => {
            handle_tray_callback(hwnd, lparam);
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
                let visible_rect = current_pet_visible_rect_in_window();
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
                remember_pet_nominal_origin(x, y, visible_rect);
                sync_current_window_layout(hwnd);
            }
            0
        }
        WM_LBUTTONUP => {
            let was_captured = LEFT_BUTTON_CAPTURED.swap(false, Ordering::Relaxed);
            let was_dragging = LEFT_BUTTON_DRAGGING.swap(false, Ordering::Relaxed);
            ReleaseCapture();
            if was_captured && !was_dragging {
                with_app_state(|state| state.interact_with_pet());
                sync_current_window_layout(hwnd);
                invalidate_pet_and_aux(hwnd);
            } else if was_dragging {
                persist_pet_window_position(hwnd);
            }
            0
        }
        WM_MOUSEWHEEL => DefWindowProcW(hwnd, msg, wparam, lparam),
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
            remove_tray_icon(hwnd);
            persist_pet_window_position(hwnd);
            flush_app_stats_now();
            destroy_aux_windows(hwnd);
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

fn window_layout_for_state(state: &AppState) -> WindowLayout {
    let pet_visible_rect = visible_pet_rect_in_window(&state.settings, state.mood);
    let (session_width, session_height) = session_window_size_for_state(state);
    WindowLayout {
        pet_visible_rect,
        pomodoro_visible: state.pomodoro.status != PomodoroStatus::Stopped,
        pomodoro_width: POMODORO_HUD_WIDTH,
        pomodoro_height: POMODORO_HUD_HEIGHT,
        fishing_visible: state.fishing.is_active(),
        fishing_width: FISHING_HUD_WIDTH,
        fishing_height: FISHING_HUD_HEIGHT,
        session_visible: session_switcher_visible(state),
        session_width,
        session_height,
    }
}

fn session_switcher_visible(state: &AppState) -> bool {
    state.settings.show_session_switcher && state.session_switcher_items().len() > 1
}

fn session_window_size_for_settings(settings: &UserSettings) -> (i32, i32) {
    let (pet_width, _) = scaled_pet_size_for_percent(settings.pet_scale_percent());
    (
        pet_width.max(SESSION_SWITCHER_MIN_WIDTH),
        session_window_height_for_rows(1),
    )
}

fn session_window_size_for_state(state: &AppState) -> (i32, i32) {
    let (width, _) = session_window_size_for_settings(&state.settings);
    let rows = state
        .session_switcher_items()
        .len()
        .min(SESSION_SWITCHER_MAX_VISIBLE_ITEMS)
        .max(1);
    (width, session_window_height_for_rows(rows))
}

fn session_window_height_for_rows(rows: usize) -> i32 {
    let rows = rows.max(1) as i32;
    SESSION_SWITCHER_VERTICAL_PADDING * 2 + rows * SESSION_BAR_HEIGHT + (rows - 1)
}

fn pet_window_size_for_settings(settings: &UserSettings, mood: PetMood) -> (i32, i32) {
    let rect = visible_pet_rect_in_window(settings, mood);
    (rect.width.max(1), rect.height.max(1))
}

unsafe fn initial_pet_position(settings: &UserSettings) -> (i32, i32) {
    let Some(position) = settings.window_position else {
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    };
    let (width, height) = pet_window_size_for_settings(settings, PetMood::Idle);
    clamp_window_position(position.x, position.y, (0, 0, width, height))
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
    let (width, height) = pet_window_size_for_settings(&settings, mood);
    (0, 0, width, height)
}

fn current_pet_visible_rect_in_window() -> PetVisibleRect {
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
    let (w, h) = scaled_pet_size_for_percent(settings.pet_scale_percent());
    (PET_X, PET_Y, w, h)
}

fn visible_pet_rect_in_window(settings: &UserSettings, mood: PetMood) -> PetVisibleRect {
    let nominal_rect = nominal_pet_rect_in_window(settings);
    let Some(bounds) = PET_RENDERER.get().and_then(|store| {
        store
            .lock()
            .expect("pet renderer poisoned")
            .visible_bounds(mood)
    }) else {
        let (x, y, width, height) = nominal_rect;
        return PetVisibleRect {
            x,
            y,
            width,
            height,
        };
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
    PetVisibleRect {
        x: left,
        y: top,
        width: (right - left).max(1),
        height: (bottom - top).max(1),
    }
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
    if context_menu_open() {
        return;
    }
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
    let current_visible_rect = current_pet_visible_rect_in_window();
    let (origin_x, origin_y) = pet_nominal_origin(&rect, current_visible_rect);
    let idle_visible_rect = visible_pet_rect_in_window(&settings, PetMood::Idle);
    settings.window_position = Some(WindowPosition {
        x: origin_x + idle_visible_rect.x,
        y: origin_y + idle_visible_rect.y,
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

fn desired_timer_interval(state: &AppState) -> u32 {
    let pending = !state.pending_permissions.is_empty() || !state.pending_choices.is_empty();
    if pending
        || state.mood.is_active_work()
        || state.pomodoro.status != PomodoroStatus::Stopped
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

unsafe fn sync_window_layout(hwnd: HWND, layout: &WindowLayout) {
    if context_menu_open() {
        return;
    }
    resize_pet_window_if_needed(hwnd, layout.pet_visible_rect);
    let Some(aux) = aux_windows(hwnd) else {
        return;
    };
    let mut pet_rect = RECT::default();
    if GetWindowRect(hwnd, &mut pet_rect) == 0 {
        return;
    }
    sync_pomodoro_window(&pet_rect, aux.pomodoro_hud, layout);
    sync_fishing_window(&pet_rect, aux.fishing_hud, layout);
    sync_session_window(&pet_rect, aux.session_switcher, layout);
}

unsafe fn sync_current_window_layout(hwnd: HWND) {
    let Some(state) = APP_STATE.get() else {
        return;
    };
    let layout = {
        let state = state.lock().expect("state poisoned");
        window_layout_for_state(&state)
    };
    sync_window_layout(hwnd, &layout);
}

unsafe fn resize_pet_window_if_needed(hwnd: HWND, visible_rect: PetVisibleRect) {
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return;
    }
    let width = visible_rect.width;
    let height = visible_rect.height;
    let (origin_x, origin_y) = pet_nominal_origin(&rect, visible_rect);
    let target_x = origin_x + visible_rect.x;
    let target_y = origin_y + visible_rect.y;
    let (x, y) = clamp_window_position(target_x, target_y, (0, 0, width, height));
    if rect.right - rect.left == width
        && rect.bottom - rect.top == height
        && rect.left == x
        && rect.top == y
    {
        return;
    }
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        x,
        y,
        width,
        height,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
}

fn pet_nominal_origin_store() -> &'static Mutex<Option<(i32, i32)>> {
    PET_NOMINAL_ORIGIN.get_or_init(|| Mutex::new(None))
}

fn pet_nominal_origin(window_rect: &RECT, visible_rect: PetVisibleRect) -> (i32, i32) {
    let mut origin = pet_nominal_origin_store()
        .lock()
        .expect("pet nominal origin poisoned");
    if let Some(origin) = *origin {
        return origin;
    }
    let current = (
        window_rect.left - visible_rect.x,
        window_rect.top - visible_rect.y,
    );
    *origin = Some(current);
    current
}

fn remember_pet_nominal_origin(x: i32, y: i32, visible_rect: PetVisibleRect) {
    *pet_nominal_origin_store()
        .lock()
        .expect("pet nominal origin poisoned") = Some((x - visible_rect.x, y - visible_rect.y));
}

unsafe fn remember_pet_nominal_origin_from_window(hwnd: HWND, visible_rect: PetVisibleRect) {
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect) != 0 {
        remember_pet_nominal_origin(rect.left, rect.top, visible_rect);
    }
}

unsafe fn sync_pomodoro_window(pet_rect: &RECT, hwnd: HWND, layout: &WindowLayout) {
    if hwnd.is_null() {
        return;
    }
    if !layout.pomodoro_visible {
        hide_aux_window(hwnd);
        return;
    }
    let (x, y) = pomodoro_window_position(pet_rect, layout.pomodoro_width, layout.pomodoro_height);
    show_aux_window_at(hwnd, x, y, layout.pomodoro_width, layout.pomodoro_height);
}

unsafe fn sync_fishing_window(pet_rect: &RECT, hwnd: HWND, layout: &WindowLayout) {
    if hwnd.is_null() {
        return;
    }
    if !layout.fishing_visible {
        hide_aux_window(hwnd);
        return;
    }
    let (x, y) = fishing_window_position(pet_rect, layout.fishing_width, layout.fishing_height);
    show_aux_window_at(hwnd, x, y, layout.fishing_width, layout.fishing_height);
}

unsafe fn sync_session_window(pet_rect: &RECT, hwnd: HWND, layout: &WindowLayout) {
    if hwnd.is_null() {
        return;
    }
    if !layout.session_visible {
        hide_aux_window(hwnd);
        return;
    }
    let (x, y) = session_window_position(pet_rect, layout.session_width, layout.session_height);
    show_aux_window_at(hwnd, x, y, layout.session_width, layout.session_height);
}

unsafe fn hide_aux_window(hwnd: HWND) {
    if IsWindowVisible(hwnd) != 0 {
        ShowWindow(hwnd, SW_HIDE);
    }
}

/// Move/show an aux window, skipping the SetWindowPos syscall when it is
/// already visible at the requested rect (the per-tick steady state).
unsafe fn show_aux_window_at(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    let mut rect = RECT::default();
    if IsWindowVisible(hwnd) != 0
        && GetWindowRect(hwnd, &mut rect) != 0
        && rect.left == x
        && rect.top == y
        && rect.right - rect.left == width
        && rect.bottom - rect.top == height
    {
        return;
    }
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        x,
        y,
        width,
        height,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
}

unsafe fn pomodoro_window_position(pet_rect: &RECT, width: i32, height: i32) -> (i32, i32) {
    const GAP: i32 = 6;
    let (_, screen_y, _, screen_h) = virtual_screen_bounds();
    let screen_bottom = screen_y + screen_h;
    let pet_w = pet_rect.right - pet_rect.left;
    let x = pet_rect.left + (pet_w - width) / 2;
    let above_y = pet_rect.top - height - GAP;
    let below_y = pet_rect.bottom + GAP;
    let y = if above_y >= screen_y {
        above_y
    } else if below_y + height <= screen_bottom {
        below_y
    } else {
        above_y
    };
    clamp_window_position(x, y, (0, 0, width, height))
}

unsafe fn fishing_window_position(pet_rect: &RECT, width: i32, height: i32) -> (i32, i32) {
    const GAP: i32 = 8;
    let (screen_x, _, screen_w, _) = virtual_screen_bounds();
    let screen_right = screen_x + screen_w;
    let right_x = pet_rect.right + GAP;
    let left_x = pet_rect.left - width - GAP;
    let x = if right_x + width <= screen_right {
        right_x
    } else if left_x >= screen_x {
        left_x
    } else {
        right_x
    };
    let pet_h = pet_rect.bottom - pet_rect.top;
    let y = pet_rect.top + (pet_h - height) / 2;
    clamp_window_position(x, y, (0, 0, width, height))
}

unsafe fn session_window_position(pet_rect: &RECT, width: i32, height: i32) -> (i32, i32) {
    let (_, screen_y, _, screen_h) = virtual_screen_bounds();
    let screen_bottom = screen_y + screen_h;
    let pet_w = pet_rect.right - pet_rect.left;
    let x = pet_rect.left + (pet_w - width) / 2;
    let below_y = pet_rect.bottom + SESSION_BAR_GAP;
    let above_y = pet_rect.top - SESSION_BAR_GAP - height;
    let y = if below_y + height <= screen_bottom {
        below_y
    } else if above_y >= screen_y {
        above_y
    } else {
        below_y
    };
    clamp_window_position(x, y, (0, 0, width, height))
}

unsafe fn virtual_screen_bounds() -> (i32, i32, i32, i32) {
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
    (min_x, min_y, screen_w, screen_h)
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

unsafe fn base_point_from_client(hwnd: HWND, x: i32, y: i32) -> (i32, i32) {
    let mut rect = RECT::default();
    if GetClientRect(hwnd, &mut rect) == 0 {
        return (-1, -1);
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if x < 0 || y < 0 || x > width || y > height {
        return (-1, -1);
    }
    (x, y)
}

unsafe fn cursor_over_pet(hwnd: HWND) -> bool {
    let mut point = POINT { x: 0, y: 0 };
    if GetCursorPos(&mut point) == 0 || ScreenToClient(hwnd, &mut point) == 0 {
        return false;
    }
    let (x, y, w, h) = current_pet_rect_in_window();
    point.x >= x && point.x <= x + w && point.y >= y && point.y <= y + h
}

unsafe extern "system" fn pomodoro_hud_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_pomodoro_hud(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn fishing_hud_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_fishing_hud(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_LBUTTONDOWN => {
            with_app_state(|state| {
                state.handle_fishing_input();
            });
            let pet_hwnd = parent_pet_hwnd(hwnd);
            sync_current_window_layout(pet_hwnd);
            invalidate_pet_and_aux(pet_hwnd);
            0
        }
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND));
            1
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn session_switcher_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_session_switcher(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_LBUTTONDOWN => {
            let x = loword(lparam as u32) as i32;
            let y = hiword(lparam as u32) as i32;
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect) != 0
                && let Some(session_id) = session_switcher_session_at(
                    x,
                    y,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                )
            {
                with_app_state(|state| state.focus_session(&session_id));
                let pet_hwnd = parent_pet_hwnd(hwnd);
                sync_current_window_layout(pet_hwnd);
                invalidate_pet_and_aux(pet_hwnd);
            }
            0
        }
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND));
            1
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn parent_pet_hwnd(hwnd: HWND) -> HWND {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HWND
}

unsafe fn store_aux_windows(hwnd: HWND, windows: AuxWindows) {
    let boxed = Box::new(windows);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(boxed) as isize);
}

unsafe fn aux_windows(hwnd: HWND) -> Option<&'static AuxWindows> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const AuxWindows;
    (!ptr.is_null()).then(|| &*ptr)
}

unsafe fn take_aux_windows(hwnd: HWND) -> Option<Box<AuxWindows>> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AuxWindows;
    if ptr.is_null() {
        return None;
    }
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    Some(Box::from_raw(ptr))
}

unsafe fn destroy_aux_windows(hwnd: HWND) {
    let Some(windows) = take_aux_windows(hwnd) else {
        return;
    };
    for aux_hwnd in [
        windows.pomodoro_hud,
        windows.fishing_hud,
        windows.session_switcher,
    ] {
        if !aux_hwnd.is_null() {
            DestroyWindow(aux_hwnd);
        }
    }
}

unsafe fn ensure_aux_topmost(hwnd: HWND) {
    if context_menu_open() {
        return;
    }
    let Some(windows) = aux_windows(hwnd) else {
        return;
    };
    for aux_hwnd in [
        windows.pomodoro_hud,
        windows.fishing_hud,
        windows.session_switcher,
    ] {
        if !aux_hwnd.is_null() {
            SetWindowPos(
                aux_hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

unsafe fn invalidate_aux_windows(hwnd: HWND) {
    let Some(windows) = aux_windows(hwnd) else {
        return;
    };
    for aux_hwnd in [
        windows.pomodoro_hud,
        windows.fishing_hud,
        windows.session_switcher,
    ] {
        if !aux_hwnd.is_null() {
            InvalidateRect(aux_hwnd, std::ptr::null(), 0);
        }
    }
}

unsafe fn invalidate_pet_and_aux(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    InvalidateRect(hwnd, std::ptr::null(), 0);
    invalidate_aux_windows(hwnd);
}

fn pet_frame_key(mood: PetMood, scale_percent: u32) -> (PetMood, usize, u64, u32) {
    let (mood, frame, generation) = PET_RENDERER
        .get()
        .map(|store| {
            store
                .lock()
                .expect("pet renderer poisoned")
                .frame_signature(mood)
        })
        .unwrap_or((mood, 0, 0));
    (mood, frame, generation, scale_percent)
}

unsafe fn invalidate_changed_windows(hwnd: HWND, keys: RepaintKeys) {
    let (pet, pomodoro, fishing, session) = LAST_REPAINT_KEYS.with(|last| {
        let mut last = last.borrow_mut();
        let changed = (
            last.pet != keys.pet,
            last.pomodoro != keys.pomodoro,
            last.fishing != keys.fishing,
            last.session != keys.session,
        );
        *last = keys;
        changed
    });
    if pet {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
    let Some(aux) = aux_windows(hwnd) else {
        return;
    };
    for (changed, aux_hwnd) in [
        (pomodoro, aux.pomodoro_hud),
        (fishing, aux.fishing_hud),
        (session, aux.session_switcher),
    ] {
        if changed && !aux_hwnd.is_null() {
            InvalidateRect(aux_hwnd, std::ptr::null(), 0);
        }
    }
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
        ensure_pet_topmost(hwnd);
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
        PostMessageW(hwnd, WM_NULL, 0, 0);
        CONTEXT_MENU_OPEN.store(false, Ordering::Relaxed);
        sync_current_window_layout(hwnd);
        sync_prompt_popup_for_parent(hwnd);
        ensure_pet_topmost(hwnd);
        ensure_aux_topmost(hwnd);
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
            sync_current_window_layout(hwnd);
            invalidate_pet_and_aux(hwnd);
        } else if command == MENU_FISHING_STOP_ID {
            with_app_state(|state| state.stop_fishing());
            sync_current_window_layout(hwnd);
            invalidate_pet_and_aux(hwnd);
        } else if command == MENU_EXIT_ID {
            DestroyWindow(hwnd);
        }
    }

    DestroyMenu(menu);
}

unsafe fn append_llm_profile_menu(menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU) {
    // Drop entries from previous menu opens so the store only describes the
    // owner-drawn items appended below.
    clear_menu_profile_draw_items();
    let entries = llm_profile_menu_entries();
    if entries.is_empty() {
        return;
    }
    let profile_menu = CreatePopupMenu();
    if profile_menu.is_null() {
        return;
    }

    for (index, label, checked, draw_item) in entries {
        let command_id = MENU_LLM_PROFILE_BASE_ID + index;
        let checked_flag = if checked { MF_CHECKED } else { 0 };
        if let Some(draw_item) = draw_item {
            store_menu_profile_draw_item(command_id, draw_item);
            AppendMenuW(
                profile_menu,
                MF_OWNERDRAW | checked_flag,
                command_id,
                command_id as *const u16,
            );
        } else {
            let label = wide(&label);
            AppendMenuW(
                profile_menu,
                MF_STRING | checked_flag,
                command_id,
                label.as_ptr(),
            );
        }
    }

    let profile_label = wide("LLM Profile");
    AppendMenuW(
        menu,
        MF_POPUP,
        profile_menu as usize,
        profile_label.as_ptr(),
    );
}

fn llm_profile_menu_entries() -> Vec<(usize, String, bool, Option<MenuProfileDrawItem>)> {
    let Some(state) = APP_STATE.get() else {
        return Vec::new();
    };
    let state = state.lock().expect("state poisoned");
    let mut db = state.llm_profiles.clone();
    let quota = state.quota.clone();
    drop(state);
    db.normalize();
    db.profiles
        .iter()
        .take(MENU_LLM_PROFILE_MAX_ITEMS)
        .enumerate()
        .map(|(index, profile)| {
            let (label, color) =
                profile_menu_label_with_usage(profile, &db.active_profile_id, &quota);
            (index, label, profile.id == db.active_profile_id, color)
        })
        .collect()
}

fn profile_menu_label_with_usage(
    profile: &LlmProfile,
    active_profile_id: &str,
    quota: &crate::app::QuotaStats,
) -> (String, Option<MenuProfileDrawItem>) {
    let label = profile_menu_label(profile);
    if !profile.is_official() {
        return (label, None);
    }
    let usage = provider_usage_display(&label, &profile.id, active_profile_id, quota);
    if !usage.has_usage {
        return (label, None);
    }
    let item = MenuProfileDrawItem {
        segments: vec![
            MenuProfileDrawSegment {
                // Keep the fixed-width usage suffix within the measured item
                // width even for long profile labels.
                text: shorten(&label, 26),
                color: rgb(17, 24, 39),
            },
            MenuProfileDrawSegment {
                text: "  ".to_string(),
                color: rgb(17, 24, 39),
            },
            MenuProfileDrawSegment {
                text: format!("5h {}", usage.five_hour.value),
                color: profile_usage_window_menu_color(usage.five_hour.percent, rgb(10, 132, 255)),
            },
            MenuProfileDrawSegment {
                text: " / ".to_string(),
                color: rgb(17, 24, 39),
            },
            MenuProfileDrawSegment {
                text: format!("7d {}", usage.seven_day.value),
                color: profile_usage_window_menu_color(usage.seven_day.percent, rgb(124, 92, 196)),
            },
        ],
    };
    (label, Some(item))
}

fn profile_usage_window_menu_color(percent: Option<u8>, default_color: u32) -> u32 {
    let percent = percent.unwrap_or(0);
    if percent >= 90 {
        rgb(214, 69, 69)
    } else if percent >= 60 {
        rgb(216, 138, 36)
    } else {
        default_color
    }
}

fn store_menu_profile_draw_item(id: usize, item: MenuProfileDrawItem) {
    let items = MENU_PROFILE_DRAW_ITEMS.get_or_init(|| Mutex::new(HashMap::new()));
    items
        .lock()
        .expect("profile menu item store poisoned")
        .insert(id, item);
}

fn clear_menu_profile_draw_items() {
    if let Some(items) = MENU_PROFILE_DRAW_ITEMS.get() {
        items
            .lock()
            .expect("profile menu item store poisoned")
            .clear();
    }
}

fn menu_profile_draw_item(id: usize) -> Option<MenuProfileDrawItem> {
    MENU_PROFILE_DRAW_ITEMS
        .get()?
        .lock()
        .expect("profile menu item store poisoned")
        .get(&id)
        .cloned()
}

unsafe fn measure_profile_menu_item(lparam: LPARAM) -> bool {
    let measure = lparam as *mut MEASUREITEMSTRUCT;
    if measure.is_null()
        || (*measure).CtlType != ODT_MENU
        || !has_profile_draw_item((*measure).itemID as usize)
    {
        return false;
    }
    (*measure).itemWidth = 292;
    (*measure).itemHeight = 24;
    true
}

unsafe fn draw_profile_menu_item(lparam: LPARAM) -> bool {
    let draw = lparam as *const DRAWITEMSTRUCT;
    if draw.is_null() || (*draw).CtlType != ODT_MENU {
        return false;
    }
    let Some(item) = menu_profile_draw_item((*draw).itemID as usize) else {
        return false;
    };

    let rect = (*draw).rcItem;
    let selected = (*draw).itemState & ODS_SELECTED != 0;
    let checked = (*draw).itemState & ODS_CHECKED != 0;
    let background = if selected {
        COLOR_MENUHILIGHT
    } else {
        COLOR_MENU
    };
    FillRect((*draw).hDC, &rect, GetSysColorBrush(background));

    SetBkMode((*draw).hDC, TRANSPARENT as i32);
    if checked {
        let mut check_rect = RECT {
            left: rect.left + 5,
            top: rect.top + 5,
            right: rect.left + 18,
            bottom: rect.top + 18,
        };
        DrawFrameControl((*draw).hDC, &mut check_rect, DFC_MENU, DFCS_MENUCHECK);
    }
    let color_override = if selected {
        Some(rgb(255, 255, 255))
    } else {
        None
    };
    draw_menu_profile_segments(
        (*draw).hDC,
        rect.left + 24,
        rect.top + 5,
        &item.segments,
        color_override,
    );
    true
}

unsafe fn draw_menu_profile_segments(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    mut x: i32,
    y: i32,
    segments: &[MenuProfileDrawSegment],
    color_override: Option<u32>,
) {
    for segment in segments {
        let text = wide(&segment.text);
        let len = (text.len() - 1) as i32;
        SetTextColor(hdc, color_override.unwrap_or(segment.color));
        TextOutW(hdc, x, y, text.as_ptr(), len);

        let mut size = SIZE { cx: 0, cy: 0 };
        if GetTextExtentPoint32W(hdc, text.as_ptr(), len, &mut size) != 0 {
            x += size.cx;
        }
    }
}

fn has_profile_draw_item(id: usize) -> bool {
    MENU_PROFILE_DRAW_ITEMS.get().is_some_and(|items| {
        items
            .lock()
            .expect("profile menu item store poisoned")
            .contains_key(&id)
    })
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

fn session_switcher_session_at(px: i32, py: i32, width: i32, height: i32) -> Option<String> {
    APP_STATE.get().and_then(|state| {
        let state = state.lock().expect("state poisoned");
        session_switcher_session_at_point(&state, px, py, width, height)
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

    let (state, pet_offset_x, pet_offset_y) = APP_STATE
        .get()
        .map(|state| {
            let state = state.lock().expect("state poisoned");
            let visible_rect = visible_pet_rect_in_window(&state.settings, state.mood);
            (snapshot_state(&state), -visible_rect.x, -visible_rect.y)
        })
        .unwrap_or_else(|| (RenderState::default(), 0, 0));

    let frame_dc = CreateCompatibleDC(hdc);
    let frame_bitmap = CreateCompatibleBitmap(hdc, width, height);
    if !frame_dc.is_null() && !frame_bitmap.is_null() {
        let old_frame_bitmap = SelectObject(frame_dc, frame_bitmap);
        fill_rect(frame_dc, &rect, TRANSPARENT_KEY);

        let base_rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        render_pet_window(frame_dc, &base_rect, &state, pet_offset_x, pet_offset_y);
        BitBlt(hdc, 0, 0, width, height, frame_dc, 0, 0, SRCCOPY);

        SelectObject(frame_dc, old_frame_bitmap);
        DeleteObject(frame_bitmap);
        DeleteDC(frame_dc);
    } else {
        render_pet_window(hdc, &rect, &state, pet_offset_x, pet_offset_y);
        if !frame_bitmap.is_null() {
            DeleteObject(frame_bitmap);
        }
        if !frame_dc.is_null() {
            DeleteDC(frame_dc);
        }
    }

    EndPaint(hwnd, &ps);
}

unsafe fn paint_pomodoro_hud(hwnd: HWND) {
    paint_state_window(hwnd, render_pomodoro_hud_window);
}

unsafe fn paint_fishing_hud(hwnd: HWND) {
    paint_state_window(hwnd, render_fishing_hud_window);
}

unsafe fn paint_session_switcher(hwnd: HWND) {
    paint_state_window(hwnd, render_session_switcher_window);
}

unsafe fn paint_state_window(
    hwnd: HWND,
    render: fn(windows_sys::Win32::Graphics::Gdi::HDC, &RECT, &RenderState),
) {
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
            right: width,
            bottom: height,
        };
        render(frame_dc, &base_rect, &state);
        BitBlt(hdc, 0, 0, width, height, frame_dc, 0, 0, SRCCOPY);

        SelectObject(frame_dc, old_frame_bitmap);
        DeleteObject(frame_bitmap);
        DeleteDC(frame_dc);
    } else {
        render(hdc, &rect, &state);
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

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}
