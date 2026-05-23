#[cfg(windows)]
pub(crate) fn notify_user(title: &str, message: &str, error: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW,
    };

    use crate::util::wide;

    let title = wide(title);
    let message = wide(message);
    let icon = if error {
        MB_ICONERROR
    } else {
        MB_ICONINFORMATION
    };
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | icon,
        );
    }
}

#[cfg(not(windows))]
pub(crate) fn notify_user(title: &str, message: &str, error: bool) {
    if error {
        eprintln!("{title}: {message}");
    }
}
