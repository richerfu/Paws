use time::OffsetDateTime;

pub(crate) fn format_unix_seconds(value: &str) -> Option<String> {
    let seconds = value.trim().parse::<i64>().ok()?;
    format_utc_seconds(seconds)
}

pub(crate) fn format_unix_nanos(value: &str) -> Option<String> {
    let nanos = value.trim().parse::<u128>().ok()?;
    let seconds = i64::try_from(nanos / 1_000_000_000).ok()?;
    format_utc_seconds(seconds)
}

fn format_utc_seconds(seconds: i64) -> Option<String> {
    let datetime = OffsetDateTime::from_unix_timestamp(seconds).ok()?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        datetime.year(),
        u8::from(datetime.month()),
        datetime.day(),
        datetime.hour(),
        datetime.minute()
    ))
}
