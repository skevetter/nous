use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use crate::error::NousError;

pub trait Clock: Send + Sync + 'static {
    fn now_utc(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> i64 {
        chrono::Utc::now().timestamp()
    }
}

#[derive(Clone)]
pub struct MockClock {
    now: Arc<std::sync::atomic::AtomicI64>,
}

impl MockClock {
    pub fn new(ts: i64) -> Self {
        Self {
            now: Arc::new(std::sync::atomic::AtomicI64::new(ts)),
        }
    }

    pub fn set(&self, ts: i64) {
        self.now.store(ts, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn advance(&self, secs: i64) {
        self.now
            .fetch_add(secs, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for MockClock {
    fn now_utc(&self) -> i64 {
        self.now.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CronExpr {
    pub minutes: BTreeSet<u32>,
    pub hours: BTreeSet<u32>,
    pub days_of_month: BTreeSet<u32>,
    pub months: BTreeSet<u32>,
    pub days_of_week: BTreeSet<u32>,
    pub dom_is_wildcard: bool,
    pub dow_is_wildcard: bool,
}

impl CronExpr {
    pub fn parse(expr: &str) -> Result<Self, NousError> {
        let expr = expr.trim();

        if let Some(expanded) = Self::expand_shorthand(expr) {
            return Self::parse(&expanded);
        }

        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(NousError::Validation(format!(
                "cron expression must have 5 fields, got {}",
                fields.len()
            )));
        }

        let minutes = Self::parse_field(fields[0], 0, 59)?;
        let hours = Self::parse_field(fields[1], 0, 23)?;
        let (days_of_month, dom_is_wildcard) = Self::parse_field_with_wildcard(fields[2], 1, 31)?;
        let months = Self::parse_field(fields[3], 1, 12)?;
        let (days_of_week, dow_is_wildcard) = Self::parse_field_with_wildcard(fields[4], 0, 6)?;

        Ok(Self {
            minutes,
            hours,
            days_of_month,
            months,
            days_of_week,
            dom_is_wildcard,
            dow_is_wildcard,
        })
    }

    fn expand_shorthand(expr: &str) -> Option<String> {
        match expr {
            "@yearly" | "@annually" => Some("0 0 1 1 *".to_string()),
            "@monthly" => Some("0 0 1 * *".to_string()),
            "@weekly" => Some("0 0 * * 0".to_string()),
            "@daily" | "@midnight" => Some("0 0 * * *".to_string()),
            "@hourly" => Some("0 * * * *".to_string()),
            _ => None,
        }
    }

    fn parse_field(field: &str, min: u32, max: u32) -> Result<BTreeSet<u32>, NousError> {
        let (set, _) = Self::parse_field_with_wildcard(field, min, max)?;
        Ok(set)
    }

    fn parse_field_with_wildcard(
        field: &str,
        min: u32,
        max: u32,
    ) -> Result<(BTreeSet<u32>, bool), NousError> {
        let mut result = BTreeSet::new();
        let mut is_wildcard = false;

        for part in field.split(',') {
            if part.contains('/') {
                let parts: Vec<&str> = part.splitn(2, '/').collect();
                let step: u32 = parts[1].parse().map_err(|_| {
                    NousError::Validation(format!("invalid step value: {}", parts[1]))
                })?;
                if step == 0 {
                    return Err(NousError::Validation("step value cannot be 0".into()));
                }

                let (range_start, range_end) = if parts[0] == "*" {
                    is_wildcard = true;
                    (min, max)
                } else if parts[0].contains('-') {
                    let range: Vec<&str> = parts[0].splitn(2, '-').collect();
                    let start: u32 = range[0].parse().map_err(|_| {
                        NousError::Validation(format!("invalid range start: {}", range[0]))
                    })?;
                    let end: u32 = range[1].parse().map_err(|_| {
                        NousError::Validation(format!("invalid range end: {}", range[1]))
                    })?;
                    (start, end)
                } else {
                    let start: u32 = parts[0].parse().map_err(|_| {
                        NousError::Validation(format!("invalid value: {}", parts[0]))
                    })?;
                    (start, max)
                };

                let mut val = range_start;
                while val <= range_end {
                    if val >= min && val <= max {
                        result.insert(val);
                    }
                    val += step;
                }
            } else if part == "*" {
                is_wildcard = true;
                for v in min..=max {
                    result.insert(v);
                }
            } else if part.contains('-') {
                let range: Vec<&str> = part.splitn(2, '-').collect();
                let start: u32 = range[0].parse().map_err(|_| {
                    NousError::Validation(format!("invalid range start: {}", range[0]))
                })?;
                let end: u32 = range[1].parse().map_err(|_| {
                    NousError::Validation(format!("invalid range end: {}", range[1]))
                })?;
                if start > end {
                    return Err(NousError::Validation(format!(
                        "invalid range: {start}-{end}"
                    )));
                }
                for v in start..=end {
                    if v >= min && v <= max {
                        result.insert(v);
                    }
                }
            } else {
                let val: u32 = part
                    .parse()
                    .map_err(|_| NousError::Validation(format!("invalid value: {part}")))?;
                if val < min || val > max {
                    return Err(NousError::Validation(format!(
                        "value {val} out of range {min}-{max}"
                    )));
                }
                result.insert(val);
            }
        }

        Ok((result, is_wildcard))
    }

    pub fn next_run(&self, after: i64) -> Option<i64> {
        let dt = chrono::DateTime::from_timestamp(after, 0)?;
        let mut current = dt.naive_utc() + chrono::Duration::minutes(1);
        current = current
            .date()
            .and_time(NaiveTime::from_hms_opt(current.hour(), current.minute(), 0)?);

        let limit = after + 4 * 366 * 86400;

        loop {
            let ts = current.and_utc().timestamp();
            if ts > limit {
                return None;
            }

            if !self.months.contains(&current.month()) {
                current = Self::next_month(current)?;
                continue;
            }

            if !self.day_matches(&current) {
                current = Self::next_day(current)?;
                continue;
            }

            if !self.hours.contains(&current.hour()) {
                current = Self::next_hour(current)?;
                continue;
            }

            if !self.minutes.contains(&current.minute()) {
                if let Some(next_min) = self.minutes.range((current.minute() + 1)..).next() {
                    current = current
                        .date()
                        .and_time(NaiveTime::from_hms_opt(current.hour(), *next_min, 0)?);
                } else {
                    current = Self::next_hour(current)?;
                }
                continue;
            }

            return Some(current.and_utc().timestamp());
        }
    }

    fn day_matches(&self, dt: &NaiveDateTime) -> bool {
        let dom = dt.day();
        let dow = dt.weekday().num_days_from_sunday();

        if self.dom_is_wildcard && self.dow_is_wildcard {
            return true;
        }
        if self.dom_is_wildcard {
            return self.days_of_week.contains(&dow);
        }
        if self.dow_is_wildcard {
            return self.days_of_month.contains(&dom);
        }
        self.days_of_month.contains(&dom) || self.days_of_week.contains(&dow)
    }

    fn next_month(dt: NaiveDateTime) -> Option<NaiveDateTime> {
        let (year, month) = if dt.month() == 12 {
            (dt.year() + 1, 1)
        } else {
            (dt.year(), dt.month() + 1)
        };
        let date = NaiveDate::from_ymd_opt(year, month, 1)?;
        date.and_hms_opt(0, 0, 0)
    }

    fn next_day(dt: NaiveDateTime) -> Option<NaiveDateTime> {
        let next = dt.date().succ_opt()?;
        next.and_hms_opt(0, 0, 0)
    }

    fn next_hour(dt: NaiveDateTime) -> Option<NaiveDateTime> {
        if dt.hour() == 23 {
            Self::next_day(dt)
        } else {
            Some(
                dt.date()
                    .and_time(NaiveTime::from_hms_opt(dt.hour() + 1, 0, 0)?),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_every_minute() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        assert_eq!(expr.minutes.len(), 60);
        assert_eq!(expr.hours.len(), 24);
        assert!(expr.dom_is_wildcard);
        assert!(expr.dow_is_wildcard);
    }

    #[test]
    fn parse_specific_values() {
        let expr = CronExpr::parse("5 3 15 6 2").unwrap();
        assert_eq!(expr.minutes, BTreeSet::from([5]));
        assert_eq!(expr.hours, BTreeSet::from([3]));
        assert_eq!(expr.days_of_month, BTreeSet::from([15]));
        assert_eq!(expr.months, BTreeSet::from([6]));
        assert_eq!(expr.days_of_week, BTreeSet::from([2]));
    }

    #[test]
    fn parse_ranges() {
        let expr = CronExpr::parse("1-5 9-17 * * 1-5").unwrap();
        assert_eq!(expr.minutes, BTreeSet::from([1, 2, 3, 4, 5]));
        assert_eq!(
            expr.hours,
            BTreeSet::from([9, 10, 11, 12, 13, 14, 15, 16, 17])
        );
        assert!(!expr.dow_is_wildcard);
    }

    #[test]
    fn parse_step() {
        let expr = CronExpr::parse("*/15 * * * *").unwrap();
        assert_eq!(expr.minutes, BTreeSet::from([0, 15, 30, 45]));
    }

    #[test]
    fn parse_step_on_range() {
        let expr = CronExpr::parse("1-10/3 * * * *").unwrap();
        assert_eq!(expr.minutes, BTreeSet::from([1, 4, 7, 10]));
    }

    #[test]
    fn parse_list() {
        let expr = CronExpr::parse("0,30 * * * *").unwrap();
        assert_eq!(expr.minutes, BTreeSet::from([0, 30]));
    }

    #[test]
    fn parse_shorthand_hourly() {
        let expr = CronExpr::parse("@hourly").unwrap();
        assert_eq!(expr.minutes, BTreeSet::from([0]));
        assert_eq!(expr.hours.len(), 24);
    }

    #[test]
    fn parse_shorthand_daily() {
        let expr = CronExpr::parse("@daily").unwrap();
        assert_eq!(expr.minutes, BTreeSet::from([0]));
        assert_eq!(expr.hours, BTreeSet::from([0]));
    }

    #[test]
    fn parse_shorthand_weekly() {
        let expr = CronExpr::parse("@weekly").unwrap();
        assert_eq!(expr.days_of_week, BTreeSet::from([0]));
    }

    #[test]
    fn parse_shorthand_monthly() {
        let expr = CronExpr::parse("@monthly").unwrap();
        assert_eq!(expr.days_of_month, BTreeSet::from([1]));
    }

    #[test]
    fn parse_shorthand_yearly() {
        let expr = CronExpr::parse("@yearly").unwrap();
        assert_eq!(expr.days_of_month, BTreeSet::from([1]));
        assert_eq!(expr.months, BTreeSet::from([1]));
    }

    #[test]
    fn parse_invalid_field_count() {
        assert!(CronExpr::parse("* * *").is_err());
    }

    #[test]
    fn parse_invalid_value() {
        assert!(CronExpr::parse("60 * * * *").is_err());
    }

    #[test]
    fn parse_invalid_step_zero() {
        assert!(CronExpr::parse("*/0 * * * *").is_err());
    }

    #[test]
    fn next_run_every_minute() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        let base = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(12, 30, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let next = expr.next_run(base).unwrap();
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(12, 31, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(next, expected);
    }

    #[test]
    fn next_run_specific_time() {
        let expr = CronExpr::parse("30 14 * * *").unwrap();
        let base = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let next = expr.next_run(base).unwrap();
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(14, 30, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(next, expected);
    }

    #[test]
    fn next_run_wraps_to_next_day() {
        let expr = CronExpr::parse("0 9 * * *").unwrap();
        let base = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let next = expr.next_run(base).unwrap();
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 1, 2)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(next, expected);
    }

    #[test]
    fn next_run_respects_dow() {
        // Monday = 1
        let expr = CronExpr::parse("0 9 * * 1").unwrap();
        // 2026-01-01 is a Thursday
        let base = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let next = expr.next_run(base).unwrap();
        // Next Monday is 2026-01-05
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 1, 5)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(next, expected);
    }

    #[test]
    fn next_run_dom_dow_or_rule() {
        // Both DOM=15 and DOW=0(Sunday) specified — fires if either matches
        let expr = CronExpr::parse("0 0 15 * 0").unwrap();
        assert!(!expr.dom_is_wildcard);
        assert!(!expr.dow_is_wildcard);
        // 2026-01-01 Thursday. Next Sunday is Jan 4; 15th is Jan 15.
        // Sunday comes first.
        let base = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let next = expr.next_run(base).unwrap();
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 1, 4)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(next, expected);
    }

    #[test]
    fn mock_clock_works() {
        let clock = MockClock::new(1000);
        assert_eq!(clock.now_utc(), 1000);
        clock.advance(500);
        assert_eq!(clock.now_utc(), 1500);
        clock.set(2000);
        assert_eq!(clock.now_utc(), 2000);
    }
}
