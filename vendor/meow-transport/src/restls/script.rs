//! restls record-script DSL — a port of `parseRecordScript` /
//! `actAccordingToScript` from `restls-client-go` (`restls_utils.go`,
//! `conn.go`).
//!
//! The script is a comma-separated list of *lines*; each line describes the
//! target size of one outgoing TLS record while the script is replaying:
//!
//! ```text
//! line   := target [ "~" | "?" range ] [ "<" responses ]
//! target := integer           // record payload bytes, <= 32767
//! range  := integer           // random range; target+range <= 32768
//! responses := integer        // < 255
//! ```
//!
//! * `250` — a 250-byte record.
//! * `350~100` — `350 + rand(0..100)` bytes, resampled per record.
//! * `250?100` — same distribution but frozen once at parse time.
//! * `<N` — `ActResponse(N)`: after this record is sent, writes block until
//!   an inbound record arrives (request/response pacing); when *received* on
//!   an inbound record the peer is asked for N fake responses.
//!
//! The default script mirrors upstream `defaultRestlsScript`.

use rand::Rng;

use crate::{Result, TransportError};

/// Upstream `defaultRestlsScript`.
pub(crate) const DEFAULT_SCRIPT: &str = "250?100<1,350~100<1,600~100,300~200,300~100";

/// `maxPlaintext` — RFC 8446 record payload cap.
pub(crate) const MAX_PLAINTEXT: usize = 16384;

/// Per-line command carried by the script or embedded in a tagged record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    /// `0x00` — no action.
    Noop,
    /// `0x01 <n>` — respond with `n` fake (all-padding) records. On the write
    /// side this marks the record as interrupting: subsequent writes block
    /// until an inbound record arrives.
    Respond(u8),
}

impl Command {
    pub(crate) fn to_bytes(self) -> [u8; 2] {
        match self {
            Command::Noop => [0x00, 0x00],
            Command::Respond(n) => [0x01, n],
        }
    }

    /// Upstream `needInterrupt`.
    pub(crate) fn need_interrupt(self) -> bool {
        matches!(self, Command::Respond(_))
    }
}

/// 2-byte command embedded in each tagged record (upstream `parseCommand`).
pub(crate) fn parse_command(buf: &[u8]) -> Result<Command> {
    if buf.len() < 2 {
        return Err(TransportError::Tls("restls: short command".into()));
    }
    match buf[0] {
        0x00 => Ok(Command::Noop),
        0x01 => Ok(Command::Respond(buf[1])),
        _ => Err(TransportError::Tls("restls: unsupported command".into())),
    }
}

/// `TargetLength` — `[fixed, random_range]`; `len()` = `fixed + rand(range)`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TargetLength {
    fixed: i32,
    range: i32,
}

impl TargetLength {
    fn len(&self) -> usize {
        if self.range != 0 {
            (self.fixed + rand::rng().random_range(0..self.range)) as usize
        } else {
            self.fixed as usize
        }
    }
}

/// One script line: a target record length plus an optional command.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Line {
    pub(crate) target_len: TargetLength,
    pub(crate) command: Command,
}

