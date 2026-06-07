use std::time::Duration;

use slint::{ComponentHandle, Timer};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GCLP_HICON, GCLP_HICONSM, GetSystemMetrics, HICON, ICON_BIG, ICON_SMALL, ICON_SMALL2,
    IMAGE_ICON, LR_DEFAULTCOLOR, LR_LOADFROMFILE, LR_SHARED, LoadIconW, LoadImageW, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SendMessageW, SetClassLongPtrW, SetWindowPos, WM_SETICON,
};

use crate::ui::slint_views::{PromptWindow, SettingsWindow};
use crate::util::wide;

pub(crate) fn apply_slint_window_icons(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::dpi::PhysicalSize;
    use slint::winit_030::winit::platform::windows::{IconExtWindows, WindowExtWindows};
    use slint::winit_030::winit::window::Icon;

    window.with_winit_window(|winit_window| {
        if let Ok(icon) = Icon::from_resource(1, Some(PhysicalSize::new(16, 16))) {
            winit_window.set_window_icon(Some(icon));
        }
        if let Ok(icon) = Icon::from_resource(1, Some(PhysicalSize::new(32, 32))) {
            winit_window.set_taskbar_icon(Some(icon));
        }
        set_win32_titlebar_icon(winit_window);
    });
}

pub(crate) fn schedule_settings_window_icon_refresh(weak: slint::Weak<SettingsWindow>) {
    Timer::single_shot(Duration::from_millis(100), move || {
        if let Some(window) = weak.upgrade() {
            apply_slint_window_icons(window.window());
        }
    });
}

pub(crate) fn schedule_prompt_window_icon_refresh(weak: slint::Weak<PromptWindow>) {
    Timer::single_shot(Duration::from_millis(100), move || {
        if let Some(window) = weak.upgrade() {
            apply_slint_window_icons(window.window());
        }
    });
}

fn set_win32_titlebar_icon(winit_window: &slint::winit_030::winit::window::Window) {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = winit_window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;
    unsafe {
        let small_icon =
            load_sized_app_icon(GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        let big_icon =
            load_sized_app_icon(GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON));
        if !small_icon.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small_icon as isize);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL2 as usize, small_icon as isize);
            SetClassLongPtrW(hwnd, GCLP_HICONSM, small_icon as isize);
        }
        if !big_icon.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big_icon as isize);
            SetClassLongPtrW(hwnd, GCLP_HICON, big_icon as isize);
        }
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
    }
}

pub(crate) unsafe fn load_sized_app_icon(width: i32, height: i32) -> HICON {
    let resource_icon = LoadImageW(
        GetModuleHandleW(std::ptr::null()),
        1_usize as *const u16,
        IMAGE_ICON,
        width,
        height,
        LR_DEFAULTCOLOR | LR_SHARED,
    ) as HICON;
    if !resource_icon.is_null() {
        return resource_icon;
    }

    let manifest_icon_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("icon.ico");
    if let Some(icon) = load_icon_from_file(&manifest_icon_path, width, height) {
        return icon;
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let packaged_icon_path = exe_dir.join("assets").join("icon.ico");
            if let Some(icon) = load_icon_from_file(&packaged_icon_path, width, height) {
                return icon;
            }
        }
    }

    LoadIconW(GetModuleHandleW(std::ptr::null()), 1_usize as *const u16)
}

unsafe fn load_icon_from_file(path: &std::path::Path, width: i32, height: i32) -> Option<HICON> {
    let path = path.to_string_lossy();
    let path = wide(&path);
    let icon = LoadImageW(
        std::ptr::null_mut(),
        path.as_ptr(),
        IMAGE_ICON,
        width,
        height,
        LR_DEFAULTCOLOR | LR_LOADFROMFILE,
    ) as HICON;
    (!icon.is_null()).then_some(icon)
}
