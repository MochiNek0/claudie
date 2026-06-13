use std::rc::Rc;

use slint::{ModelRc, VecModel};

use crate::app::stats::{DailyStats, DailyStatsDb, load_daily_stats, sum_days};
use crate::time_util::{date_key_minus_days, parse_date_key};
use crate::ui::slint_views::{SettingsWindow, StatHighlight, StatTrendBar};

use super::{SettingsController, shared};

const TREND_DAYS: usize = 14;

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
    let window = calendar_window(&db, &today.date, TREND_DAYS);
    let last7: Vec<&DailyStats> = window.iter().rev().take(7).collect();
    let recent = sum_days(last7.iter().copied());
    let active7 = last7.iter().filter(|day| day.prompts > 0).count();

    set_kpis(ui, &today, &recent);
    set_trend(ui, &window, &today.date, active7);
    set_highlights(ui, &recent, active7);
    set_recent_bars(ui, &recent);
    set_recent_token_bars(ui, &recent);
}

/// Build the last `days` calendar days ending today (oldest first), filling
/// idle days with zero so the trend reads as a continuous timeline.
fn calendar_window(db: &DailyStatsDb, today: &str, days: usize) -> Vec<DailyStats> {
    (0..days)
        .rev()
        .map(|offset| {
            let key =
                date_key_minus_days(today, offset as i64).unwrap_or_else(|| today.to_string());
            db.days
                .iter()
                .find(|day| day.date == key)
                .cloned()
                .unwrap_or(DailyStats {
                    date: key,
                    ..DailyStats::default()
                })
        })
        .collect()
}

fn set_kpis(ui: &SettingsWindow, today: &DailyStats, recent: &DailyStats) {
    ui.set_stats_kpi_prompts(shared(&today.prompts.to_string()));
    ui.set_stats_kpi_prompts_sub(shared(&format!("7d · {}", recent.prompts)));
    ui.set_stats_kpi_tokens(shared(&compact_number(today.token_delta)));
    ui.set_stats_kpi_tokens_sub(shared(&format!(
        "7d · {}",
        compact_number(recent.token_delta)
    )));
    ui.set_stats_kpi_cache(shared(&cache_hit_label(today)));
    ui.set_stats_kpi_cache_sub(shared(&format!("7d · {}", cache_hit_label(recent))));
    ui.set_stats_kpi_tools(shared(&today.tool_uses.to_string()));
    ui.set_stats_kpi_tools_sub(shared(&format!("7d · {}", recent.tool_uses)));
}

fn set_trend(ui: &SettingsWindow, window: &[DailyStats], today: &str, active7: usize) {
    let peak = window.iter().map(|day| day.prompts).max().unwrap_or(0);
    let bars: Vec<StatTrendBar> = window
        .iter()
        .map(|day| StatTrendBar {
            day_label: shared(&day_label(&day.date)),
            height: if peak == 0 {
                0.0
            } else {
                (day.prompts as f32 / peak as f32) * 100.0
            },
            today: day.date == today,
        })
        .collect();
    ui.set_stats_trend(ModelRc::from(Rc::new(VecModel::from(bars))));
    ui.set_stats_trend_caption(shared(&format!(
        "prompts/day · peak {} · {}/7 active",
        peak, active7
    )));
}

fn set_highlights(ui: &SettingsWindow, recent: &DailyStats, active7: usize) {
    let avg = if recent.prompts > 0 {
        compact_number(recent.token_delta / recent.prompts)
    } else {
        "—".to_string()
    };
    let top_tool = [
        ("Write", recent.write_tools),
        ("Bash", recent.bash_tools),
        ("Search", recent.search_tools),
        ("Agent", recent.subagent_tools),
    ]
    .into_iter()
    .filter(|(_, value)| *value > 0)
    .max_by_key(|(_, value)| *value)
    .map(|(name, value)| format!("{} · {}", name, compact_number(value)))
    .unwrap_or_else(|| "—".to_string());

    let items = [
        ("Active days", format!("{} / 7", active7)),
        ("Avg / prompt", avg),
        ("Top tool", top_tool),
        ("Focus done", recent.completed_focus.to_string()),
    ];
    let highlights: Vec<StatHighlight> = items
        .into_iter()
        .map(|(label, value)| StatHighlight {
            label: shared(label),
            value: shared(&value),
        })
        .collect();
    ui.set_stats_highlights(ModelRc::from(Rc::new(VecModel::from(highlights))));
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

/// Share of prompt context served from the cache, as `42%` or `—` when there
/// is no prompt-token traffic yet.
fn cache_hit_label(stats: &DailyStats) -> String {
    let prompt = stats
        .input_tokens
        .saturating_add(stats.cache_creation_tokens)
        .saturating_add(stats.cache_read_tokens);
    if prompt == 0 {
        return "—".to_string();
    }
    let pct = ((stats.cache_read_tokens as f64 / prompt as f64) * 100.0).round() as u32;
    format!("{}%", pct)
}

fn day_label(key: &str) -> String {
    parse_date_key(key)
        .map(|(_, _, day)| day.to_string())
        .unwrap_or_default()
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