/// Parse a `restls-script` string. Mirrors upstream `parseRecordScript`.
pub(crate) fn parse_record_script(script: &str) -> Result<Vec<Line>> {
    let mut lines = Vec::new();
    for raw in script.replace(' ', "").split(',') {
        if raw.is_empty() {
            continue;
        }
        let mut rest = raw.as_bytes();
        // Upstream `getInteger` yields 0 on a missing target — `~10` is a
        // valid zero-fixed line. Non-numeric lines still fail below via
        // "unexpected content".
        let target =
            take_integer(&mut rest).ok_or_else(|| invalid_script(raw, "target len overflow"))?;
        if target > 32767 {
            return Err(invalid_script(raw, "target len > 32767"));
        }
        let mut target_len = TargetLength {
            fixed: target as i32,
            range: 0,
        };
        if matches!(rest.first(), Some(b'~' | b'?')) {
            let kind = rest[0];
            rest = &rest[1..];
            let range = take_integer(&mut rest)
                .ok_or_else(|| invalid_script(raw, "random range overflow"))?;
            if range > 32767 {
                return Err(invalid_script(raw, "random target range > 32767"));
            }
            if range + target > 32768 {
                return Err(invalid_script(raw, "random target len > 32768"));
            }
            target_len.range = range as i32;
            if kind == b'?' {
                // '?' resolves the random component once, at parse time.
                target_len.fixed = target_len.len() as i32;
                target_len.range = 0;
            }
        }
        let command = if matches!(rest.first(), Some(b'<')) {
            rest = &rest[1..];
            let responses = take_integer(&mut rest)
                .ok_or_else(|| invalid_script(raw, "response count overflow"))?;
            if !rest.is_empty() {
                return Err(invalid_script(raw, "trailing garbage after '<'"));
            }
            if responses >= 255 {
                return Err(invalid_script(raw, "response count >= 255"));
            }
            Command::Respond(responses as u8)
        } else if rest.is_empty() {
            Command::Noop
        } else {
            return Err(invalid_script(raw, "unexpected content"));
        };
        lines.push(Line {
            target_len,
            command,
        });
    }
    Ok(lines)
}

/// Decimal integer at the head of `*rest`. Mirrors upstream `getInteger`
/// exactly: no digits → `0` (so `~10`, `10~`, `10<` all parse); only
/// intermediate overflow > 32768 is an error.
fn take_integer(rest: &mut &[u8]) -> Option<usize> {
    let mut res = 0usize;
    let mut i = 0;
    while i < rest.len() && rest[i].is_ascii_digit() {
        res = res * 10 + (rest[i] - b'0') as usize;
        if res > 32768 {
            return None;
        }
        i += 1;
    }
    *rest = &rest[i..];
    Some(res)
}

fn invalid_script(line: &str, why: &str) -> TransportError {
    TransportError::Config(format!("restls: invalid script {line:?}: {why}"))
}

