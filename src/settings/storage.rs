use serde::{Serialize, de::DeserializeOwned};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

pub(crate) fn read_json<T>(path: &Path) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str(json_without_bom(&text)).map_err(|err| err.to_string())
}

pub(crate) fn read_json_or_default<T>(path: &Path) -> T
where
    T: DeserializeOwned + Default,
{
    read_json(path).unwrap_or_default()
}

pub(crate) fn save_pretty_json<T>(path: &Path, value: &T) -> Result<(), String>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    write_text_atomic(path, &format!("{text}\n"))
}

pub(crate) fn json_without_bom(text: &str) -> &str {
    text.trim_start_matches('\u{feff}')
}

pub(crate) fn write_text_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let temp_path = unique_temp_path(path)?;
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|err| err.to_string())?;
        file.write_all(text.as_bytes())
            .map_err(|err| err.to_string())?;
        file.flush().map_err(|err| err.to_string())?;
        file.sync_all().map_err(|err| err.to_string())
    })();
    if let Err(err) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    if !move_file_replace(&temp_path, path) {
        let err = std::io::Error::last_os_error().to_string();
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    Ok(())
}

fn unique_temp_path(path: &Path) -> Result<std::path::PathBuf, String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("claudie");
    let pid = std::process::id();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    for attempt in 0..100_u32 {
        let candidate = parent.join(format!(".{stem}.{pid}.{now}.{attempt}.tmp"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("failed to allocate a temporary file name".to_string())
}

fn move_file_replace(from: &Path, to: &Path) -> bool {
    let from = wide_path(from);
    let to = wide_path(to);
    unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        ) != 0
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
