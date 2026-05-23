use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::path::Path;

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
    fs::write(path, format!("{text}\n")).map_err(|err| err.to_string())
}

pub(crate) fn json_without_bom(text: &str) -> &str {
    text.trim_start_matches('\u{feff}')
}
