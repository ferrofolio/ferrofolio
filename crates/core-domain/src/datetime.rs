// SPDX-License-Identifier: AGPL-3.0-only

//! Python's `datetime.datetime` as the backend writes and reads timestamps: with
//! microseconds and an optional fixed offset from UTC.
//!
//! `from_isoformat`, `localize` and `isoformat` follow CPython's C implementation
//! (`Modules/_datetimemodule.c`), which accepts more than ISO 8601: any single
//! character between date and time, ISO week dates, `24:00` as the next midnight,
//! offsets to the microsecond. The parity fixtures of the settings record them for
//! many inputs.

use jiff::Timestamp;
use jiff::tz::TimeZone;

const MIN_YEAR: i64 = 1;
const MAX_YEAR: i64 = 9999;
const SECONDS_PER_DAY: i64 = 86_400;
const MICROSECONDS_PER_SECOND: i64 = 1_000_000;
/// `date(1970, 1, 1).toordinal()`, in seconds.
const EPOCH: i64 = 719_163 * SECONDS_PER_DAY;
/// How far `astimezone()` looks for a second solution of a local time.
const MAX_FOLD_SECONDS: i64 = 24 * 3600;
const DAYS_BEFORE_MONTH: [i64; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const DAYS_IN_MONTH: [i64; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// A fixed offset from UTC, `datetime.timezone`: less than a day either way, to
/// the microsecond.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcOffset {
    microseconds: i64,
}

impl UtcOffset {
    pub const UTC: UtcOffset = UtcOffset { microseconds: 0 };

    /// The offset, or `None` outside the range Python's `timezone` accepts.
    pub fn from_microseconds(microseconds: i64) -> Option<Self> {
        // `unsigned_abs`, as `abs` overflows for `i64::MIN`.
        (microseconds.unsigned_abs() < (SECONDS_PER_DAY * MICROSECONDS_PER_SECOND).unsigned_abs())
            .then_some(UtcOffset { microseconds })
    }

    pub fn microseconds(self) -> i64 {
        self.microseconds
    }

    /// `_format_offset`: `+HH:MM`, with `:SS` and `.ffffff` when they are not zero.
    fn isoformat(self) -> String {
        let sign = if self.microseconds < 0 { '-' } else { '+' };
        let total = self.microseconds.abs();
        let hours = total / (3600 * MICROSECONDS_PER_SECOND);
        let minutes = total / (60 * MICROSECONDS_PER_SECOND) % 60;
        let rest = total % (60 * MICROSECONDS_PER_SECOND);
        let mut text = format!("{sign}{hours:02}:{minutes:02}");
        if rest != 0 {
            text.push_str(&format!(":{:02}", rest / MICROSECONDS_PER_SECOND));
            let microseconds = rest % MICROSECONDS_PER_SECOND;
            if microseconds != 0 {
                text.push_str(&format!(".{microseconds:06}"));
            }
        }
        text
    }
}

/// A `datetime`, naive or with a fixed offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    microsecond: i64,
    offset: Option<UtcOffset>,
}

/// A string `datetime.fromisoformat` rejects with a `ValueError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid isoformat string")]
pub struct InvalidIsoformat;

/// A conversion Python fails with an exception, with its message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DateTimeError(String);

