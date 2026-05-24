use std::{fmt, str::FromStr};

use chrono::{FixedOffset, NaiveTime};

use crate::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcOffset {
    seconds: i32,
}

impl UtcOffset {
    pub const UTC: Self = Self { seconds: 0 };
    pub const MAX_SECONDS: i32 = 14 * 60 * 60;

    pub fn from_seconds(seconds: i32) -> Result<Self, DomainError> {
        if seconds.abs() > Self::MAX_SECONDS || seconds % 60 != 0 {
            return Err(DomainError::InvalidUtcOffset {
                input: seconds.to_string(),
            });
        }
        Ok(Self { seconds })
    }

    pub const fn seconds(self) -> i32 {
        self.seconds
    }

    pub fn fixed_offset(self) -> FixedOffset {
        FixedOffset::east_opt(self.seconds).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap())
    }
}

impl Default for UtcOffset {
    fn default() -> Self {
        Self::UTC
    }
}

impl FromStr for UtcOffset {
    type Err = DomainError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let normalized = input.trim().to_uppercase();
        let rest = normalized
            .strip_prefix("UTC")
            .or_else(|| normalized.strip_prefix("GMT"))
            .unwrap_or(&normalized)
            .trim();

        if rest.is_empty() || rest == "Z" {
            return Ok(Self::UTC);
        }

        let (sign, rest) = if let Some(stripped) = rest.strip_prefix('+') {
            (1, stripped)
        } else if let Some(stripped) = rest.strip_prefix('-') {
            (-1, stripped)
        } else {
            (1, rest)
        };

        let parts: Vec<&str> = rest.split([':', '.']).collect();
        let hours: i32 = parts
            .first()
            .and_then(|value| value.trim().parse().ok())
            .ok_or_else(|| DomainError::InvalidUtcOffset {
                input: input.to_string(),
            })?;
        let minutes: i32 = parts
            .get(1)
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0);

        if hours > 14 || minutes >= 60 || (hours == 14 && minutes != 0) {
            return Err(DomainError::InvalidUtcOffset {
                input: input.to_string(),
            });
        }

        Self::from_seconds(sign * (hours * 3600 + minutes * 60)).map_err(|_| {
            DomainError::InvalidUtcOffset {
                input: input.to_string(),
            }
        })
    }
}

impl fmt::Display for UtcOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.seconds < 0 { '-' } else { '+' };
        let total = self.seconds.abs();
        let hours = total / 3600;
        let minutes = (total % 3600) / 60;
        write!(f, "{}{:02}:{:02}", sign, hours, minutes)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum TimeZone {
    #[default]
    Utc,
    Fixed(UtcOffset),
    Iana(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimePreferences {
    pub morning: NaiveTime,
    pub afternoon: NaiveTime,
    pub evening: NaiveTime,
    pub utc_offset: UtcOffset,
    pub time_zone: TimeZone,
}

impl TimePreferences {
    pub fn new(
        morning: NaiveTime,
        afternoon: NaiveTime,
        evening: NaiveTime,
        utc_offset: UtcOffset,
    ) -> Self {
        Self {
            morning,
            afternoon,
            evening,
            utc_offset,
            time_zone: TimeZone::Fixed(utc_offset),
        }
    }

    pub fn from_fixed_offset_strings(
        morning: &str,
        afternoon: &str,
        evening: &str,
        utc_offset: &str,
    ) -> Result<Self, DomainError> {
        let offset = if utc_offset.trim().is_empty() || utc_offset.eq_ignore_ascii_case("nil") {
            UtcOffset::UTC
        } else {
            utc_offset.parse()?
        };

        Ok(Self::new(
            parse_time(morning)?,
            parse_time(afternoon)?,
            parse_time(evening)?,
            offset,
        ))
    }

    pub fn fixed_offset(&self) -> FixedOffset {
        self.utc_offset.fixed_offset()
    }
}

impl Default for TimePreferences {
    fn default() -> Self {
        let mut preferences = Self::new(
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(19, 0, 0).unwrap(),
            UtcOffset::UTC,
        );
        preferences.time_zone = TimeZone::Utc;
        preferences
    }
}

fn parse_time(input: &str) -> Result<NaiveTime, DomainError> {
    let parts: Vec<&str> = input.trim().split(':').collect();
    if parts.len() < 2 {
        return Err(DomainError::InvalidTime {
            input: input.to_string(),
        });
    }

    let hour: u32 = parts[0].parse().map_err(|_| DomainError::InvalidTime {
        input: input.to_string(),
    })?;
    let minute: u32 = parts[1].parse().map_err(|_| DomainError::InvalidTime {
        input: input.to_string(),
    })?;

    NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(|| DomainError::InvalidTime {
        input: input.to_string(),
    })
}
