use std::collections::BTreeSet;

use chrono::{Datelike, Duration, NaiveDate, TimeZone, Timelike};
use chrono_tz::Tz;

#[derive(Debug, Clone)]
pub struct CronExpr {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    days_of_month: BTreeSet<u32>,
    months: BTreeSet<u32>,
    days_of_week: BTreeSet<u32>,
    dom_is_wildcard: bool,
    dow_is_wildcard: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CronParseError {
    #[error("empty expression")]
    Empty,
    #[error("expected 5 fields, got {0}")]
    WrongFieldCount(usize),
    #[error("invalid value in {field} field: {detail}")]
    InvalidField { field: &'static str, detail: String },
}

const FIELD_DEFS: [(&str, u32, u32); 5] = [
    ("minute", 0, 59),
    ("hour", 0, 23),
    ("day-of-month", 1, 31),
    ("month", 1, 12),
    ("day-of-week", 0, 6),
];

fn parse_field(
    token: &str,
    name: &'static str,
    min: u32,
    max: u32,
) -> Result<(BTreeSet<u32>, bool), CronParseError> {
    let err = |detail: String| CronParseError::InvalidField {
        field: name,
        detail,
    };

    let mut values = BTreeSet::new();
    let mut is_wildcard = false;

    for part in token.split(',') {
        let (base, step) = match part.split_once('/') {
            Some((b, s)) => {
                let step: u32 = s.parse().map_err(|_| err(format!("bad step '{s}'")))?;
                if step == 0 {
                    return Err(err("step cannot be 0".into()));
                }
                (b, Some(step))
            }
            None => (part, None),
        };

        let (range_start, range_end) = if base == "*" {
            if step.is_none() {
                is_wildcard = true;
            }
            (min, max)
        } else if let Some((lo, hi)) = base.split_once('-') {
            let lo: u32 = lo.parse().map_err(|_| err(format!("bad value '{lo}'")))?;
            let hi: u32 = hi.parse().map_err(|_| err(format!("bad value '{hi}'")))?;
            if lo < min || hi > max {
                return Err(err(format!("range {lo}-{hi} outside {min}-{max}")));
            }
            if lo > hi {
                return Err(err(format!("range start {lo} > end {hi}")));
            }
            (lo, hi)
        } else {
            let val: u32 = base
                .parse()
                .map_err(|_| err(format!("bad value '{base}'")))?;
            if val < min || val > max {
                return Err(err(format!("value {val} outside {min}-{max}")));
            }
            match step {
                Some(_) => (val, max),
                None => {
                    values.insert(val);
                    continue;
                }
            }
        };

        match step {
            Some(s) => {
                let mut v = range_start;
                while v <= range_end {
                    values.insert(v);
                    v += s;
                }
            }
            None => {
                for v in range_start..=range_end {
                    values.insert(v);
                }
            }
        }
    }

    Ok((values, is_wildcard))
}

impl CronExpr {
    pub fn parse(expr: &str) -> Result<Self, CronParseError> {
        let expr = expr.trim();
        if expr.is_empty() {
            return Err(CronParseError::Empty);
        }

        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(CronParseError::WrongFieldCount(fields.len()));
        }

        let (minutes, _) =
            parse_field(fields[0], FIELD_DEFS[0].0, FIELD_DEFS[0].1, FIELD_DEFS[0].2)?;
        let (hours, _) = parse_field(fields[1], FIELD_DEFS[1].0, FIELD_DEFS[1].1, FIELD_DEFS[1].2)?;
        let (days_of_month, dom_is_wildcard) =
            parse_field(fields[2], FIELD_DEFS[2].0, FIELD_DEFS[2].1, FIELD_DEFS[2].2)?;
        let (months, _) =
            parse_field(fields[3], FIELD_DEFS[3].0, FIELD_DEFS[3].1, FIELD_DEFS[3].2)?;
        let (days_of_week, dow_is_wildcard) =
            parse_field(fields[4], FIELD_DEFS[4].0, FIELD_DEFS[4].1, FIELD_DEFS[4].2)?;

        Ok(CronExpr {
            minutes,
            hours,
            days_of_month,
            months,
            days_of_week,
            dom_is_wildcard,
            dow_is_wildcard,
        })
    }

    pub fn next_run(&self, after: chrono::DateTime<Tz>) -> Option<chrono::DateTime<Tz>> {
        let tz = after.timezone();
        // Advance one minute and zero out seconds via UTC to avoid DST gap issues
        let utc_next = after.naive_utc() + Duration::minutes(1);
        let utc_snapped =
            utc_next
                .date()
                .and_hms_opt(utc_next.time().hour(), utc_next.time().minute(), 0)?;
        let mut dt = utc_snapped.and_utc().with_timezone(&tz);

        // Safety limit: don't search more than 4 years ahead
        let limit = after + Duration::days(366 * 4 + 1);

        'outer: loop {
            if dt > limit {
                return None;
            }

            // Month
            if !self.months.contains(&dt.month()) {
                // Advance to the first day of the next matching month
                dt = advance_month(&tz, dt, &self.months)?;
                continue 'outer;
            }

            // Day — OR semantics when both dom and dow are constrained
            if !self.day_matches(&dt) {
                dt = next_day(&tz, dt)?;
                continue 'outer;
            }

            // Hour
            if !self.hours.contains(&dt.hour()) {
                if let Some(next_h) = self.hours.range(dt.hour()..).next() {
                    let local = dt
                        .date_naive()
                        .and_hms_opt(*next_h, *self.minutes.first()?, 0)?;
                    match tz.from_local_datetime(&local).earliest() {
                        Some(resolved) => {
                            dt = resolved;
                            continue 'outer;
                        }
                        None => {
                            // DST gap — this hour doesn't exist today, advance to next day
                            dt = next_day(&tz, dt)?;
                            continue 'outer;
                        }
                    }
                }
                dt = next_day(&tz, dt)?;
                continue 'outer;
            }

            // Minute
            if !self.minutes.contains(&dt.minute()) {
                if let Some(next_m) = self.minutes.range(dt.minute()..).next() {
                    let local = dt.date_naive().and_hms_opt(dt.hour(), *next_m, 0)?;
                    match tz.from_local_datetime(&local).earliest() {
                        Some(c) if c > after => {
                            dt = c;
                            continue 'outer;
                        }
                        _ => {}
                    }
                }
                // Advance to next hour
                if let Some(next_h) = self.hours.range((dt.hour() + 1)..).next() {
                    let local = dt
                        .date_naive()
                        .and_hms_opt(*next_h, *self.minutes.first()?, 0)?;
                    match tz.from_local_datetime(&local).earliest() {
                        Some(resolved) => {
                            dt = resolved;
                            continue 'outer;
                        }
                        None => {
                            dt = next_day(&tz, dt)?;
                            continue 'outer;
                        }
                    }
                }
                dt = next_day(&tz, dt)?;
                continue 'outer;
            }

            // All fields match — resolve to a concrete timezone-aware instant
            let resolved = tz
                .from_local_datetime(&dt.date_naive().and_hms_opt(dt.hour(), dt.minute(), 0)?)
                .earliest();

            match resolved {
                Some(r) if r > after => return Some(r),
                _ => {
                    // DST gap — this local time doesn't exist; advance one minute
                    dt = advance_minute(&tz, dt)?;
                    continue 'outer;
                }
            }
        }
    }

