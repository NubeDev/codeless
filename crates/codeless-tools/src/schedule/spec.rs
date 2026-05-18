//! `Schedule` and friends — pure timing arithmetic, no I/O.

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    fn to_chrono(self) -> chrono::Weekday {
        match self {
            Weekday::Mon => chrono::Weekday::Mon,
            Weekday::Tue => chrono::Weekday::Tue,
            Weekday::Wed => chrono::Weekday::Wed,
            Weekday::Thu => chrono::Weekday::Thu,
            Weekday::Fri => chrono::Weekday::Fri,
            Weekday::Sat => chrono::Weekday::Sat,
            Weekday::Sun => chrono::Weekday::Sun,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl TimeOfDay {
    pub fn new(hour: u8, minute: u8) -> Result<Self, ScheduleError> {
        if hour > 23 || minute > 59 {
            return Err(ScheduleError::InvalidTime { hour, minute });
        }
        Ok(Self { hour, minute })
    }
}

/// Timezone the weekly grid is interpreted in.
///
/// `Local` is the host machine's local time. `Utc` is wall-clock UTC.
/// Per-tz IANA names are deliberately out of scope for the first cut
/// — adding `chrono-tz` later expands this enum without changing the
/// `Schedule` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleTz {
    #[default]
    Local,
    Utc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    OneShot {
        /// Fire exactly once at this instant.
        at: DateTime<Utc>,
    },
    Weekly {
        days: Vec<Weekday>,
        times: Vec<TimeOfDay>,
        #[serde(default)]
        tz: ScheduleTz,
    },
}

impl Schedule {
    /// Return the next fire instant strictly after `now`, or `None`
    /// if the schedule has no future firings (a one-shot in the past).
    pub fn next_fire_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Schedule::OneShot { at } => (*at > now).then_some(*at),
            Schedule::Weekly { days, times, tz } => {
                if days.is_empty() || times.is_empty() {
                    return None;
                }
                let mut sorted_times = times.clone();
                sorted_times.sort();
                match tz {
                    ScheduleTz::Local => {
                        next_weekly(now.with_timezone(&Local), days, &sorted_times)
                            .map(|dt| dt.with_timezone(&Utc))
                    }
                    ScheduleTz::Utc => next_weekly(now, days, &sorted_times),
                }
            }
        }
    }
}

