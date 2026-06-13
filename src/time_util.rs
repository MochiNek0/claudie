//! Shared timestamp and usage-percentage parsing used by hook quota capture,
//! the official usage poller, and usage display formatting.

use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

/// `used_percentage`/`utilization` values are expected on a 0-100 scale.
pub(crate) fn usage_percent_value(value: &Value) -> Option<u8> {
    let raw = value.as_f64().or_else(|| {
        value
            .as_str()
            .and_then(|text| text.trim().parse::<f64>().ok())
    })?;
    if !raw.is_finite() {
        return None;
    }
    Some(raw.round().clamp(0.0, 100.0) as u8)
}

/// Accepts epoch seconds, epoch milliseconds, or an RFC3339 timestamp and
/// returns Unix milliseconds.
pub(crate) fn date_value_unix_ms(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return Some(epoch_number_to_ms(number));
    }
    date_text_unix_ms(value.as_str()?)
}

pub(crate) fn date_text_unix_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(number) = text.parse::<u64>() {
        return Some(epoch_number_to_ms(number));
    }
    parse_rfc3339_utc_ms(text)
}

fn epoch_number_to_ms(number: u64) -> u64 {
    if number > 1_000_000_000_000 {
        number
    } else {
        number.saturating_mul(1000)
    }
}