    fn day_matches(&self, dt: &chrono::DateTime<Tz>) -> bool {
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
        // Both constrained — OR semantics per POSIX cron
        self.days_of_month.contains(&dom) || self.days_of_week.contains(&dow)
    }
}

fn advance_month(
    tz: &Tz,
    dt: chrono::DateTime<Tz>,
    months: &BTreeSet<u32>,
) -> Option<chrono::DateTime<Tz>> {
    let mut year = dt.year();
    let mut month = dt.month();

    for _ in 0..48 {
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
        if months.contains(&month) {
            let date = NaiveDate::from_ymd_opt(year, month, 1)?;
            return tz
                .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
                .earliest();
        }
    }
    None
}

fn next_day(tz: &Tz, dt: chrono::DateTime<Tz>) -> Option<chrono::DateTime<Tz>> {
    let next = dt.date_naive().succ_opt()?;
    tz.from_local_datetime(&next.and_hms_opt(0, 0, 0)?)
        .earliest()
}

fn advance_minute(tz: &Tz, dt: chrono::DateTime<Tz>) -> Option<chrono::DateTime<Tz>> {
    let utc = dt.naive_utc() + Duration::minutes(1);
    Some(utc.and_utc().with_timezone(tz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::US::Eastern;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<Tz> {
        chrono_tz::UTC.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn eastern(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<Tz> {
        Eastern.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    // ── Standard expression tests ──

    #[test]
    fn parse_every_minute() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        let after = utc(2026, 1, 15, 10, 30);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2026, 1, 15, 10, 31));
    }

    #[test]
    fn parse_hourly() {
        let expr = CronExpr::parse("0 * * * *").unwrap();
        let after = utc(2026, 1, 15, 10, 30);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2026, 1, 15, 11, 0));
    }

    #[test]
    fn parse_daily() {
        let expr = CronExpr::parse("0 0 * * *").unwrap();
        let after = utc(2026, 1, 15, 10, 30);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2026, 1, 16, 0, 0));
    }

    #[test]
    fn parse_monthly() {
        let expr = CronExpr::parse("0 0 1 * *").unwrap();
        let after = utc(2026, 1, 15, 10, 30);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2026, 2, 1, 0, 0));
    }

    #[test]
    fn parse_yearly() {
        let expr = CronExpr::parse("0 0 1 1 *").unwrap();
        let after = utc(2026, 3, 15, 10, 30);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2027, 1, 1, 0, 0));
    }

    // ── Edge case tests ──

    #[test]
    fn dst_spring_forward() {
        // US Eastern: 2026-03-08 at 2:00 AM clocks jump to 3:00 AM
        // An expression targeting 2:30 AM should skip to 3:00 AM (next matching minute)
        let expr = CronExpr::parse("30 2 * * *").unwrap();
        let after = eastern(2026, 3, 7, 23, 0);
        let next = expr.next_run(after);
        // 2:30 doesn't exist on spring-forward day, so it should skip to next valid occurrence
        // which is March 9th at 2:30 AM (when the day exists again)
        assert!(next.is_some());
        let next = next.unwrap();
        // Should NOT fire on March 8 (gap day)
        assert!(next.day() != 8 || next.month() != 3 || next.hour() != 2);
    }

    #[test]
    fn dst_fall_back() {
        // US Eastern: 2026-11-01 at 2:00 AM clocks fall back to 1:00 AM
        // 1:30 AM occurs twice — next_run should return only once (the earliest)
        let expr = CronExpr::parse("30 1 * * *").unwrap();
        let after = eastern(2026, 10, 31, 23, 0);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next.hour(), 1);
        assert_eq!(next.minute(), 30);
        assert_eq!(next.day(), 1);
        assert_eq!(next.month(), 11);
    }

    #[test]
    fn leap_year_feb_29() {
        let expr = CronExpr::parse("0 0 29 2 *").unwrap();
        let after = utc(2027, 1, 1, 0, 0);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2028, 2, 29, 0, 0));
    }

    #[test]
    fn non_leap_year_feb_29() {
        let expr = CronExpr::parse("0 0 29 2 *").unwrap();
        let after = utc(2025, 3, 1, 0, 0);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2028, 2, 29, 0, 0));
    }

    #[test]
    fn month_end_31st() {
        // April has 30 days — day 31 should skip to May 31
        let expr = CronExpr::parse("0 0 31 * *").unwrap();
        let after = utc(2026, 3, 31, 1, 0);
        let next = expr.next_run(after).unwrap();
        // April has only 30 days, so next 31st is May
        assert_eq!(next, utc(2026, 5, 31, 0, 0));
    }

    #[test]
    fn month_end_30th_feb() {
        // Feb never has 30 days
        let expr = CronExpr::parse("0 0 30 * *").unwrap();
        let after = utc(2026, 1, 31, 0, 0);
        let next = expr.next_run(after).unwrap();
        // Should skip Feb entirely, land on March 30
        assert_eq!(next, utc(2026, 3, 30, 0, 0));
    }

    #[test]
    fn midnight_boundary() {
        let expr = CronExpr::parse("59 23 * * *").unwrap();
        let after = utc(2026, 1, 15, 23, 58);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2026, 1, 15, 23, 59));
    }

    #[test]
    fn year_boundary() {
        let expr = CronExpr::parse("0 0 1 1 *").unwrap();
        let after = utc(2026, 12, 31, 23, 59);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next, utc(2027, 1, 1, 0, 0));
    }

    #[test]
    fn dow_and_dom_or_semantics() {
        // Day 15 OR Monday — should fire on whichever comes first
        let expr = CronExpr::parse("0 0 15 * 1").unwrap();
        // 2026-01-12 is a Monday
        let after = utc(2026, 1, 11, 0, 0);
        let next = expr.next_run(after).unwrap();
        // Monday Jan 12 comes before Jan 15
        assert_eq!(next, utc(2026, 1, 12, 0, 0));
    }

    #[test]
    fn all_fields_constrained() {
        // minute=30, hour=14, dom=15, month=6, dow=3(Wednesday)
        // OR semantics: fires when (month=6 AND minute=30 AND hour=14) AND (dom=15 OR dow=Wednesday)
        let expr = CronExpr::parse("30 14 15 6 3").unwrap();
        let after = utc(2026, 1, 1, 0, 0);
        let next = expr.next_run(after).unwrap();
        assert_eq!(next.month(), 6);
        assert_eq!(next.hour(), 14);
        assert_eq!(next.minute(), 30);
        // Should be either the 15th or a Wednesday in June
        let is_dom = next.day() == 15;
        let is_dow = next.weekday().num_days_from_sunday() == 3;
        assert!(is_dom || is_dow);
    }

    // ── Boundary condition tests ──

    #[test]
    fn field_min_values() {
        let expr = CronExpr::parse("0 0 1 1 0").unwrap();
        assert!(expr.minutes.contains(&0));
        assert!(expr.hours.contains(&0));
        assert!(expr.days_of_month.contains(&1));
        assert!(expr.months.contains(&1));
        assert!(expr.days_of_week.contains(&0));
    }

    #[test]
    fn field_max_values() {
        let expr = CronExpr::parse("59 23 31 12 6").unwrap();
        assert!(expr.minutes.contains(&59));
        assert!(expr.hours.contains(&23));
        assert!(expr.days_of_month.contains(&31));
        assert!(expr.months.contains(&12));
        assert!(expr.days_of_week.contains(&6));
    }

    #[test]
    fn overlapping_range_and_list() {
        let expr = CronExpr::parse("1-5,3-7 * * * *").unwrap();
        let expected: BTreeSet<u32> = (1..=7).collect();
        assert_eq!(expr.minutes, expected);
    }

    #[test]
    fn step_exceeds_range() {
        // */60 on minute field (0-59) should only fire at 0
        let expr = CronExpr::parse("*/60 * * * *").unwrap();
        assert_eq!(expr.minutes.len(), 1);
        assert!(expr.minutes.contains(&0));
    }

    #[test]
    fn single_value_range() {
        let expr = CronExpr::parse("5-5 * * * *").unwrap();
        let expected: BTreeSet<u32> = [5].into();
        assert_eq!(expr.minutes, expected);
    }

    // ── Rejection tests ──

    #[test]
    fn reject_empty_expr() {
        assert!(matches!(CronExpr::parse(""), Err(CronParseError::Empty)));
    }

    #[test]
    fn reject_too_few_fields() {
        assert!(matches!(
            CronExpr::parse("* * *"),
            Err(CronParseError::WrongFieldCount(3))
        ));
    }

    #[test]
    fn reject_out_of_range() {
        assert!(matches!(
            CronExpr::parse("60 * * * *"),
            Err(CronParseError::InvalidField { .. })
        ));
    }

    // ── Property-based tests ──

    #[test]
    fn next_run_always_future() {
        let expressions = [
            "* * * * *",
            "0 * * * *",
            "0 0 * * *",
            "*/5 * * * *",
            "0 0 1 * *",
            "30 14 15 6 3",
            "0 0 29 2 *",
        ];
        let times = [
            utc(2026, 1, 1, 0, 0),
            utc(2026, 6, 15, 12, 30),
            utc(2026, 12, 31, 23, 59),
            utc(2027, 2, 28, 23, 59),
            utc(2028, 2, 29, 0, 0),
        ];
        for expr_str in &expressions {
            let expr = CronExpr::parse(expr_str).unwrap();
            for &t in &times {
                if let Some(next) = expr.next_run(t) {
                    assert!(
                        next > t,
                        "next_run({expr_str}, {t}) = {next} is not after {t}"
                    );
                }
            }
        }
    }

    #[test]
    fn next_run_matches_expr() {
        let cases: Vec<(&str, Box<dyn Fn(chrono::DateTime<Tz>) -> bool>)> = vec![
            ("*/15 * * * *", Box::new(|dt| dt.minute() % 15 == 0)),
            (
                "0 */6 * * *",
                Box::new(|dt| dt.minute() == 0 && dt.hour() % 6 == 0),
            ),
            (
                "0 0 1 * *",
                Box::new(|dt| dt.minute() == 0 && dt.hour() == 0 && dt.day() == 1),
            ),
            (
                "30 14 * * 1-5",
                Box::new(|dt| {
                    dt.minute() == 30
                        && dt.hour() == 14
                        && (1..=5).contains(&dt.weekday().num_days_from_sunday())
                }),
            ),
        ];

        let times = [
            utc(2026, 1, 1, 0, 0),
            utc(2026, 6, 15, 12, 30),
            utc(2026, 12, 31, 23, 59),
        ];

        for (expr_str, check) in &cases {
            let expr = CronExpr::parse(expr_str).unwrap();
            for &t in &times {
                if let Some(next) = expr.next_run(t) {
                    assert!(
                        check(next),
                        "next_run({expr_str}, {t}) = {next} does not match expression"
                    );
                }
            }
        }
    }
}
