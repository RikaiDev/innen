//! Time-window arithmetic for `--since`. Every form resolves to a UNIX epoch
//! second threshold, so the filter downstream compares one kind of value.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::conversation::ReadError;

/// Convert (year, month, day) to days since 1970-01-01 (Gregorian calendar algorithm).
pub(crate) fn ymd_to_days(y: u32, m: u32, d: u32) -> u64 {
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as u64 * 146097 + doe as u64).saturating_sub(719468)
}

/// Parse date/timestamp or duration string into UNIX epoch seconds threshold.
/// Default (None) is the start of yesterday UTC (yesterday + today).
pub fn parse_since(since: Option<&str>) -> Result<u64, ReadError> {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days_now = now_secs / 86400;

    let Some(s) = since.map(str::trim).filter(|s| !s.is_empty()) else {
        // Default: start of yesterday (00:00:00 UTC)
        return Ok(days_now.saturating_sub(1) * 86400);
    };

    if s.eq_ignore_ascii_case("today") {
        return Ok(days_now * 86400);
    }
    if s.eq_ignore_ascii_case("yesterday") {
        return Ok(days_now.saturating_sub(1) * 86400);
    }

    // Relative duration e.g. "2d", "48h", "1d"
    if let Some(num_str) = s.strip_suffix('d').or_else(|| s.strip_suffix('D')) {
        if let Ok(days) = num_str.parse::<u64>() {
            return Ok(now_secs.saturating_sub(days * 86400));
        }
    }
    if let Some(num_str) = s.strip_suffix('h').or_else(|| s.strip_suffix('H')) {
        if let Ok(hours) = num_str.parse::<u64>() {
            return Ok(now_secs.saturating_sub(hours * 3600));
        }
    }

    // Date "YYYY-MM-DD"
    if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() == 3 {
            if let (Ok(y), Ok(m), Ok(d)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
            ) {
                if (1..=12).contains(&m) && (1..=31).contains(&d) {
                    return Ok(ymd_to_days(y, m, d) * 86400);
                }
            }
        }
    }

    // RFC3339 datetime e.g. "YYYY-MM-DDTHH:MM:SSZ"
    if s.len() >= 19 && (s.contains('T') || s.contains(' ')) {
        let date_part = &s[..10];
        let time_part = &s[11..19];
        let parts_d: Vec<&str> = date_part.split('-').collect();
        let parts_t: Vec<&str> = time_part.split(':').collect();
        if parts_d.len() == 3 && parts_t.len() == 3 {
            if let (Ok(y), Ok(m), Ok(d), Ok(hr), Ok(mn), Ok(sc)) = (
                parts_d[0].parse::<u32>(),
                parts_d[1].parse::<u32>(),
                parts_d[2].parse::<u32>(),
                parts_t[0].parse::<u64>(),
                parts_t[1].parse::<u64>(),
                parts_t[2].parse::<u64>(),
            ) {
                let base_days = ymd_to_days(y, m, d);
                return Ok(base_days * 86400 + hr * 3600 + mn * 60 + sc);
            }
        }
    }

    Err(ReadError(format!(
        "invalid --since format: '{s}'; expected YYYY-MM-DD, RFC3339 timestamp, duration (e.g. 2d, 48h), today, or yesterday"
    )))
}

/// Convert RFC3339 timestamp string to UNIX epoch seconds.
pub fn rfc3339_to_secs(s: &str) -> Option<u64> {
    if s.len() >= 19 {
        let date_part = &s[..10];
        let time_part = &s[11..19];
        let parts_d: Vec<&str> = date_part.split('-').collect();
        let parts_t: Vec<&str> = time_part.split(':').collect();
        if parts_d.len() == 3 && parts_t.len() == 3 {
            if let (Ok(y), Ok(m), Ok(d), Ok(hr), Ok(mn), Ok(sc)) = (
                parts_d[0].parse::<u32>(),
                parts_d[1].parse::<u32>(),
                parts_d[2].parse::<u32>(),
                parts_t[0].parse::<u64>(),
                parts_t[1].parse::<u64>(),
                parts_t[2].parse::<u64>(),
            ) {
                let days = ymd_to_days(y, m, d);
                return Some(days * 86400 + hr * 3600 + mn * 60 + sc);
            }
        }
    }
    None
}
