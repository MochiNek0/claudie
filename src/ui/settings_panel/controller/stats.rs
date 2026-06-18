use std::collections::HashMap;
use std::rc::Rc;

use slint::{ModelRc, VecModel};

use crate::app::stats::{DailyStats, DailyStatsDb, load_daily_stats, sum_days};
use crate::time_util::{date_key_minus_days, parse_date_key};
use crate::ui::slint_views::{ModelPieSlice, SettingsWindow, StatHighlight, StatTrendBar};

use super::{SettingsController, shared};

const TREND_DAYS: usize = 14;
/// Days summed into the per-model token pie.
const MODEL_CHART_DAYS: usize = 7;
/// Models drawn as their own wedge; the rest collapse into an "Other" wedge.
const MODEL_PIE_SLICES: usize = 5;

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
    set_model_token_pie(ui, &window);
}

/// Per-model token pie: one wedge per model over the last `MODEL_CHART_DAYS`
/// days, sized by each model's share of the 7-day total, with low-volume
/// models folded into an "Other" wedge. Also feeds the legend and the
/// hover-detail bar chart (per-day usage with a peak/zero y-scale).
fn set_model_token_pie(ui: &SettingsWindow, window: &[DailyStats]) {
    let start = window.len().saturating_sub(MODEL_CHART_DAYS);
    let days = &window[start..];

    // Day-of-month labels for the hover detail's x-axis ticks (shared by all
    // wedges, since every model spans the same window).
    let day_labels: Vec<slint::SharedString> = days
        .iter()
        .map(|day| shared(&day_label(&day.date)))
        .collect();
    ui.set_stats_model_days(ModelRc::from(Rc::new(VecModel::from(day_labels))));

    // Rank models by their total tokens across the window, ignoring Claude
    // Code's pseudo-model ids (see `is_real_model`).
    let mut totals: HashMap<&str, u64> = HashMap::new();
    for day in days {
        for entry in &day.models {
            if !is_real_model(&entry.model) {
                continue;
            }
            *totals.entry(entry.model.as_str()).or_insert(0) += entry.tokens;
        }
    }
    let mut ranked: Vec<(&str, u64)> = totals.into_iter().filter(|(_, t)| *t > 0).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    if ranked.is_empty() {
        ui.set_stats_model_slices(ModelRc::from(Rc::new(VecModel::<ModelPieSlice>::default())));
        ui.set_stats_model_caption(shared(crate::i18n::strings().stats_no_model_data));
        return;
    }

    let named = &ranked[..ranked.len().min(MODEL_PIE_SLICES)];
    let has_other = ranked.len() > MODEL_PIE_SLICES;

    // Per-day series for each wedge (named models + an "Other" bucket).
    let mut series: Vec<(String, u64, Vec<u64>)> = Vec::new();
    for (model, total) in named {
        let daily = days
            .iter()
            .map(|day| model_tokens(day, model))
            .collect::<Vec<u64>>();
        series.push((short_model_name(model), *total, daily));
    }
    if has_other {
        let named_set: Vec<&str> = named.iter().map(|(m, _)| *m).collect();
        let daily = days
            .iter()
            .map(|day| {
                day.models
                    .iter()
                    .filter(|e| is_real_model(&e.model) && !named_set.contains(&e.model.as_str()))
                    .map(|e| e.tokens)
                    .sum()
            })
            .collect::<Vec<u64>>();
        let total: u64 = daily.iter().sum();
        series.push((
            crate::i18n::strings().stats_model_other.to_string(),
            total,
            daily,
        ));
    }

    let grand_total: u64 = series.iter().map(|(_, total, _)| *total).sum();

    // Cumulative angles around the circle; the last wedge snaps to 360° so
    // floating-point drift never leaves a sliver gap.
    let mut cursor = 0.0f64;
    let last = series.len() - 1;
    let slices: Vec<ModelPieSlice> = series
        .iter()
        .enumerate()
        .map(|(index, (name, total, daily))| {
            let frac = if grand_total == 0 {
                0.0
            } else {
                *total as f64 / grand_total as f64
            };
            let start_angle = cursor;
            let end_angle = if index == last {
                360.0
            } else {
                cursor + frac * 360.0
            };
            cursor = end_angle;
            let (pop_dx, pop_dy) = pop_offset((start_angle + end_angle) / 2.0);
            // Per-day heights normalized to this model's own peak day (so a
            // small model's daily shape is still legible), with a small floor
            // so non-zero days stay visible.
            let peak_day = daily.iter().copied().max().unwrap_or(0);
            let bars: Vec<f32> = daily
                .iter()
                .map(|value| {
                    if *value == 0 || peak_day == 0 {
                        0.0
                    } else {
                        (*value as f32 / peak_day as f32).max(0.04)
                    }
                })
                .collect();
            ModelPieSlice {
                name: shared(name),
                value: shared(&compact_number(*total)),
                percent: shared(&format!("{}%", (frac * 100.0).round() as u64)),
                commands: shared(&wedge_path(start_angle, end_angle)),
                bars: ModelRc::from(Rc::new(VecModel::from(bars))),
                peak: shared(&compact_number(peak_day)),
                color_index: index as i32,
                start_angle: start_angle as f32,
                end_angle: end_angle as f32,
                pop_dx,
                pop_dy,
            }
        })
        .collect();

    ui.set_stats_model_slices(ModelRc::from(Rc::new(VecModel::from(slices))));
    ui.set_stats_model_caption(shared(
        &crate::i18n::strings()
            .stats_model_caption_fmt
            .replacen("{}", &compact_number(grand_total), 1)
            .replacen("{}", &ranked.len().to_string(), 1),
    ));
}