impl DateTime {
    /// The `datetime` constructor's checks.
    fn new(
        (year, month, day): (i64, i64, i64),
        hour: i64,
        minute: i64,
        second: i64,
        microsecond: i64,
        offset: Option<UtcOffset>,
    ) -> Option<Self> {
        let valid = (MIN_YEAR..=MAX_YEAR).contains(&year)
            && (1..=12).contains(&month)
            && (1..=days_in_month(year, month)).contains(&day)
            && (0..24).contains(&hour)
            && (0..60).contains(&minute)
            && (0..60).contains(&second)
            && (0..MICROSECONDS_PER_SECOND).contains(&microsecond);
        valid.then_some(DateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            microsecond,
            offset,
        })
    }

    /// `datetime.fromisoformat(text)`.
    pub fn from_isoformat(text: &str) -> Result<Self, InvalidIsoformat> {
        // Python counts characters here, the parser below bytes of UTF-8.
        if text.chars().count() < 7 {
            return Err(InvalidIsoformat);
        }
        let bytes = Bytes(text.as_bytes());
        let length = text.len();
        let separator = find_separator(bytes, length).ok_or(InvalidIsoformat)?;
        let (mut year, mut month, mut day) = parse_date(bytes, separator)?;
        let mut time = Time::default();
        if length > separator {
            // The separator is any one character.
            let lead = bytes.at(separator);
            let width = if lead & 0x80 == 0 {
                1
            } else {
                match lead & 0xF0 {
                    0xE0 => 3,
                    0xF0 => 4,
                    _ => 2,
                }
            };
            time = parse_time(bytes, (separator + width).min(length), length)?;
        }
        let offset = match time.offset {
            None => None,
            Some(microseconds) => {
                Some(UtcOffset::from_microseconds(microseconds).ok_or(InvalidIsoformat)?)
            }
        };
        let mut hour = time.hour;
        // `24:00` is midnight of the next day, if the date exists.
        if hour == 24 && (1..=12).contains(&month) {
            let days = days_in_month(year, month);
            if day <= days {
                if time.minute != 0 || time.second != 0 || time.microsecond != 0 {
                    return Err(InvalidIsoformat);
                }
                hour = 0;
                day += 1;
                if day > days {
                    day = 1;
                    month += 1;
                    if month > 12 {
                        month = 1;
                        year += 1;
                    }
                }
            }
        }
        DateTime::new(
            (year, month, day),
            hour,
            time.minute,
            time.second,
            time.microsecond,
            offset,
        )
        .ok_or(InvalidIsoformat)
    }

    /// `datetime.now().astimezone()` for a zoned time: its wall time to the
    /// microsecond with its offset; `None` outside years 1 to 9999.
    pub fn from_zoned(zoned: &jiff::ZonedDateTime) -> Option<Self> {
        let offset = UtcOffset::from_microseconds(
            i64::from(zoned.offset().seconds()) * MICROSECONDS_PER_SECOND,
        )?;
        DateTime::new(
            (
                i64::from(zoned.year()),
                i64::from(zoned.month()),
                i64::from(zoned.day()),
            ),
            i64::from(zoned.hour()),
            i64::from(zoned.minute()),
            i64::from(zoned.second()),
            i64::from(zoned.subsec_nanosecond()) / 1000,
            Some(offset),
        )
    }

    /// The offset of an aware datetime.
    pub fn offset(&self) -> Option<UtcOffset> {
        self.offset
    }

    /// `datetime.isoformat()`.
    pub fn isoformat(&self) -> String {
        let mut text = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        if self.microsecond != 0 {
            text.push_str(&format!(".{:06}", self.microsecond));
        }
        if let Some(offset) = self.offset {
            text.push_str(&offset.isoformat());
        }
        text
    }

    /// Microseconds since the Unix epoch of an aware datetime, which orders aware
    /// datetimes as Python compares them.
    pub fn timestamp_micros(&self) -> Option<i128> {
        let offset = self.offset?;
        let seconds = utc_to_seconds(
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
        );
        Some(
            i128::from(seconds - EPOCH) * i128::from(MICROSECONDS_PER_SECOND)
                + i128::from(self.microsecond)
                - i128::from(offset.microseconds),
        )
    }

    /// `self.astimezone()` of a naive datetime, with `time_zone` as the local time
    /// zone: the same instant in local time, with the offset in force then. A
    /// local time that does not exist moves forward by the gap, one that exists
    /// twice is the earlier. An aware datetime is returned as it is.
    ///
    /// Like Python, it fails within a day of the ends of the calendar, where it
    /// looks for a second solution beyond them.
    pub fn localize(&self, time_zone: &TimeZone) -> Result<DateTime, DateTimeError> {
        if self.offset.is_some() {
            return Ok(*self);
        }
        let local = Local(time_zone);
        let seconds = local.instant_of(self)?;
        let offset = local.offset_at(seconds - EPOCH);
        let wall = utc_to_seconds(
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
        );
        // result = self - offset, as UTC.
        let utc = wall - offset;
        check_range(utc)?;
        // `local_timezone(result)`: the offset in force at that instant.
        let offset = local.offset_at(utc - EPOCH);
        let local_seconds = utc + offset;
        check_range(local_seconds)?;
        let (year, month, day) = ord_to_ymd(local_seconds.div_euclid(SECONDS_PER_DAY));
        let in_day = local_seconds.rem_euclid(SECONDS_PER_DAY);
        let offset = UtcOffset::from_microseconds(offset * MICROSECONDS_PER_SECOND)
            .ok_or_else(|| DateTimeError("offset out of range".to_owned()))?;
        DateTime::new(
            (year, month, day),
            in_day / 3600,
            in_day / 60 % 60,
            in_day % 60,
            self.microsecond,
            Some(offset),
        )
        .ok_or_else(|| DateTimeError("date value out of range".to_owned()))
    }
}