fn parse_rfc3339_utc_ms(text: &str) -> Option<u64> {
    let (date, time) = text.split_once('T')?;
    let (time, offset_minutes) = split_rfc3339_time_offset(time)?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i32>().ok()?;
    let month = date_parts.next()?.parse::<u32>().ok()?;
    let day = date_parts.next()?.parse::<u32>().ok()?;
    if date_parts.next().is_some() || !valid_date(year, month, day) {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<u32>().ok()?;
    let minute = time_parts.next()?.parse::<u32>().ok()?;
    let second_part = time_parts.next()?;
    if time_parts.next().is_some() || hour > 23 || minute > 59 {
        return None;
    }
    let (second_text, fraction_text) = second_part
        .split_once('.')
        .map(|(second, fraction)| (second, Some(fraction)))
        .unwrap_or((second_part, None));
    let second = second_text.parse::<u32>().ok()?;
    if second > 59 {
        return None;
    }
    let millis = fraction_text.map(parse_millis).unwrap_or(Some(0))?;

    let days = days_from_civil(year, month, day);
    if days < 0 {
        return None;
    }
    let day_ms = i128::from(days).saturating_mul(86_400_000);
    let time_ms = i128::from(hour)
        .saturating_mul(3_600_000)
        .saturating_add(i128::from(minute).saturating_mul(60_000))
        .saturating_add(i128::from(second).saturating_mul(1000))
        .saturating_add(i128::from(millis));
    let offset_ms = i128::from(offset_minutes).saturating_mul(60_000);
    u64::try_from(day_ms.saturating_add(time_ms).saturating_sub(offset_ms)).ok()
}

fn split_rfc3339_time_offset(time: &str) -> Option<(&str, i32)> {
    if let Some(time) = time.strip_suffix('Z') {
        return Some((time, 0));
    }
    let split_at = time.rfind(['+', '-'])?;
    let (time, offset) = time.split_at(split_at);
    Some((time, parse_timezone_offset_minutes(offset)?))
}

fn parse_timezone_offset_minutes(offset: &str) -> Option<i32> {
    let sign = match offset.as_bytes().first().copied()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let offset = &offset[1..];
    let (hours, minutes) = offset.split_once(':')?;
    let hours = hours.parse::<i32>().ok()?;
    let minutes = minutes.parse::<i32>().ok()?;
    if !(0..=23).contains(&hours) || !(0..=59).contains(&minutes) {
        return None;
    }
    Some(sign * (hours * 60 + minutes))
}

fn parse_millis(fraction: &str) -> Option<u32> {
    if fraction.is_empty() || !fraction.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let mut digits = fraction.chars().take(3).collect::<String>();
    while digits.len() < 3 {
        digits.push('0');
    }
    digits.parse::<u32>().ok()
}

fn valid_date(year: i32, month: u32, day: u32) -> bool {
    if year < 1970 || !(1..=12).contains(&month) {
        return false;
    }
    day >= 1 && day <= days_in_month(year, month)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::from(era * 146_097 + doe - 719_468)
}

/// Inverse of [`days_from_civil`]: days since 1970-01-01 back to a civil date.
fn civil_from_days(serial: i64) -> (i32, u32, u32) {
    let z = serial + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (
        (year + if month <= 2 { 1 } else { 0 }) as i32,
        month as u32,
        day,
    )
}

/// Parse a local `YYYY-MM-DD` date key into its (year, month, day) parts.
pub(crate) fn parse_date_key(key: &str) -> Option<(i32, u32, u32)> {
    let mut parts = key.trim().split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || !valid_date(year, month, day) {
        return None;
    }
    Some((year, month, day))
}

/// Returns the `YYYY-MM-DD` key for `days` days before `key`, handling month and
/// year boundaries. Returns `None` if `key` is not a valid date.
pub(crate) fn date_key_minus_days(key: &str, days: i64) -> Option<String> {
    let (year, month, day) = parse_date_key(key)?;
    let serial = days_from_civil(year, month, day) - days;
    let (y, m, d) = civil_from_days(serial);
    Some(format!("{:04}-{:02}-{:02}", y, m, d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_utc_iso_timestamp() {
        assert_eq!(
            date_text_unix_ms("2026-04-20T15:00:00.000Z"),
            Some(1_776_697_200_000)
        );
    }

    #[test]
    fn parses_offset_iso_timestamps() {
        assert_eq!(
            date_text_unix_ms("2026-04-20T15:00:00+00:00"),
            Some(1_776_697_200_000)
        );
        assert_eq!(
            date_text_unix_ms("2026-04-20T17:00:00+02:00"),
            Some(1_776_697_200_000)
        );
        assert_eq!(
            date_text_unix_ms("2026-04-20T10:00:00-05:00"),
            Some(1_776_697_200_000)
        );
    }

    #[test]
    fn rejects_invalid_timestamps() {
        assert_eq!(date_text_unix_ms(""), None);
        assert_eq!(date_text_unix_ms("2026-04-20T15:00:00"), None);
        assert_eq!(date_text_unix_ms("2026-13-01T00:00:00Z"), None);
        assert_eq!(date_text_unix_ms("soon"), None);
    }

    #[test]
    fn scales_epoch_seconds_to_milliseconds() {
        assert_eq!(
            date_value_unix_ms(&json!(1_776_697_200_u64)),
            Some(1_776_697_200_000)
        );
        assert_eq!(
            date_value_unix_ms(&json!(1_776_697_200_000_u64)),
            Some(1_776_697_200_000)
        );
        assert_eq!(date_value_unix_ms(&json!(-1)), None);
    }

    #[test]
    fn date_key_minus_days_crosses_boundaries() {
        assert_eq!(
            date_key_minus_days("2026-06-14", 13).as_deref(),
            Some("2026-06-01")
        );
        assert_eq!(
            date_key_minus_days("2026-03-01", 1).as_deref(),
            Some("2026-02-28")
        );
        assert_eq!(
            date_key_minus_days("2024-03-01", 1).as_deref(),
            Some("2024-02-29")
        );
        assert_eq!(
            date_key_minus_days("2026-01-01", 1).as_deref(),
            Some("2025-12-31")
        );
        assert_eq!(date_key_minus_days("nope", 1), None);
    }

    #[test]
    fn clamps_usage_percentages() {
        assert_eq!(usage_percent_value(&json!(42.4)), Some(42));
        assert_eq!(usage_percent_value(&json!("87.6")), Some(88));
        assert_eq!(usage_percent_value(&json!(250)), Some(100));
        assert_eq!(usage_percent_value(&json!("NaN")), None);
        assert_eq!(usage_percent_value(&json!("soon")), None);
    }
}
