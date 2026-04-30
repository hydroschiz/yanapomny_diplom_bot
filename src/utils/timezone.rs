use chrono::{DateTime, FixedOffset, Offset, Utc};
use chrono_tz::Tz;

use crate::api::db::User;

pub fn user_has_timezone(user: &User) -> bool {
    !user.time_zone.is_empty() || (!user.utc.is_empty() && !user.utc.eq_ignore_ascii_case("nil"))
}

pub fn parse_utc_offset_seconds(raw: &str) -> Option<i32> {
    let normalized = raw.trim().to_uppercase();
    let normalized = normalized
        .strip_prefix("UTC")
        .or_else(|| normalized.strip_prefix("GMT"))
        .unwrap_or(&normalized)
        .trim();

    if normalized.is_empty() || normalized == "NIL" {
        return Some(0);
    }

    let (sign, rest) = if let Some(value) = normalized.strip_prefix('-') {
        (-1, value)
    } else if let Some(value) = normalized.strip_prefix('+') {
        (1, value)
    } else {
        (1, normalized)
    };

    let parts: Vec<&str> = rest.split([':', '.']).collect();
    let hours: i32 = parts.first()?.trim().parse().ok()?;
    let minutes: i32 = parts
        .get(1)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);

    if hours.abs() > 14 || minutes >= 60 {
        return None;
    }

    Some(sign * (hours * 3600 + minutes * 60))
}

pub fn format_offset_seconds(offset_seconds: i32) -> String {
    let sign = if offset_seconds >= 0 { '+' } else { '-' };
    let absolute = offset_seconds.abs();
    let hours = absolute / 3600;
    let minutes = (absolute % 3600) / 60;

    format!("{sign}{hours:02}:{minutes:02}")
}

pub fn user_offset_seconds_at(user: &User, at: DateTime<Utc>) -> i32 {
    if !user.time_zone.is_empty() {
        if let Ok(tz) = user.time_zone.parse::<Tz>() {
            return at.with_timezone(&tz).offset().fix().local_minus_utc();
        }
    }

    if !user.utc.is_empty() && !user.utc.eq_ignore_ascii_case("nil") {
        return parse_utc_offset_seconds(&user.utc).unwrap_or(0);
    }

    0
}

pub fn user_fixed_offset_at(user: &User, at: DateTime<Utc>) -> FixedOffset {
    FixedOffset::east_opt(user_offset_seconds_at(user, at))
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC offset is valid"))
}

pub fn user_local_time(user: &User, at: DateTime<Utc>) -> DateTime<FixedOffset> {
    at.with_timezone(&user_fixed_offset_at(user, at))
}

pub fn user_offset_string_at(user: &User, at: DateTime<Utc>) -> String {
    format_offset_seconds(user_offset_seconds_at(user, at))
}

pub fn user_datetime_string(user: &User, at: DateTime<Utc>) -> String {
    user_local_time(user, at)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_offset_with_minutes() {
        assert_eq!(parse_utc_offset_seconds("+05:30"), Some(19_800));
        assert_eq!(parse_utc_offset_seconds("UTC-3"), Some(-10_800));
        assert_eq!(parse_utc_offset_seconds("nil"), Some(0));
    }

    #[test]
    fn formats_offset_with_minutes() {
        assert_eq!(format_offset_seconds(19_800), "+05:30");
        assert_eq!(format_offset_seconds(-10_800), "-03:00");
    }
}