/// `OverflowError: date value out of range` of datetime arithmetic.
fn check_range(seconds: i64) -> Result<(), DateTimeError> {
    let ordinal = seconds.div_euclid(SECONDS_PER_DAY);
    if ordinal < 1 || ordinal > ymd_to_ord(MAX_YEAR, 12, 31) {
        return Err(DateTimeError("date value out of range".to_owned()));
    }
    Ok(())
}

/// The bytes of a string as C reads them: a zero byte after the end.
#[derive(Clone, Copy)]
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn at(self, index: usize) -> u8 {
        self.0.get(index).copied().unwrap_or(0)
    }

    /// `parse_digits`: `count` ASCII digits from `start`, and the position after them.
    fn digits(self, start: usize, count: usize) -> Option<(i64, usize)> {
        let mut value = 0;
        for index in start..start + count {
            let digit = self.at(index).wrapping_sub(b'0');
            if digit > 9 {
                return None;
            }
            value = value * 10 + i64::from(digit);
        }
        Some((value, start + count))
    }
}

/// `_find_isoformat_datetime_separator`: where the date ends, `None` where C
/// answers -1.
fn find_separator(bytes: Bytes<'_>, length: usize) -> Option<usize> {
    if length == 7 {
        return Some(7);
    }
    if bytes.at(4) == b'-' {
        if bytes.at(5) != b'W' {
            return Some(10);
        }
        if length > 8 && bytes.at(8) == b'-' {
            if length == 9 {
                return None;
            }
            if length > 10 && bytes.at(10).is_ascii_digit() {
                return Some(8);
            }
            return Some(10);
        }
        return Some(8);
    }
    if bytes.at(4) != b'W' {
        return Some(8);
    }
    let mut index = 7;
    while index < length && bytes.at(index).is_ascii_digit() {
        index += 1;
    }
    if index < 9 {
        return Some(index);
    }
    Some(if index % 2 == 0 { 7 } else { 8 })
}

/// `parse_isoformat_date`: the date in the first `length` bytes.
fn parse_date(bytes: Bytes<'_>, length: usize) -> Result<(i64, i64, i64), InvalidIsoformat> {
    let (year, mut position) = bytes.digits(0, 4).ok_or(InvalidIsoformat)?;
    let separated = bytes.at(position) == b'-';
    if separated {
        position += 1;
    }
    // `p++` happens only when the separator is expected.
    let separator = |position: &mut usize| {
        let character = bytes.at(*position);
        *position += 1;
        character == b'-'
    };
    if bytes.at(position) == b'W' {
        let (week, after) = bytes.digits(position + 1, 2).ok_or(InvalidIsoformat)?;
        position = after;
        let mut weekday = 1;
        if position < length {
            if separated && !separator(&mut position) {
                return Err(InvalidIsoformat);
            }
            let (day, after) = bytes.digits(position, 1).ok_or(InvalidIsoformat)?;
            weekday = day;
            position = after;
        }
        if position != length {
            return Err(InvalidIsoformat);
        }
        return iso_to_ymd(year, week, weekday).ok_or(InvalidIsoformat);
    }
    let (month, after) = bytes.digits(position, 2).ok_or(InvalidIsoformat)?;
    position = after;
    if separated && !separator(&mut position) {
        return Err(InvalidIsoformat);
    }
    let (day, after) = bytes.digits(position, 2).ok_or(InvalidIsoformat)?;
    if after != length {
        return Err(InvalidIsoformat);
    }
    Ok((year, month, day))
}

#[derive(Debug, Default)]
struct Time {
    hour: i64,
    minute: i64,
    second: i64,
    microsecond: i64,
    /// The offset in microseconds, if the time has one.
    offset: Option<i64>,
}

