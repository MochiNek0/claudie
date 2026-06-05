use crate::app::{OfficialUsageWindow, QuotaStats};
use crate::settings::OFFICIAL_LLM_PROFILE_ID;
use crate::time_util::{date_text_unix_ms, now_unix_ms};
use crate::util::shorten;

#[derive(Clone, Debug)]
pub(crate) struct UsageLine {
    pub(crate) value: String,
    pub(crate) bar: f32,
    pub(crate) percent: Option<u8>,
    pub(crate) reset: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ProviderUsageDisplay {
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) five_hour: UsageLine,
    pub(crate) seven_day: UsageLine,
    pub(crate) has_usage: bool,
}

pub(crate) fn provider_usage_display(
    profile_name: &str,
    profile_id: &str,
    active_profile_id: &str,
    quota: &QuotaStats,
) -> ProviderUsageDisplay {
    let title = if profile_name.trim().is_empty() {
        "Provider usage".to_string()
    } else {
        format!("{} usage", profile_name.trim())
    };
    let five_hour = usage_line(&quota.official_five_hour);
    let seven_day = usage_line(&quota.official_seven_day);
    let has_usage = five_hour.percent.is_some() || seven_day.percent.is_some();

    let summary = if has_usage {
        let mut summary = reset_pair_text(&five_hour, &seven_day);
        let plan = quota.official_plan.trim();
        if !plan.is_empty() {
            summary.push_str(&format!(" · {plan}"));
        }
        if let Some(updated) = quota.official_usage_updated_at_unix_ms.map(elapsed_text) {
            summary.push_str(&format!(" · updated {updated}"));
        }
        summary
    } else if !quota.official_usage_error.trim().is_empty() && profile_id == OFFICIAL_LLM_PROFILE_ID
    {
        shorten(quota.official_usage_error.trim(), 93)
    } else if profile_id != active_profile_id {
        "No cached usage for this provider yet.".to_string()
    } else {
        "This provider has not reported 5h/7d limits.".to_string()
    };

    ProviderUsageDisplay {
        title,
        summary,
        five_hour,
        seven_day,
        has_usage,
    }
}

fn reset_pair_text(five_hour: &UsageLine, seven_day: &UsageLine) -> String {
    let five = if five_hour.reset.is_empty() {
        "5h unknown".to_string()
    } else {
        format!("5h {}", five_hour.reset)
    };
    let seven = if seven_day.reset.is_empty() {
        "7d unknown".to_string()
    } else {
        format!("7d {}", seven_day.reset)
    };
    format!("Resets: {five}, {seven}")
}

fn usage_line(window: &OfficialUsageWindow) -> UsageLine {
    let value = window
        .used_percentage
        .map(|percent| format!("{percent}%"))
        .unwrap_or_else(|| "--".to_string());
    let bar = window.used_percentage.map(f32::from).unwrap_or(0.0);
    UsageLine {
        value,
        bar,
        percent: window.used_percentage,
        reset: reset_text(window),
    }
}

fn reset_text(window: &OfficialUsageWindow) -> String {
    if let Some(reset_at) = window.reset_at_unix_ms {
        let now = now_unix_ms();
        if reset_at > now {
            return format!("in {}", duration_text(reset_at - now));
        }
        return "due".to_string();
    }
    let label = window.reset_label.trim();
    if label.is_empty() {
        String::new()
    } else if let Some(reset_at) = date_text_unix_ms(label) {
        let now = now_unix_ms();
        if reset_at > now {
            format!("in {}", duration_text(reset_at - now))
        } else {
            "due".to_string()
        }
    } else {
        label.to_string()
    }
}

fn elapsed_text(timestamp: u64) -> String {
    let now = now_unix_ms();
    if timestamp >= now {
        return "just now".to_string();
    }
    format!("{} ago", duration_text(now - timestamp))
}

fn duration_text(ms: u64) -> String {
    let minutes = (ms / 60_000).max(1);
    if minutes >= 60 * 24 {
        let days = minutes / (60 * 24);
        let hours = (minutes % (60 * 24)) / 60;
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else if minutes >= 60 {
        let hours = minutes / 60;
        let mins = minutes % 60;
        if mins > 0 {
            format!("{hours}h {mins}m")
        } else {
            format!("{hours}h")
        }
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_reset_label_is_rendered_as_relative_time() {
        let window = OfficialUsageWindow {
            used_percentage: Some(42),
            reset_at_unix_ms: None,
            reset_label: "2099-06-05T12:30:00.000Z".to_string(),
        };

        let text = reset_text(&window);

        assert!(text.starts_with("in "));
        assert!(!text.contains('T'));
    }

    #[test]
    fn offset_iso_reset_label_is_rendered_as_relative_time() {
        let window = OfficialUsageWindow {
            used_percentage: Some(42),
            reset_at_unix_ms: None,
            reset_label: "2099-06-05T12:30:00+00:00".to_string(),
        };

        let text = reset_text(&window);

        assert!(text.starts_with("in "));
        assert!(!text.contains('T'));
    }

    #[test]
    fn summary_includes_resets_plan_and_freshness() {
        let quota = QuotaStats {
            official_plan: "Max".to_string(),
            official_five_hour: OfficialUsageWindow {
                used_percentage: Some(42),
                reset_at_unix_ms: None,
                reset_label: String::new(),
            },
            official_usage_updated_at_unix_ms: Some(1),
            ..QuotaStats::default()
        };

        let display = provider_usage_display("Official", "official", "official", &quota);

        assert!(display.has_usage);
        assert!(display.summary.starts_with("Resets: "));
        assert!(display.summary.contains("Max"));
        assert!(display.summary.contains("updated"));
    }

    #[test]
    fn rate_limits_alone_do_not_imply_official_usage() {
        let quota = QuotaStats {
            rate_limits: "{\"requests\":100}".to_string(),
            ..QuotaStats::default()
        };

        let display = provider_usage_display("Official", "official", "official", &quota);

        assert!(!display.has_usage);
        assert_eq!(
            display.summary,
            "This provider has not reported 5h/7d limits."
        );
    }
}