/// Compute the record layout for one outgoing record — upstream
/// `actAccordingToScript`. `data` is the remaining application bytes;
/// `to_server_counter` indexes into the script (records beyond the script are
/// unsized); `header_len` is the per-record restls auth overhead (12, or 20
/// for TLS 1.2 GCM mode).
///
/// Returns `(payload_len, data_len, padding_len, command)` where
/// `payload_len` is the TLS record payload size (data + padding + header) and
/// `data_len` is the slice of `data` to embed.
pub(crate) fn act_according_to_script(
    data: &[u8],
    to_server_counter: u64,
    script: &[Line],
    header_len: usize,
) -> (usize, usize, usize, Command) {
    let mut padding_len = 0usize;
    let mut data_len = data.len();
    let mut command = Command::Noop;
    if (to_server_counter as usize) < script.len() {
        let line = script[to_server_counter as usize];
        data_len = line.target_len.len();
        command = line.command;
    }
    if data_len == 0 {
        padding_len = 19 + rand::rng().random_range(0..100);
    }
    if data.len() < data_len {
        padding_len = data_len - data.len();
        data_len = data.len();
    }
    // Upstream clamps the payload first, then recomputes
    // `data_len = payload - header - padding` — the padding is kept and the
    // *data* shrinks under an over-cap script target. A negative result
    // panics upstream; clamp the padding instead so a bad `restls-script`
    // degrades rather than crashing the task.
    let payload_len = (data_len + padding_len + header_len).min(MAX_PLAINTEXT);
    let room = payload_len.saturating_sub(header_len);
    let padding_len = padding_len.min(room);
    let data_len = room - padding_len;
    (payload_len, data_len, padding_len, command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Vec<(i32, i32, Command)> {
        parse_record_script(s)
            .unwrap()
            .iter()
            .map(|l| (l.target_len.fixed, l.target_len.range, l.command))
            .collect()
    }

    #[test]
    fn parses_plain_targets() {
        assert_eq!(
            parse("100,200,300"),
            vec![
                (100, 0, Command::Noop),
                (200, 0, Command::Noop),
                (300, 0, Command::Noop)
            ]
        );
    }

    #[test]
    fn parses_random_range() {
        assert_eq!(parse("350~100"), vec![(350, 100, Command::Noop)]);
    }

    #[test]
    fn question_mark_freezes_range() {
        // '?': range resolved once at parse → fixed, range=0.
        let (fixed, range, _) = parse("250?100")[0];
        assert_eq!(range, 0);
        assert!((250..350).contains(&fixed));
    }

    #[test]
    fn parses_response_command() {
        assert_eq!(parse("250<3"), vec![(250, 0, Command::Respond(3))]);
        assert_eq!(parse("350~100<1"), vec![(350, 100, Command::Respond(1))]);
    }

    #[test]
    fn parses_default_script() {
        let lines = parse_record_script(DEFAULT_SCRIPT).unwrap();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0].command, Command::Respond(1));
        assert_eq!(lines[1].command, Command::Respond(1));
    }

    #[test]
    fn rejects_bad_scripts() {
        for bad in [
            "abc",
            "32768",
            "100~32768",
            "100<255",
            "10<3x",
            "10<x",
            "32769~1",
        ] {
            assert!(parse_record_script(bad).is_err(), "expected reject: {bad}");
        }
    }

    #[test]
    fn empty_integers_default_to_zero() {
        // Upstream `getInteger` returns 0 on missing digits — `~10`, `10~`,
        // `10<` are all accepted.
        assert_eq!(parse("~10"), vec![(0, 10, Command::Noop)]);
        assert_eq!(parse("10~"), vec![(10, 0, Command::Noop)]);
        assert_eq!(parse("10<"), vec![(10, 0, Command::Respond(0))]);
    }

    #[test]
    fn spaces_are_ignored() {
        assert_eq!(
            parse(" 100 , 200 "),
            vec![(100, 0, Command::Noop), (200, 0, Command::Noop)]
        );
    }

    #[test]
    fn command_round_trip() {
        assert_eq!(
            parse_command(&Command::Noop.to_bytes()).unwrap(),
            Command::Noop
        );
        assert_eq!(
            parse_command(&Command::Respond(7).to_bytes()).unwrap(),
            Command::Respond(7)
        );
        assert!(parse_command(&[0x02, 0x00]).is_err());
    }

    #[test]
    fn act_pads_to_target() {
        // data shorter than target → padding = target - data; the record
        // payload ends up target + header (upstream: `dataLen` is the data
        // region, header rides on top).
        let (payload, data_len, padding, cmd) =
            act_according_to_script(&[0u8; 100], 0, &parse_record_script("500").unwrap(), 12);
        assert_eq!(payload, 500 + 12);
        assert_eq!(data_len, 100);
        assert_eq!(padding, 400);
        assert_eq!(cmd, Command::Noop);
    }

    #[test]
    fn act_splits_oversize() {
        // data longer than target → record carries `target` data bytes.
        let (payload, data_len, padding, _) =
            act_according_to_script(&[0u8; 1000], 0, &parse_record_script("300").unwrap(), 12);
        assert_eq!(payload, 300 + 12);
        assert_eq!(data_len, 300);
        assert_eq!(padding, 0);
    }

    #[test]
    fn act_caps_at_max_plaintext() {
        let (payload, data_len, _, _) =
            act_according_to_script(&[0u8; 20000], 0, &parse_record_script("20000").unwrap(), 12);
        assert_eq!(payload, MAX_PLAINTEXT);
        assert_eq!(data_len, MAX_PLAINTEXT - 12);
    }

    #[test]
    fn act_beyond_script_is_unsized() {
        let (payload, data_len, padding, _) =
            act_according_to_script(&[0u8; 700], 5, &parse_record_script("300").unwrap(), 12);
        assert_eq!(payload, 700 + 12);
        assert_eq!(data_len, 700);
        assert_eq!(padding, 0);
    }
}
