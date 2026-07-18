#[path = "../src/time_format.rs"]
mod time_format;

#[test]
fn formats_subscription_expire_unix_seconds() {
    assert_eq!(
        time_format::format_unix_seconds("1893456000").as_deref(),
        Some("2030-01-01 00:00 UTC")
    );
}

#[test]
fn formats_profile_nanosecond_timestamps() {
    assert_eq!(
        time_format::format_unix_nanos("1893456000123456789").as_deref(),
        Some("2030-01-01 00:00 UTC")
    );
}

#[test]
fn invalid_timestamps_are_left_to_callers() {
    assert!(time_format::format_unix_seconds("not-a-time").is_none());
    assert!(time_format::format_unix_nanos("999999999999999999999999999999999999999").is_none());
}
