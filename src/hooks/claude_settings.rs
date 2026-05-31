use serde_json::{Map, Value, json};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::app::AppState;
use crate::settings::storage::write_text_atomic;

pub(crate) fn settings_snippet(port: u16) -> String {
    serde_json::to_string_pretty(&hooks_value(port)).expect("valid settings")
}

fn hook_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/hook")
}

fn hooks_value(port: u16) -> Value {
    let mut hooks = Map::new();
    for event in hook_events() {
        hooks.insert(
            event.to_string(),
            Value::Array(vec![json!({
                "matcher": "",
                "hooks": [{
                    "type": "http",
                    "url": hook_url(port),
                    "timeout": 600
                }]
            })]),
        );
    }
    json!({ "hooks": hooks })
}

fn hook_events() -> &'static [&'static str] {
    &[
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PostToolBatch",
        "PermissionRequest",
        "PermissionDenied",
        "Notification",
        "Elicitation",
        "SubagentStart",
        "SubagentStop",
        "TaskCreated",
        "TaskCompleted",
        "PreCompact",
        "PostCompact",
        "WorktreeCreate",
        "Stop",
        "StopFailure",
        "SessionEnd",
    ]
}

pub(crate) fn install_claude_hooks(port: u16) -> Result<PathBuf, String> {
    let home = env::var_os("USERPROFILE").ok_or_else(|| "USERPROFILE is not set".to_string())?;
    let claude_dir = PathBuf::from(home).join(".claude");
    fs::create_dir_all(&claude_dir).map_err(|err| err.to_string())?;
    let settings_path = claude_dir.join("settings.json");

    let mut settings: Value = if settings_path.exists() {
        let text = fs::read_to_string(&settings_path).map_err(|err| err.to_string())?;
        serde_json::from_str(&text)
            .map_err(|err| format!("settings.json is not valid JSON: {err}"))?
    } else {
        json!({})
    };

    merge_hooks(&mut settings, port)?;

    let backup_path = settings_path.with_extension("json.claudie.bak");
    if settings_path.exists() && !backup_path.exists() {
        fs::copy(&settings_path, &backup_path).map_err(|err| err.to_string())?;
    }

    write_text_atomic(
        &settings_path,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&settings).map_err(|err| err.to_string())?
        ),
    )?;

    Ok(settings_path)
}

pub(crate) fn ensure_claude_hooks(_state: Arc<Mutex<AppState>>, port: u16) -> Result<(), String> {
    install_claude_hooks(port)?;
    Ok(())
}

pub(crate) fn uninstall_claude_hooks() -> Result<Option<PathBuf>, String> {
    let home = env::var_os("USERPROFILE").ok_or_else(|| "USERPROFILE is not set".to_string())?;
    let settings_path = PathBuf::from(home).join(".claude").join("settings.json");
    if !settings_path.exists() {
        return Ok(None);
    }

    let text = fs::read_to_string(&settings_path).map_err(|err| err.to_string())?;
    let mut settings: Value = serde_json::from_str(&text)
        .map_err(|err| format!("settings.json is not valid JSON: {err}"))?;
    if !remove_claudie_hooks(&mut settings)? {
        return Ok(Some(settings_path));
    }

    write_text_atomic(
        &settings_path,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&settings).map_err(|err| err.to_string())?
        ),
    )?;

    Ok(Some(settings_path))
}

fn merge_hooks(settings: &mut Value, port: u16) -> Result<(), String> {
    if !settings.is_object() {
        *settings = json!({});
    }

    remove_claudie_hooks(settings)?;

    let root = settings
        .as_object_mut()
        .ok_or_else(|| "settings root is not an object".to_string())?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }

    let url = hook_url(port);
    let hooks = hooks.as_object_mut().expect("hooks object");

    for event in hook_events() {
        let entries = hooks
            .entry((*event).to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !entries.is_array() {
            *entries = Value::Array(Vec::new());
        }
        let entries = entries.as_array_mut().expect("entries array");
        if event_has_url(entries, &url) {
            continue;
        }
        entries.push(json!({
            "matcher": "",
            "hooks": [{
                "type": "http",
                "url": url,
                "timeout": 600
            }]
        }));
    }

    Ok(())
}

fn event_has_url(entries: &[Value], url: &str) -> bool {
    entries.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| {
                hooks
                    .iter()
                    .any(|hook| hook.get("url").and_then(Value::as_str) == Some(url))
            })
    })
}

