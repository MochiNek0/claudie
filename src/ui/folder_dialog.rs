//! Native "choose folder" dialog, used by the Settings panel to pick a custom
//! GIF directory.
//!
//! Uses the modern Vista+ `IFileOpenDialog` (Common Item Dialog) in
//! folder-pick mode instead of the legacy `SHBrowseForFolderW`. The old API
//! eagerly enumerated the whole Shell namespace tree on the calling thread,
//! which could stall the UI for seconds when a disconnected network/mapped
//! drive or slow shell extension was present; the Common Item Dialog opens on
//! the same fast path Explorer uses and does not block like that.
//!
//! `windows-sys` exposes the COM *functions* and CLSIDs but not the COM
//! *interfaces*, so we declare the minimal vtables we call by hand.

use core::ffi::c_void;
use std::path::PathBuf;
use std::ptr;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows_sys::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, SIGDN, SIGDN_FILESYSPATH,
};
use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};

use crate::util::wide;

const S_OK: HRESULT = 0;
const S_FALSE: HRESULT = 1;

// IID_IFileOpenDialog {d57c7288-d4ad-4768-be02-9d969532d960}
const IID_IFILEOPENDIALOG: GUID = GUID::from_u128(0xd57c7288_d4ad_4768_be02_9d969532d960);

/// Flattened vtable for `IFileOpenDialog` up to the methods we use. Methods we
/// never call are typed as opaque `usize` slots purely to keep their vtable
/// offsets correct; their order follows IUnknown -> IModalWindow -> IFileDialog.
#[repr(C)]
struct IFileDialogVtbl {
    // IUnknown
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IModalWindow
    show: unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT,
    // IFileDialog
    set_file_types: usize,
    set_file_type_index: usize,
    get_file_type_index: usize,
    advise: usize,
    unadvise: usize,
    set_options: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    get_options: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    set_default_folder: usize,
    set_folder: usize,
    get_folder: usize,
    get_current_selection: usize,
    set_file_name: usize,
    get_file_name: usize,
    set_title: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    set_ok_button_label: usize,
    set_file_name_label: usize,
    get_result: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct IFileDialog {
    vtbl: *const IFileDialogVtbl,
}

/// Flattened vtable for `IShellItem` up to `GetDisplayName`.
#[repr(C)]
struct IShellItemVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    bind_to_handler: usize,
    get_parent: usize,
    get_display_name: unsafe extern "system" fn(*mut c_void, SIGDN, *mut PWSTR) -> HRESULT,
}

#[repr(C)]
struct IShellItem {
    vtbl: *const IShellItemVtbl,
}

/// Show a modal folder picker owned by `owner` (may be null). Returns the
/// chosen path, or `None` if the user cancelled or an error occurred.
pub(crate) fn pick_folder(title: &str, owner: HWND) -> Option<PathBuf> {
    unsafe {
        // The Common Item Dialog needs COM initialized on this (STA) thread.
        // winit usually already did this; a redundant init returns S_FALSE and
        // must still be balanced, while RPC_E_CHANGED_MODE must not.
        let hr_init = CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let should_uninit = hr_init == S_OK || hr_init == S_FALSE;

        let result = pick_folder_inner(title, owner);

        if should_uninit {
            CoUninitialize();
        }
        result
    }
}

unsafe fn pick_folder_inner(title: &str, owner: HWND) -> Option<PathBuf> {
    let mut dialog: *mut IFileDialog = ptr::null_mut();
    let hr = CoCreateInstance(
        &FileOpenDialog,
        ptr::null_mut(),
        CLSCTX_INPROC_SERVER,
        &IID_IFILEOPENDIALOG,
        &mut dialog as *mut _ as *mut *mut c_void,
    );
    if hr < 0 || dialog.is_null() {
        return None;
    }

    let result = configure_and_show(dialog, title, owner);
    ((*(*dialog).vtbl).release)(dialog as *mut c_void);
    result
}

unsafe fn configure_and_show(
    dialog: *mut IFileDialog,
    title: &str,
    owner: HWND,
) -> Option<PathBuf> {
    let vtbl = &*(*dialog).vtbl;
    let this = dialog as *mut c_void;

    // Restrict to filesystem folders so GetDisplayName(FILESYSPATH) succeeds.
    let mut opts: u32 = 0;
    if (vtbl.get_options)(this, &mut opts) < 0 {
        return None;
    }
    if (vtbl.set_options)(this, opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM) < 0 {
        return None;
    }

    let title_wide = wide(title);
    let _ = (vtbl.set_title)(this, title_wide.as_ptr());

    // Returns HRESULT_FROM_WIN32(ERROR_CANCELLED) when the user cancels.
    if (vtbl.show)(this, owner) < 0 {
        return None;
    }

    let mut item: *mut IShellItem = ptr::null_mut();
    if (vtbl.get_result)(this, &mut item as *mut _ as *mut *mut c_void) < 0 || item.is_null() {
        return None;
    }

    let path = shell_item_path(item);
    ((*(*item).vtbl).release)(item as *mut c_void);
    path
}

unsafe fn shell_item_path(item: *mut IShellItem) -> Option<PathBuf> {
    let vtbl = &*(*item).vtbl;
    let mut raw: PWSTR = ptr::null_mut();
    if (vtbl.get_display_name)(item as *mut c_void, SIGDN_FILESYSPATH, &mut raw) < 0
        || raw.is_null()
    {
        return None;
    }

    let len = (0..).take_while(|&i| *raw.add(i) != 0).count();
    let path = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(
        raw, len,
    )));
    CoTaskMemFree(raw as *const c_void);
    Some(path)
}
