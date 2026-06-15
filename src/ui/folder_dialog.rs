//! Native "choose folder" dialog (Win32 `SHBrowseForFolderW`), used by the
//! Settings panel to pick a custom GIF directory.

use std::path::PathBuf;

use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW,
};

use crate::util::wide;

const MAX_PATH: usize = 260;

/// Show a modal folder picker. Returns the chosen path, or `None` if the user
/// cancelled or the selection has no filesystem path.
pub(crate) fn pick_folder(title: &str) -> Option<PathBuf> {
    let title_wide = wide(title);
    let mut display = [0u16; MAX_PATH];

    let mut info: BROWSEINFOW = unsafe { std::mem::zeroed() };
    info.lpszTitle = title_wide.as_ptr();
    info.pszDisplayName = display.as_mut_ptr();
    info.ulFlags = (BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE) as u32;

    unsafe {
        let pidl = SHBrowseForFolderW(&info);
        if pidl.is_null() {
            return None;
        }
        let mut buffer = [0u16; MAX_PATH];
        let ok = SHGetPathFromIDListW(pidl, buffer.as_mut_ptr());
        CoTaskMemFree(pidl as *const core::ffi::c_void);
        if ok == 0 {
            return None;
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
    }
}
