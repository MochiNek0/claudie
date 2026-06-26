//! Minimal Win32 clipboard writer for copying short UTF-16 text (LLM profile
//! launch commands). Kept tiny and close to the FFI boundary.

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

use crate::util::wide;

/// `CF_UNICODETEXT`; declared locally so the whole `Win32_System_Ole` feature
/// need not be pulled in just for one constant.
const CF_UNICODETEXT: u32 = 13;

/// Copy `text` to the Windows clipboard as Unicode. Best-effort: any failure
/// leaves the clipboard untouched and returns `false`.
pub(crate) fn copy_text(text: &str) -> bool {
    let utf16 = wide(text); // already NUL-terminated
    let bytes = utf16.len() * std::mem::size_of::<u16>();
    unsafe {
        let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if hmem.is_null() {
            return false;
        }
        let ptr = GlobalLock(hmem) as *mut u16;
        if ptr.is_null() {
            // Leaking this just-allocated block on the (essentially
            // unreachable) lock failure is preferable to pulling another
            // binding; the process is short-lived.
            return false;
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
        GlobalUnlock(hmem);

        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let set = SetClipboardData(CF_UNICODETEXT, hmem as HANDLE);
        CloseClipboard();
        // On success the system takes ownership of hmem.
        !set.is_null()
    }
}
