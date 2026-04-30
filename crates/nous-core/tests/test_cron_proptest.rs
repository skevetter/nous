use nous_core::schedules::CronExpr;
use proptest::prelude::*;

fn arb_minute() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("*".to_string()),
        (0u32..60).prop_map(|v| v.to_string()),
        (0u32..50, 1u32..11).prop_map(|(start, span)| format!("{}-{}", start, start + span)),
        prop_oneof![Just(5u32), Just(10), Just(15), Just(20), Just(30)]
            .prop_map(|step| format!("*/{step}")),
    ]
}

fn arb_hour() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("*".to_string()),
        (0u32..24).prop_map(|v| v.to_string()),
        (0u32..20, 1u32..5).prop_map(|(start, span)| format!("{}-{}", start, start + span)),
        prop_oneof![Just(2u32), Just(3), Just(4), Just(6), Just(8), Just(12)]
            .prop_map(|step| format!("*/{step}")),
    ]
}

fn arb_dom() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("*".to_string()),
        (1u32..32).prop_map(|v| v.to_string()),
        (1u32..28, 1u32..5).prop_map(|(start, span)| format!("{}-{}", start, start + span)),
    ]
}

fn arb_month() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("*".to_string()),
        (1u32..13).prop_map(|v| v.to_string()),
        (1u32..10, 1u32..4).prop_map(|(start, span)| format!("{}-{}", start, start + span)),
    ]
}

fn arb_dow() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("*".to_string()),
        (0u32..7).prop_map(|v| v.to_string()),
        (0u32..5, 1u32..3).prop_map(|(start, span)| format!("{}-{}", start, start + span)),
    ]
}

fn arb_cron_expr() -> impl Strategy<Value = String> {
    (arb_minute(), arb_hour(), arb_dom(), arb_month(), arb_dow())
        .prop_map(|(min, hr, dom, mon, dow)| format!("{min} {hr} {dom} {mon} {dow}"))
}

proptest! {
    #[test]
    fn parse_never_panics(expr in arb_cron_expr()) {
        let _ = CronExpr::parse(&expr);
    }

    #[test]
    fn valid_expr_parses_successfully(expr in arb_cron_expr()) {
        let result = CronExpr::parse(&expr);
        prop_assert!(result.is_ok(), "Failed to parse valid expr: {}", expr);
    }

    #[test]
    fn next_run_after_is_always_in_future(expr in arb_cron_expr()) {
        let parsed = CronExpr::parse(&expr).unwrap();
        let base = 1_700_000_000i64;
        if let Some(next) = parsed.next_run(base) {
            prop_assert!(next > base, "next_run {} must be > base {}", next, base);
        }
    }

    #[test]
    fn next_run_is_deterministic(expr in arb_cron_expr()) {
        let parsed = CronExpr::parse(&expr).unwrap();
        let base = 1_700_000_000i64;
        let r1 = parsed.next_run(base);
        let r2 = parsed.next_run(base);
        prop_assert_eq!(r1, r2);
    }

    #[test]
    fn next_run_result_matches_fields(expr in arb_cron_expr()) {
        let parsed = CronExpr::parse(&expr).unwrap();
        let base = 1_700_000_000i64;
        if let Some(next) = parsed.next_run(base) {
            let dt = chrono::DateTime::from_timestamp(next, 0).unwrap().naive_utc();
            let min = chrono::Timelike::minute(&dt);
            let hr = chrono::Timelike::hour(&dt);
            let mon = chrono::Datelike::month(&dt);
            prop_assert!(parsed.minutes.contains(&min));
            prop_assert!(parsed.hours.contains(&hr));
            prop_assert!(parsed.months.contains(&mon));
        }
    }

    #[test]
    fn shorthand_always_parses(s in prop_oneof![
        Just("@hourly"),
        Just("@daily"),
        Just("@weekly"),
        Just("@monthly"),
        Just("@yearly"),
        Just("@annually"),
        Just("@midnight"),
    ]) {
        let result = CronExpr::parse(s);
        prop_assert!(result.is_ok());
    }
}
