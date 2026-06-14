//! Next-run computation for [`Schedule`].
//!
//! Every function answers one question: given a reference instant `after_ms`,
//! what is the next instant (epoch millis) strictly after it at which the
//! schedule fires? The scheduler treats a task as *due* when the next fire after
//! its last run (or creation) is `<= now`. This makes time-of-day schedules
//! (daily/weekly/cron) work the same way as intervals.
//!
//! Simple recurrences are computed directly against the local clock; cron
//! expressions delegate to the `croner` crate (native 5-field Unix cron).

use chrono::{Datelike, Duration, Local, TimeZone};
use croner::Cron;

use super::config::Schedule;

/// Next instant (epoch millis) strictly after `after_ms` at which `schedule`
/// fires. Returns `None` for a malformed cron expression or a zero interval.
pub fn next_run_after(schedule: &Schedule, after_ms: i64) -> Option<i64> {
    match schedule {
        Schedule::Interval { every_secs } => {
            if *every_secs == 0 {
                return None;
            }
            let step = (*every_secs as i64).saturating_mul(1000);
            Some(after_ms.saturating_add(step))
        }
        Schedule::Daily { hour, minute } => next_daily(*hour, *minute, after_ms),
        Schedule::Weekly {
            weekday,
            hour,
            minute,
        } => next_weekly(*weekday, *hour, *minute, after_ms),
        Schedule::Cron { expr } => next_cron(expr, after_ms),
    }
}

/// Local `DateTime` for an epoch-millis instant, if representable.
fn local_from_ms(ms: i64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

/// Resolve a naive local datetime to a concrete instant, picking the earliest
/// candidate across DST folds rather than failing on ambiguity.
fn resolve_local(naive: chrono::NaiveDateTime) -> Option<chrono::DateTime<Local>> {
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Some(dt),
        chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => None,
    }
}

fn next_daily(hour: u8, minute: u8, after_ms: i64) -> Option<i64> {
    let after = local_from_ms(after_ms)?;
    for add_days in 0..=1 {
        let date = after.date_naive() + Duration::days(add_days);
        let naive = date.and_hms_opt(hour as u32, minute as u32, 0)?;
        if let Some(dt) = resolve_local(naive) {
            if dt > after {
                return Some(dt.timestamp_millis());
            }
        }
    }
    None
}

fn next_weekly(weekday: u8, hour: u8, minute: u8, after_ms: i64) -> Option<i64> {
    let after = local_from_ms(after_ms)?;
    // Look ahead up to 7 days (inclusive) for the next matching weekday/time.
    for add_days in 0..=7 {
        let date = after.date_naive() + Duration::days(add_days);
        if date.weekday().num_days_from_sunday() != weekday as u32 {
            continue;
        }
        let naive = date.and_hms_opt(hour as u32, minute as u32, 0)?;
        if let Some(dt) = resolve_local(naive) {
            if dt > after {
                return Some(dt.timestamp_millis());
            }
        }
    }
    None
}

fn next_cron(expr: &str, after_ms: i64) -> Option<i64> {
    let cron = Cron::new(expr).parse().ok()?;
    let after = local_from_ms(after_ms)?;
    let next = cron.find_next_occurrence(&after, false).ok()?;
    Some(next.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn ms(s: i64) -> i64 {
        s * 1000
    }

    #[test]
    fn interval_is_step_after_reference() {
        let sched = Schedule::Interval { every_secs: 60 };
        let after = ms(1_000_000);
        assert_eq!(next_run_after(&sched, after), Some(after + ms(60)));
    }

    #[test]
    fn interval_zero_is_none() {
        let sched = Schedule::Interval { every_secs: 0 };
        assert_eq!(next_run_after(&sched, ms(1_000_000)), None);
    }

    #[test]
    fn cron_every_two_minutes_advances() {
        let sched = Schedule::Cron {
            expr: "*/2 * * * *".to_string(),
        };
        let after = ms(1_700_000_000);
        let next_ms = next_run_after(&sched, after).unwrap();
        assert!(next_ms > after);
        let next = local_from_ms(next_ms).unwrap();
        assert_eq!(next.minute() % 2, 0);
        assert_eq!(next.second(), 0);
    }

    #[test]
    fn cron_invalid_is_none() {
        let sched = Schedule::Cron {
            expr: "not a cron".to_string(),
        };
        assert_eq!(next_run_after(&sched, ms(1_700_000_000)), None);
    }

    #[test]
    fn daily_is_in_future_at_requested_time() {
        let sched = Schedule::Daily {
            hour: 9,
            minute: 30,
        };
        let after = ms(1_700_000_000);
        let next_ms = next_run_after(&sched, after).unwrap();
        assert!(next_ms > after);
        let next = local_from_ms(next_ms).unwrap();
        assert_eq!(next.hour(), 9);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn weekly_lands_on_requested_weekday() {
        let sched = Schedule::Weekly {
            weekday: 3, // Wednesday
            hour: 8,
            minute: 0,
        };
        let after = ms(1_700_000_000);
        let next_ms = next_run_after(&sched, after).unwrap();
        assert!(next_ms > after);
        let next = local_from_ms(next_ms).unwrap();
        assert_eq!(next.weekday().num_days_from_sunday(), 3);
        assert_eq!(next.hour(), 8);
        assert_eq!(next.minute(), 0);
    }
}