fn model_tokens(day: &DailyStats, model: &str) -> u64 {
    day.models
        .iter()
        .find(|e| e.model == model)
        .map(|e| e.tokens)
        .unwrap_or(0)
}

/// A point on the pie circle. Angles are degrees with 0° at 12 o'clock and
/// increasing clockwise, matching the hover hit-test in `ModelTokenPie`.
fn point_on(deg: f64, radius: f64) -> (f64, f64) {
    let a = deg.to_radians();
    (100.0 + radius * a.sin(), 100.0 - radius * a.cos())
}

/// Filled wedge from `start_deg` to `end_deg` as an SVG path in a 200x200
/// viewbox (centre 100/100, radius 90). A full circle (single model) is drawn
/// as two semicircle arcs because one arc cannot span 360°.
fn wedge_path(start_deg: f64, end_deg: f64) -> String {
    const R: f64 = 90.0;
    if end_deg - start_deg >= 359.999 {
        return format!(
            "M 100.00 10.00 A {0:.2} {0:.2} 0 1 1 100.00 190.00 A {0:.2} {0:.2} 0 1 1 100.00 10.00 Z",
            R
        );
    }
    let (sx, sy) = point_on(start_deg, R);
    let (ex, ey) = point_on(end_deg, R);
    let large = if end_deg - start_deg > 180.0 { 1 } else { 0 };
    format!(
        "M 100.00 100.00 L {:.2} {:.2} A {:.2} {:.2} 0 {} 1 {:.2} {:.2} Z",
        sx, sy, R, R, large, ex, ey
    )
}

/// Offset (px) the hovered wedge shifts along its bisector to "pop" out.
fn pop_offset(mid_deg: f64) -> (f32, f32) {
    const POP: f64 = 7.0;
    let a = mid_deg.to_radians();
    ((POP * a.sin()) as f32, (-POP * a.cos()) as f32)
}

/// Real model ids only. Claude Code attributes some tokens to angle-bracket
/// pseudo-model ids (e.g. "<synthetic>") for messages it generates without a
/// real model call; those should not appear as their own pie wedge.
fn is_real_model(model: &str) -> bool {
    !model.trim_start().starts_with('<')
}

/// Drop the provider prefix and `[1m]` suffix so the legend stays readable.
fn short_model_name(model: &str) -> String {
    let trimmed = model.strip_suffix("[1m]").unwrap_or(model);
    trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(trimmed)
        .to_string()
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
    let sub = |value: &str| {
        crate::i18n::strings()
            .stats_kpi_sub_fmt
            .replace("{}", value)
    };
    ui.set_stats_kpi_prompts(shared(&today.prompts.to_string()));
    ui.set_stats_kpi_prompts_sub(shared(&sub(&recent.prompts.to_string())));
    ui.set_stats_kpi_tokens(shared(&compact_number(today.token_delta)));
    ui.set_stats_kpi_tokens_sub(shared(&sub(&compact_number(recent.token_delta))));
    ui.set_stats_kpi_cache(shared(&cache_hit_label(today)));
    ui.set_stats_kpi_cache_sub(shared(&sub(&cache_hit_label(recent))));
    ui.set_stats_kpi_tools(shared(&today.tool_uses.to_string()));
    ui.set_stats_kpi_tools_sub(shared(&sub(&recent.tool_uses.to_string())));
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
    ui.set_stats_trend_caption(shared(
        &crate::i18n::strings()
            .stats_trend_caption_fmt
            .replacen("{}", &peak.to_string(), 1)
            .replacen("{}", &active7.to_string(), 1),
    ));
}

fn set_highlights(ui: &SettingsWindow, recent: &DailyStats, active7: usize) {
    let avg = if recent.prompts > 0 {
        compact_number(recent.token_delta / recent.prompts)
    } else {
        "—".to_string()
    };
    let s = crate::i18n::strings();
    let top_tool = [
        (s.stat_write, recent.write_tools),
        (s.stat_bash, recent.bash_tools),
        (s.stat_search, recent.search_tools),
        (s.stat_agent, recent.subagent_tools),
    ]
    .into_iter()
    .filter(|(_, value)| *value > 0)
    .max_by_key(|(_, value)| *value)
    .map(|(name, value)| format!("{} · {}", name, compact_number(value)))
    .unwrap_or_else(|| "—".to_string());

    let items = [
        (s.highlight_active_days, format!("{} / 7", active7)),
        (s.highlight_avg_per_prompt, avg),
        (s.highlight_top_tool, top_tool),
        (s.highlight_focus_done, recent.completed_focus.to_string()),
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
