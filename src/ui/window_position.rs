use std::mem::size_of;

use slint::PhysicalPosition;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

pub(crate) fn center_window_on_screen(
    window: &slint::Window,
    parent: HWND,
    fallback_logical_size: (f32, f32),
) {
    let Some(bounds) = screen_bounds(parent) else {
        return;
    };
    let (width, height) = window_outer_size(window).unwrap_or_else(|| {
        let size = window.size();
        if size.width > 0 && size.height > 0 {
            (size.width as i32, size.height as i32)
        } else {
            let scale = window.scale_factor();
            (
                (fallback_logical_size.0 * scale).round().max(1.0) as i32,
                (fallback_logical_size.1 * scale).round().max(1.0) as i32,
            )
        }
    });
    if width <= 0 || height <= 0 {
        return;
    }

    let screen_width = bounds.right - bounds.left;
    let screen_height = bounds.bottom - bounds.top;
    if screen_width <= 0 || screen_height <= 0 {
        return;
    }

    let x = bounds.left + (screen_width - width) / 2;
    let y = bounds.top + (screen_height - height) / 2;
    window.set_position(PhysicalPosition::new(x, y));
}

fn window_outer_size(window: &slint::Window) -> Option<(i32, i32)> {
    use slint::winit_030::WinitWindowAccessor;

    window
        .with_winit_window(|winit_window| {
            let size = winit_window.outer_size();
            (size.width as i32, size.height as i32)
        })
        .filter(|(width, height)| *width > 0 && *height > 0)
}

fn screen_bounds(parent: HWND) -> Option<RECT> {
    monitor_bounds(parent).or_else(primary_screen_bounds)
}

fn monitor_bounds(parent: HWND) -> Option<RECT> {
    unsafe {
        let monitor = MonitorFromWindow(parent, MONITOR_DEFAULTTOPRIMARY);
        if monitor.is_null() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let bounds = info.rcMonitor;
        (bounds.right > bounds.left && bounds.bottom > bounds.top).then_some(bounds)
    }
}

fn primary_screen_bounds() -> Option<RECT> {
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        (width > 0 && height > 0).then_some(RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        })
    }
}