fn remove_claudie_hooks(settings: &mut Value) -> Result<bool, String> {
    let Some(root) = settings.as_object_mut() else {
        return Ok(false);
    };
    let Some(hooks_value) = root.get_mut("hooks") else {
        return Ok(false);
    };
    let Some(hooks) = hooks_value.as_object_mut() else {
        return Ok(false);
    };

    let mut changed = false;
    let event_names: Vec<String> = hooks.keys().cloned().collect();
    for event_name in event_names {
        let remove_event =
            if let Some(entries) = hooks.get_mut(&event_name).and_then(Value::as_array_mut) {
                let mut next_entries = Vec::with_capacity(entries.len());
                let mut event_changed = false;
                for mut entry in std::mem::take(entries) {
                    let mut removed_from_entry = false;
                    if let Some(entry_obj) = entry.as_object_mut() {
                        if let Some(hook_items) =
                            entry_obj.get_mut("hooks").and_then(Value::as_array_mut)
                        {
                            let before = hook_items.len();
                            hook_items.retain(|hook| !is_claudie_hook(hook));
                            removed_from_entry = hook_items.len() != before;
                        }
                    }

                    if removed_from_entry {
                        changed = true;
                        event_changed = true;
                        let is_empty = entry
                            .get("hooks")
                            .and_then(Value::as_array)
                            .is_some_and(Vec::is_empty);
                        if is_empty {
                            continue;
                        }
                    }
                    next_entries.push(entry);
                }
                let is_empty = event_changed && next_entries.is_empty();
                *entries = next_entries;
                is_empty
            } else {
                false
            };

        if remove_event {
            hooks.remove(&event_name);
            changed = true;
        }
    }

    if changed && hooks.is_empty() {
        root.remove("hooks");
    }

    Ok(changed)
}

fn is_claudie_hook(hook: &Value) -> bool {
    hook.get("url")
        .and_then(Value::as_str)
        .is_some_and(is_claudie_hook_url)
}

fn is_claudie_hook_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let Some((host_port, path)) = rest.split_once('/') else {
        return false;
    };
    if path != "hook" {
        return false;
    }

    let Some((host, port)) = host_port.rsplit_once(':') else {
        return false;
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]") && port.parse::<u16>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_hooks_is_idempotent_for_same_port() {
        let mut settings = json!({});
        merge_hooks(&mut settings, 17387).unwrap();
        merge_hooks(&mut settings, 17387).unwrap();

        let hooks = settings.get("hooks").and_then(Value::as_object).unwrap();
        for event in hook_events() {
            let entries = hooks.get(*event).and_then(Value::as_array).unwrap();
            assert_eq!(entries.len(), 1);
        }
    }

    #[test]
    fn merge_hooks_replaces_stale_claudie_ports() {
        let mut settings = json!({
            "hooks": {
                "PostToolUse": [
                    {
                        "matcher": "",
                        "hooks": [
                            { "type": "http", "url": "http://127.0.0.1:17387/hook" }
                        ]
                    }
                ]
            }
        });

        merge_hooks(&mut settings, 17388).unwrap();

        let hooks = settings.get("hooks").and_then(Value::as_object).unwrap();
        for event in hook_events() {
            let entries = hooks.get(*event).and_then(Value::as_array).unwrap();
            assert_eq!(entries.len(), 1);
            let url = entries[0]["hooks"][0]["url"].as_str();
            assert_eq!(url, Some("http://127.0.0.1:17388/hook"));
        }
    }

    #[test]
    fn remove_claudie_hooks_preserves_unrelated_hooks() {
        let mut settings = json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "",
                        "hooks": [
                            { "type": "http", "url": "http://127.0.0.1:17387/hook" },
                            { "type": "command", "command": "echo keep" }
                        ]
                    },
                    {
                        "matcher": "",
                        "hooks": [
                            { "type": "http", "url": "http://example.test/hook" }
                        ]
                    }
                ],
                "Stop": [
                    {
                        "matcher": "",
                        "hooks": [
                            { "type": "http", "url": "http://localhost:17387/hook" }
                        ]
                    }
                ]
            },
            "env": {
                "KEEP": "yes"
            }
        });

        assert!(remove_claudie_hooks(&mut settings).unwrap());
        let hooks = settings.get("hooks").and_then(Value::as_object).unwrap();
        let pre_tool = hooks.get("PreToolUse").and_then(Value::as_array).unwrap();
        assert_eq!(pre_tool.len(), 2);
        assert_eq!(
            pre_tool[0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert!(hooks.get("Stop").is_none());
        assert_eq!(settings["env"]["KEEP"].as_str(), Some("yes"));
    }

    #[test]
    fn remove_claudie_hooks_leaves_unrelated_empty_hooks_alone() {
        let mut settings = json!({
            "hooks": {
                "PreToolUse": []
            }
        });

        assert!(!remove_claudie_hooks(&mut settings).unwrap());
        assert!(settings.get("hooks").is_some());
        assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 0);
    }
}
