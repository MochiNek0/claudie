use crate::app::stats::{DailyStats, load_daily_stats};
use crate::ui::slint_views::SettingsWindow;

use super::{SettingsController, shared};

impl SettingsController {
    pub(super) fn refresh_stats_tab(&self) {
        let Some(ui) = self.ui() else {
            return;
        };
        set_stats_status(&ui);
    }
}

pub(super) fn set_stats_status(ui: &SettingsWindow) {
    let db = load_daily_stats();
    let today = db.today();
    let recent = db.recent_total(7);
    ui.set_stats_today_title(shared(&format!("Today {}", today.date)));
    ui.set_stats_today_summary(shared(&summary_line(&today)));
    ui.set_stats_recent_title(shared("Last 7 days"));
    ui.set_stats_recent_summary(shared(&summary_line(&recent)));
    set_today_bars(ui, &today);
    set_recent_bars(ui, &recent);
    set_today_token_bars(ui, &today);
    set_recent_token_bars(ui, &recent);
}

fn summary_line(stats: &DailyStats) -> String {
    format!(
        "{} prompts, {} tools, {} tokens, {} focus sessions, {} errors",
        stats.prompts,
        stats.tool_uses,
        compact_number(stats.token_delta),
        stats.completed_focus,
        stats.errors
    )
}

fn set_today_bars(ui: &SettingsWindow, stats: &DailyStats) {
    let values = bar_values(stats);
    ui.set_stats_today_write_value(shared(&values[0].to_string()));
    ui.set_stats_today_bash_value(shared(&values[1].to_string()));
    ui.set_stats_today_search_value(shared(&values[2].to_string()));
    ui.set_stats_today_subagent_value(shared(&values[3].to_string()));
    ui.set_stats_today_permission_value(shared(&values[4].to_string()));
    ui.set_stats_today_choice_value(shared(&values[5].to_string()));
    ui.set_stats_today_write_bar(bar_percent(values[0], &values));
    ui.set_stats_today_bash_bar(bar_percent(values[1], &values));
    ui.set_stats_today_search_bar(bar_percent(values[2], &values));
    ui.set_stats_today_subagent_bar(bar_percent(values[3], &values));
    ui.set_stats_today_permission_bar(bar_percent(values[4], &values));
    ui.set_stats_today_choice_bar(bar_percent(values[5], &values));
}

fn set_recent_bars(ui: &SettingsWindow, stats: &DailyStats) {
    let values = bar_values(stats);
    ui.set_stats_recent_write_value(shared(&values[0].to_string()));
    ui.set_stats_recent_bash_value(shared(&values[1].to_string()));
    ui.set_stats_recent_search_value(shared(&values[2].to_string()));
    ui.set_stats_recent_subagent_value(shared(&values[3].to_string()));
    ui.set_stats_recent_permission_value(shared(&values[4].to_string()));
    ui.set_stats_recent_choice_value(shared(&values[5].to_string()));
    ui.set_stats_recent_write_bar(bar_percent(values[0], &values));
    ui.set_stats_recent_bash_bar(bar_percent(values[1], &values));
    ui.set_stats_recent_search_bar(bar_percent(values[2], &values));
    ui.set_stats_recent_subagent_bar(bar_percent(values[3], &values));
    ui.set_stats_recent_permission_bar(bar_percent(values[4], &values));
    ui.set_stats_recent_choice_bar(bar_percent(values[5], &values));
}

fn bar_values(stats: &DailyStats) -> [u64; 6] {
    [
        stats.write_tools,
        stats.bash_tools,
        stats.search_tools,
        stats.subagent_tools,
        stats.permission_requests,
        stats.choice_requests,
    ]
}

fn set_today_token_bars(ui: &SettingsWindow, stats: &DailyStats) {
    let values = token_values(stats);
    ui.set_stats_today_input_value(shared(&compact_number(values[0])));
    ui.set_stats_today_output_value(shared(&compact_number(values[1])));
    ui.set_stats_today_cache_write_value(shared(&compact_number(values[2])));
    ui.set_stats_today_cache_read_value(shared(&compact_number(values[3])));
    ui.set_stats_today_input_bar(token_bar_percent(values[0], &values));
    ui.set_stats_today_output_bar(token_bar_percent(values[1], &values));
    ui.set_stats_today_cache_write_bar(token_bar_percent(values[2], &values));
    ui.set_stats_today_cache_read_bar(token_bar_percent(values[3], &values));
}

fn set_recent_token_bars(ui: &SettingsWindow, stats: &DailyStats) {
    let values = token_values(stats);
    ui.set_stats_recent_input_value(shared(&compact_number(values[0])));
    ui.set_stats_recent_output_value(shared(&compact_number(values[1])));
    ui.set_stats_recent_cache_write_value(shared(&compact_number(values[2])));
    ui.set_stats_recent_cache_read_value(shared(&compact_number(values[3])));
    ui.set_stats_recent_input_bar(token_bar_percent(values[0], &values));
    ui.set_stats_recent_output_bar(token_bar_percent(values[1], &values));
    ui.set_stats_recent_cache_write_bar(token_bar_percent(values[2], &values));
    ui.set_stats_recent_cache_read_bar(token_bar_percent(values[3], &values));
}

fn token_values(stats: &DailyStats) -> [u64; 4] {
    [
        stats.input_tokens,
        stats.output_tokens,
        stats.cache_creation_tokens,
        stats.cache_read_tokens,
    ]
}

fn bar_percent(value: u64, values: &[u64; 6]) -> f32 {
    let max_value = values.iter().copied().max().unwrap_or(0);
    if max_value == 0 || value == 0 {
        return 0.0;
    }
    ((value as f32 / max_value as f32) * 100.0).clamp(8.0, 100.0)
}

fn token_bar_percent(value: u64, values: &[u64; 4]) -> f32 {
    let max_value = values.iter().copied().max().unwrap_or(0);
    if max_value == 0 || value == 0 {
        return 0.0;
    }
    ((value as f32 / max_value as f32) * 100.0).clamp(8.0, 100.0)
}

fn compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 10_000 {
        format!("{}k", value / 1_000)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}