/// `parse_hh_mm_ss_ff` between `start` and `end`: hours, minutes, seconds,
/// microseconds, and whether the string goes on after them.
fn parse_hh_mm_ss_ff(
    bytes: Bytes<'_>,
    start: usize,
    end: usize,
) -> Result<([i64; 3], i64, bool), InvalidIsoformat> {
    let mut values = [0; 3];
    let mut position = start;
    let mut separated = true;
    for (index, value) in values.iter_mut().enumerate() {
        let (parsed, after) = bytes.digits(position, 2).ok_or(InvalidIsoformat)?;
        *value = parsed;
        let character = bytes.at(after);
        position = after + 1;
        if index == 0 {
            separated = character == b':';
        }
        if character == b'.' || character == b',' {
            if index < 2 || position >= end {
                return Err(InvalidIsoformat);
            }
            break;
        } else if position >= end {
            return Ok((values, 0, character != 0));
        } else if separated && character == b':' {
            if index == 2 {
                return Err(InvalidIsoformat);
            }
        } else if !separated {
            position -= 1;
        } else {
            return Err(InvalidIsoformat);
        }
    }
    let to_parse = end.saturating_sub(position).min(6);
    if to_parse == 0 {
        return Err(InvalidIsoformat);
    }
    let (fraction, mut position) = bytes.digits(position, to_parse).ok_or(InvalidIsoformat)?;
    let scale = 10_i64.pow(u32::try_from(6 - to_parse).unwrap_or(0));
    // Digits beyond the sixth are dropped.
    while bytes.at(position).is_ascii_digit() {
        position += 1;
    }
    Ok((values, fraction * scale, bytes.at(position) != 0))
}

/// `parse_isoformat_time` of the bytes from `start` to `end`.
fn parse_time(bytes: Bytes<'_>, start: usize, end: usize) -> Result<Time, InvalidIsoformat> {
    let mut zone = start;
    loop {
        if matches!(bytes.at(zone), b'Z' | b'+' | b'-') {
            break;
        }
        zone += 1;
        if zone >= end {
            break;
        }
    }
    let ([hour, minute, second], microsecond, more) = parse_hh_mm_ss_ff(bytes, start, zone)?;
    let mut time = Time {
        hour,
        minute,
        second,
        microsecond,
        offset: None,
    };
    if zone == end {
        return if more {
            Err(InvalidIsoformat)
        } else {
            Ok(time)
        };
    }
    if bytes.at(zone) == b'Z' {
        if bytes.at(zone + 1) != 0 {
            return Err(InvalidIsoformat);
        }
        time.offset = Some(0);
        return Ok(time);
    }
    let sign = if bytes.at(zone) == b'-' { -1 } else { 1 };
    let ([hours, minutes, seconds], microseconds, more) = parse_hh_mm_ss_ff(bytes, zone + 1, end)?;
    if more {
        return Err(InvalidIsoformat);
    }
    time.offset = Some(
        sign * ((hours * 3600 + minutes * 60 + seconds) * MICROSECONDS_PER_SECOND + microseconds),
    );
    Ok(time)
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    if month == 2 && is_leap(year) {
        return 29;
    }
    usize::try_from(month)
        .ok()
        .and_then(|month| DAYS_IN_MONTH.get(month))
        .copied()
        .unwrap_or(0)
}

fn days_before_month(year: i64, month: i64) -> i64 {
    let days = usize::try_from(month)
        .ok()
        .and_then(|month| DAYS_BEFORE_MONTH.get(month))
        .copied()
        .unwrap_or(0);
    if month > 2 && is_leap(year) {
        days + 1
    } else {
        days
    }
}

