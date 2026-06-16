use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

use crate::settings::claudie_home;
use crate::settings::storage::{read_json_or_default, save_pretty_json};

const MAX_DAYS: usize = 45;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DailyStatsDb {
    pub(crate) days: Vec<DailyStats>,
    #[serde(skip)]
    dirty: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DailyStats {
    pub(crate) date: String,
    pub(crate) prompts: u64,
    pub(crate) tool_uses: u64,
    pub(crate) write_tools: u64,
    pub(crate) bash_tools: u64,
    pub(crate) search_tools: u64,
    pub(crate) subagent_tools: u64,
    pub(crate) permission_requests: u64,
    pub(crate) choice_requests: u64,
    pub(crate) errors: u64,
    pub(crate) completed_focus: u64,
    pub(crate) token_delta: u64,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    /// Total tokens attributed per model id seen on this day.
    pub(crate) models: Vec<ModelTokens>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ModelTokens {
    pub(crate) model: String,
    pub(crate) tokens: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolStatsKind {
    Write,
    Bash,
    Search,
    Subagent,
    Other,
}

pub(crate) fn stats_path() -> PathBuf {
    claudie_home().join("daily_stats.json")
}

pub(crate) fn load_daily_stats() -> DailyStatsDb {
    let mut db: DailyStatsDb = read_json_or_default(&stats_path());
    db.normalize();
    db
}

pub(crate) fn save_daily_stats(db: &DailyStatsDb) -> Result<(), String> {
    save_pretty_json(&stats_path(), db)
}

impl DailyStatsDb {
    pub(crate) fn normalize(&mut self) {
        self.days.retain(|day| !day.date.trim().is_empty());
        self.days.sort_by(|a, b| a.date.cmp(&b.date));
        self.days.dedup_by(|a, b| {
            if a.date == b.date {
                b.merge(a);
                true
            } else {
                false
            }
        });
        if self.days.len() > MAX_DAYS {
            let remove = self.days.len() - MAX_DAYS;
            self.days.drain(0..remove);
        }
    }

    pub(crate) fn today(&self) -> DailyStats {
        let key = today_key();
        self.days
            .iter()
            .find(|day| day.date == key)
            .cloned()
            .unwrap_or_else(|| DailyStats {
                date: key,
                ..DailyStats::default()
            })
    }

    pub(crate) fn record(&mut self, update: impl FnOnce(&mut DailyStats)) {
        let key = today_key();
        let index = match self.days.iter().position(|day| day.date == key) {
            Some(index) => index,
            None => {
                self.days.push(DailyStats {
                    date: key,
                    ..DailyStats::default()
                });
                self.days.len() - 1
            }
        };
        update(&mut self.days[index]);
        self.normalize();
        self.dirty = true;
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn flush(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        save_daily_stats(self)?;
        self.dirty = false;
        Ok(())
    }
}

impl DailyStats {
    fn merge(&mut self, other: &DailyStats) {
        self.prompts = self.prompts.saturating_add(other.prompts);
        self.tool_uses = self.tool_uses.saturating_add(other.tool_uses);
        self.write_tools = self.write_tools.saturating_add(other.write_tools);
        self.bash_tools = self.bash_tools.saturating_add(other.bash_tools);
        self.search_tools = self.search_tools.saturating_add(other.search_tools);
        self.subagent_tools = self.subagent_tools.saturating_add(other.subagent_tools);
        self.permission_requests = self
            .permission_requests
            .saturating_add(other.permission_requests);
        self.choice_requests = self.choice_requests.saturating_add(other.choice_requests);
        self.errors = self.errors.saturating_add(other.errors);
        self.completed_focus = self.completed_focus.saturating_add(other.completed_focus);
        self.token_delta = self.token_delta.saturating_add(other.token_delta);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        for entry in &other.models {
            self.add_model_tokens(&entry.model, entry.tokens);
        }
    }

    /// Attribute `tokens` to `model` within this day, creating the bucket on
    /// first sight. Empty model ids are bucketed under "unknown".
    pub(crate) fn add_model_tokens(&mut self, model: &str, tokens: u64) {
        if tokens == 0 {
            return;
        }
        let name = match model.trim() {
            "" => "unknown",
            other => other,
        };
        if let Some(entry) = self.models.iter_mut().find(|entry| entry.model == name) {
            entry.tokens = entry.tokens.saturating_add(tokens);
        } else {
            self.models.push(ModelTokens {
                model: name.to_string(),
                tokens,
            });
        }
    }
}

/// Sum an arbitrary set of daily buckets into a single aggregate (used by the
/// Stats tab to total a calendar window).
pub(crate) fn sum_days<'a>(days: impl IntoIterator<Item = &'a DailyStats>) -> DailyStats {
    let mut total = DailyStats::default();
    for day in days {
        total.merge(day);
    }
    total
}

pub(crate) fn tool_stats_kind(tool_name: &str) -> ToolStatsKind {
    let normalized = tool_name.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "task" | "agent" => ToolStatsKind::Subagent,
        "bash" | "shell" => ToolStatsKind::Bash,
        "edit" | "multiedit" | "write" | "notebookedit" => ToolStatsKind::Write,
        "read" | "grep" | "glob" | "ls" | "webfetch" | "websearch" => ToolStatsKind::Search,
        _ if normalized.contains("edit")
            || normalized.contains("write")
            || normalized.contains("patch")
            || normalized.contains("replace") =>
        {
            ToolStatsKind::Write
        }
        _ if normalized.contains("bash")
            || normalized.contains("shell")
            || normalized.contains("terminal")
            || normalized.contains("command") =>
        {
            ToolStatsKind::Bash
        }
        _ if normalized.contains("read")
            || normalized.contains("grep")
            || normalized.contains("glob")
            || normalized.contains("search")
            || normalized.contains("find")
            || normalized.contains("lookup")
            || normalized.contains("fetch")
            || normalized.contains("list") =>
        {
            ToolStatsKind::Search
        }
        _ => ToolStatsKind::Other,
    }
}

fn today_key() -> String {
    let mut time = SYSTEMTIME::default();
    unsafe {
        GetLocalTime(&mut time);
    }
    format!("{:04}-{:02}-{:02}", time.wYear, time.wMonth, time.wDay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_tool_names_map_to_subagent_kind() {
        assert_eq!(tool_stats_kind("Task"), ToolStatsKind::Subagent);
        assert_eq!(tool_stats_kind("Agent"), ToolStatsKind::Subagent);
    }
}