fn next_weekly<Tz>(now: DateTime<Tz>, days: &[Weekday], times: &[TimeOfDay]) -> Option<DateTime<Tz>>
where
    Tz: TimeZone,
{
    // Walk up to 8 days forward. 7 covers any weekly slot; the 8th
    // handles the case where every slot today has already passed and
    // the next match is exactly one week from now.
    for offset in 0..=8 {
        let candidate_date = now.date_naive() + Duration::days(offset);
        let wd_match = days
            .iter()
            .any(|d| d.to_chrono() == candidate_date.weekday());
        if !wd_match {
            continue;
        }
        for t in times {
            let naive_time = NaiveTime::from_hms_opt(t.hour as u32, t.minute as u32, 0)?;
            let candidate_naive = candidate_date.and_time(naive_time);
            let candidate = match now.timezone().from_local_datetime(&candidate_naive) {
                chrono::LocalResult::Single(dt) => dt,
                chrono::LocalResult::Ambiguous(_, latest) => latest,
                chrono::LocalResult::None => continue,
            };
            if candidate > now {
                return Some(candidate);
            }
        }
    }
    None
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScheduleError {
    #[error("invalid time of day: {hour}:{minute:02}")]
    InvalidTime { hour: u8, minute: u8 },
}

impl Weekday {
    /// Lowercase three-letter name → `Weekday`. Accepts the seven
    /// forms produced by the JSON serde representation.
    pub fn parse(s: &str) -> Option<Weekday> {
        Some(match s.to_ascii_lowercase().as_str() {
            "mon" | "monday" => Weekday::Mon,
            "tue" | "tues" | "tuesday" => Weekday::Tue,
            "wed" | "wednesday" => Weekday::Wed,
            "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
            "fri" | "friday" => Weekday::Fri,
            "sat" | "saturday" => Weekday::Sat,
            "sun" | "sunday" => Weekday::Sun,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    #[test]
    fn one_shot_past_returns_none() {
        let s = Schedule::OneShot {
            at: utc(2026, 1, 1, 0, 0),
        };
        assert!(s.next_fire_after(utc(2026, 1, 2, 0, 0)).is_none());
    }

    #[test]
    fn one_shot_future_returns_at() {
        let at = utc(2026, 6, 1, 12, 0);
        let s = Schedule::OneShot { at };
        assert_eq!(s.next_fire_after(utc(2026, 1, 1, 0, 0)), Some(at));
    }

    #[test]
    fn weekly_mon_wed_three_times_utc() {
        // 2026-05-18 is a Monday.
        let s = Schedule::Weekly {
            days: vec![Weekday::Mon, Weekday::Wed],
            times: vec![
                TimeOfDay::new(8, 0).unwrap(),
                TimeOfDay::new(11, 0).unwrap(),
                TimeOfDay::new(17, 0).unwrap(),
            ],
            tz: ScheduleTz::Utc,
        };

        // Sunday 07:00 → Monday 08:00.
        let now = utc(2026, 5, 17, 7, 0);
        assert_eq!(s.next_fire_after(now), Some(utc(2026, 5, 18, 8, 0)));

        // Monday 09:00 → Monday 11:00.
        let now = utc(2026, 5, 18, 9, 0);
        assert_eq!(s.next_fire_after(now), Some(utc(2026, 5, 18, 11, 0)));

        // Monday 17:00 (exactly) → must be strictly after, so Monday 17:00 the next
        // matching slot — that's the 17:00 on this Monday tied with now, so we
        // expect Wednesday 08:00 because we use strict inequality.
        let now = utc(2026, 5, 18, 17, 0);
        assert_eq!(s.next_fire_after(now), Some(utc(2026, 5, 20, 8, 0)));

        // Monday 17:30 → Wednesday 08:00.
        let now = utc(2026, 5, 18, 17, 30);
        assert_eq!(s.next_fire_after(now), Some(utc(2026, 5, 20, 8, 0)));

        // Wednesday 18:00 → next Monday 08:00.
        let now = utc(2026, 5, 20, 18, 0);
        assert_eq!(s.next_fire_after(now), Some(utc(2026, 5, 25, 8, 0)));
    }

    #[test]
    fn empty_days_or_times_yields_none() {
        let s = Schedule::Weekly {
            days: vec![],
            times: vec![TimeOfDay::new(8, 0).unwrap()],
            tz: ScheduleTz::Utc,
        };
        assert!(s.next_fire_after(utc(2026, 1, 1, 0, 0)).is_none());

        let s = Schedule::Weekly {
            days: vec![Weekday::Mon],
            times: vec![],
            tz: ScheduleTz::Utc,
        };
        assert!(s.next_fire_after(utc(2026, 1, 1, 0, 0)).is_none());
    }

    #[test]
    fn invalid_time_rejected() {
        assert!(matches!(
            TimeOfDay::new(24, 0),
            Err(ScheduleError::InvalidTime { .. })
        ));
        assert!(matches!(
            TimeOfDay::new(0, 60),
            Err(ScheduleError::InvalidTime { .. })
        ));
    }

    #[test]
    fn weekday_parse_accepts_long_and_short() {
        assert_eq!(Weekday::parse("Mon"), Some(Weekday::Mon));
        assert_eq!(Weekday::parse("monday"), Some(Weekday::Mon));
        assert_eq!(Weekday::parse("THURSDAY"), Some(Weekday::Thu));
        assert_eq!(Weekday::parse("nope"), None);
    }
}