fn days_before_year(year: i64) -> i64 {
    let y = year - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

/// The ordinal of a date, 1 January of year 1 being 1.
fn ymd_to_ord(year: i64, month: i64, day: i64) -> i64 {
    days_before_year(year) + days_before_month(year, month) + day
}

/// `ord_to_ymd` for an ordinal of at least 1.
fn ord_to_ymd(ordinal: i64) -> (i64, i64, i64) {
    const DAYS_400_YEARS: i64 = 146_097;
    const DAYS_100_YEARS: i64 = 36_524;
    const DAYS_4_YEARS: i64 = 1461;
    let n = ordinal - 1;
    let n400 = n.div_euclid(DAYS_400_YEARS);
    let n = n.rem_euclid(DAYS_400_YEARS);
    let n100 = n / DAYS_100_YEARS;
    let n = n % DAYS_100_YEARS;
    let n4 = n / DAYS_4_YEARS;
    let n = n % DAYS_4_YEARS;
    let n1 = n / 365;
    let n = n % 365;
    let year = n400 * 400 + 1 + n100 * 100 + n4 * 4 + n1;
    if n1 == 4 || n100 == 4 {
        return (year - 1, 12, 31);
    }
    let mut month = (n + 50) >> 5;
    let mut preceding = days_before_month(year, month);
    if preceding > n {
        month -= 1;
        preceding -= days_in_month(year, month);
    }
    (year, month, n - preceding + 1)
}

/// `iso_to_ymd`: the date of an ISO week date, if the week and day exist.
fn iso_to_ymd(year: i64, week: i64, day: i64) -> Option<(i64, i64, i64)> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
        return None;
    }
    if week <= 0 || week >= 53 {
        // Years starting on a Thursday, and leap years starting on a Wednesday,
        // have 53 weeks.
        let first_weekday = (ymd_to_ord(year, 1, 1) + 6) % 7;
        if !(week == 53 && (first_weekday == 3 || (first_weekday == 2 && is_leap(year)))) {
            return None;
        }
    }
    if !(1..=7).contains(&day) {
        return None;
    }
    let first_day = ymd_to_ord(year, 1, 1);
    let first_weekday = (first_day + 6) % 7;
    let mut week1_monday = first_day - first_weekday;
    if first_weekday > 3 {
        week1_monday += 7;
    }
    Some(ord_to_ymd(week1_monday + (week - 1) * 7 + day - 1))
}

/// `utc_to_seconds` for a year in range: seconds since the start of ordinal 0.
fn utc_to_seconds(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    ((ymd_to_ord(year, month, day) * 24 + hour) * 60 + minute) * 60 + second
}

/// The local time zone as CPython's `astimezone()` consults `localtime()`.
struct Local<'a>(&'a TimeZone);

impl Local<'_> {
    /// The offset in seconds in force at a Unix time. jiff covers years -9999 to
    /// 9999; beyond them the offset at the nearest end applies.
    fn offset_at(&self, unix_seconds: i64) -> i64 {
        let timestamp = Timestamp::from_second(unix_seconds).unwrap_or(if unix_seconds < 0 {
            Timestamp::MIN
        } else {
            Timestamp::MAX
        });
        i64::from(self.0.to_offset(timestamp).seconds())
    }

    /// `local(u)`: the local wall time at `u` in the seconds of `utc_to_seconds`,
    /// which fails outside years 1 to 9999. Offsets and the probes of
    /// `instant_of` stay within days of the range, so years 0 and 10000 are the
    /// only ones beyond it.
    fn local(&self, u: i64) -> Result<i64, DateTimeError> {
        let wall = u + self.offset_at(u - EPOCH);
        let ordinal = wall.div_euclid(SECONDS_PER_DAY);
        let year = if ordinal < 1 {
            0
        } else if ordinal > ymd_to_ord(MAX_YEAR, 12, 31) {
            MAX_YEAR + 1
        } else {
            return Ok(wall);
        };
        Err(DateTimeError(format!(
            "year must be in {MIN_YEAR}..{MAX_YEAR}, not {year}"
        )))
    }

    /// `local_to_seconds`: the instant, in seconds of `utc_to_seconds`, at which
    /// the wall time `t` is local time; `fold` picks the later of two.
    fn to_seconds(&self, t: i64, fold: bool) -> Result<i64, DateTimeError> {
        let a = self.local(t)? - t;
        let u1 = t - a;
        let t1 = self.local(u1)?;
        let b = if t1 == t {
            let u2 = if fold {
                u1 + MAX_FOLD_SECONDS
            } else {
                u1 - MAX_FOLD_SECONDS
            };
            let b = self.local(u2)? - u2;
            if a == b {
                return Ok(u1);
            }
            b
        } else {
            t1 - u1
        };
        let u2 = t - b;
        if self.local(u2)? == t {
            return Ok(u2);
        }
        if t1 == t {
            return Ok(u1);
        }
        // Neither offset gives a solution: the wall time is in a gap.
        Ok(if fold { u1.min(u2) } else { u1.max(u2) })
    }

    /// `local_timezone_from_local` for a naive datetime with `fold=0`: the instant
    /// whose offset becomes the datetime's own.
    fn instant_of(&self, local: &DateTime) -> Result<i64, DateTimeError> {
        let t = utc_to_seconds(
            local.year,
            local.month,
            local.day,
            local.hour,
            local.minute,
            local.second,
        );
        let seconds = self.to_seconds(t, false)?;
        let other = self.to_seconds(t, true)?;
        // In a gap the earlier solution decides.
        Ok(if other < seconds { other } else { seconds })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offset_is_less_than_a_day_either_way() {
        let day = SECONDS_PER_DAY * MICROSECONDS_PER_SECOND;
        assert!(UtcOffset::from_microseconds(0).is_some());
        assert!(UtcOffset::from_microseconds(day - 1).is_some());
        assert!(UtcOffset::from_microseconds(1 - day).is_some());
        assert!(UtcOffset::from_microseconds(day).is_none());
        assert!(UtcOffset::from_microseconds(-day).is_none());
        assert!(UtcOffset::from_microseconds(i64::MAX).is_none());
        assert!(UtcOffset::from_microseconds(i64::MIN).is_none());
    }

    fn parse(text: &str) -> Option<String> {
        DateTime::from_isoformat(text)
            .ok()
            .map(|value| value.isoformat())
    }

    #[test]
    fn reads_and_writes_isoformat() {
        assert_eq!(
            parse("2026-03-01T10:00:00+01:00").as_deref(),
            Some("2026-03-01T10:00:00+01:00")
        );
        assert_eq!(parse("2026-03-01").as_deref(), Some("2026-03-01T00:00:00"));
        assert_eq!(
            parse("2026-03-01T10:00:00.5Z").as_deref(),
            Some("2026-03-01T10:00:00.500000+00:00")
        );
        assert_eq!(
            parse("2026-03-01T10:00:00-00:00:00.5").as_deref(),
            Some("2026-03-01T10:00:00-00:00:00.500000")
        );
        assert_eq!(
            parse("2026-12-31T24:00").as_deref(),
            Some("2027-01-01T00:00:00")
        );
        assert_eq!(parse("2026-W09-7").as_deref(), Some("2026-03-01T00:00:00"));
        assert_eq!(
            parse("2026-03-01T10:00:00 +01:00").as_deref(),
            Some("2026-03-01T10:00:00+01:00")
        );
        for invalid in [
            "2026-3-1",
            "2026-02-30",
            "2026-03-01T10:00z",
            "2026-03-01T10:00:00+24:00",
            "x",
        ] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn localizes_like_astimezone() {
        let zone = TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        let localized = |text: &str| {
            DateTime::from_isoformat(text)
                .unwrap()
                .localize(&zone)
                .map(|value| value.isoformat())
        };
        assert_eq!(
            localized("2026-03-01T10:00:00").unwrap(),
            "2026-03-01T10:00:00+01:00"
        );
        // The gap moves forward, the fold takes the earlier time.
        assert_eq!(
            localized("2026-03-29T02:30:00").unwrap(),
            "2026-03-29T03:30:00+02:00"
        );
        assert_eq!(
            localized("2026-10-25T02:30:00").unwrap(),
            "2026-10-25T02:30:00+02:00"
        );
        assert_eq!(
            localized("0001-01-01T01:00:00").unwrap_err().to_string(),
            "year must be in 1..9999, not 0"
        );
        assert_eq!(
            localized("9999-12-31T22:59:59").unwrap_err().to_string(),
            "year must be in 1..9999, not 10000"
        );
    }

    #[test]
    fn orders_aware_datetimes_by_instant() {
        let micros = |text: &str| DateTime::from_isoformat(text).unwrap().timestamp_micros();
        assert_eq!(
            micros("2026-03-01T10:00:00+01:00"),
            micros("2026-03-01T09:00:00Z")
        );
        assert_eq!(micros("1970-01-01T00:00:00.000001Z"), Some(1));
        assert_eq!(micros("2026-03-01T10:00:00"), None);
    }

    #[test]
    fn calendar_round_trips() {
        for ordinal in [1, 365, 366, 730_120, 3_652_059] {
            let (year, month, day) = ord_to_ymd(ordinal);
            assert_eq!(ymd_to_ord(year, month, day), ordinal);
        }
    }
}
