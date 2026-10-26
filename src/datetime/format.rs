//! Format-driven parsing and the shared date extension diagnostic state.
use super::{Civil, DateTime, Zone, civil, days, month_days, timestamp, weekday};
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    pub warnings: BTreeMap<usize, String>,
    pub errors: BTreeMap<usize, String>,
    pub warning_count: usize,
    pub error_count: usize,
}
impl Diagnostics {
    pub fn error(&mut self, position: usize, message: &str) {
        self.error_count += 1;
        self.errors.insert(position, message.into());
    }
    pub fn warning(&mut self, position: usize, message: &str) {
        self.warning_count += 1;
        self.warnings.insert(position, message.into());
    }
    pub fn is_empty(&self) -> bool {
        self.error_count == 0 && self.warning_count == 0
    }
}
thread_local! { static LAST: RefCell<Diagnostics> = RefCell::new(Diagnostics::default()); }
pub fn last_errors() -> Diagnostics {
    LAST.with(|last| last.borrow().clone())
}
pub fn record(diagnostics: Diagnostics) {
    LAST.with(|last| *last.borrow_mut() = diagnostics);
}

fn number(input: &[u8], pos: &mut usize, min: usize, max: usize) -> Option<i64> {
    let n = input[*pos..]
        .iter()
        .take(max)
        .take_while(|b| b.is_ascii_digit())
        .count();
    if n < min {
        *pos += n;
        return None;
    }
    let value = std::str::from_utf8(&input[*pos..*pos + n])
        .ok()?
        .parse()
        .ok()?;
    *pos += n;
    Some(value)
}
fn word(input: &[u8], pos: &mut usize, names: &[&str]) -> Option<usize> {
    for (i, name) in names.iter().enumerate() {
        if input
            .get(*pos..*pos + name.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(name.as_bytes()))
        {
            *pos += name.len();
            return Some(i);
        }
    }
    None
}
const MONTHS: &[&str] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: &[&str] = &[
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Parse using PHP's format controls, greedy numeric fields, and normalization.
/// Bytes are kept intact so diagnostics index the original PHP byte string.
pub fn parse(format: &[u8], input: &[u8], base: &DateTime) -> Option<DateTime> {
    let mut diagnostics = Diagnostics::default();
    if input.len() > 1_048_576 || format.len() > 1_048_576 {
        diagnostics.error(0, "The date/time could not be parsed");
        record(diagnostics);
        return None;
    }
    let mut result = base.clone();
    result.micros = 0;
    let mut c = result.civil();
    let mut seen = [false; 7]; // year, month, day, hour, minute, second, microsecond
    let epoch = Civil {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    let mut pos = 0;
    let mut fi = 0;
    let mut allow_trailing = false;
    let mut clock = false;
    let mut wanted_day = None;
    let mut failed_field = false;
    while fi < format.len() {
        let token = format[fi];
        fi += 1;
        if token == b'!' {
            c = epoch;
            result.micros = 0;
            seen.fill(true);
            clock = true;
            continue;
        }
        if token == b'|' {
            if !seen[0] {
                c.year = 1970;
            }
            if !seen[1] {
                c.month = 1;
            }
            if !seen[2] {
                c.day = 1;
            }
            if !seen[3] {
                c.hour = 0;
            }
            if !seen[4] {
                c.minute = 0;
            }
            if !seen[5] {
                c.second = 0;
            }
            if !seen[6] {
                result.micros = 0;
            }
            continue;
        }
        if token == b'+' {
            allow_trailing = true;
            continue;
        }
        if token.is_ascii_whitespace() {
            while input.get(pos).is_some_and(u8::is_ascii_whitespace) {
                pos += 1;
            }
            continue;
        }
        if pos >= input.len() {
            diagnostics.error(pos, "Not enough data available to satisfy format");
            if failed_field {
                diagnostics.error(pos, "Not enough data available to satisfy format");
            }
            break;
        }
        let start = pos;
        let field = match token {
            b'Y' | b'y' | b'X' | b'x' => Some((
                0,
                1,
                if token == b'y' {
                    2
                } else if matches!(token, b'X' | b'x') {
                    19
                } else {
                    4
                },
                "A four digit year could not be found",
            )),
            b'm' | b'n' => Some((1, 1, 2, "A two digit month could not be found")),
            b'd' | b'j' => Some((2, 1, 2, "A two digit day could not be found")),
            b'H' | b'G' => Some((3, 1, 2, "A two digit hour could not be found")),
            b'h' | b'g' => Some((3, 1, 2, "A two digit hour could not be found")),
            b'i' => Some((4, 2, 2, "A two digit minute could not be found")),
            b's' => Some((5, 2, 2, "A two digit second could not be found")),
            b'u' => Some((6, 1, 6, "A six digit microsecond could not be found")),
            b'v' => Some((6, 1, 3, "A three digit millisecond could not be found")),
            _ => None,
        };
        if let Some((field, min, max, message)) = field {
            let negative = field == 0 && matches!(token, b'X' | b'x') && input[pos] == b'-';
            if field == 0 && matches!(token, b'X' | b'x') && matches!(input[pos], b'+' | b'-') {
                pos += 1;
            }
            if let Some(mut value) = number(input, &mut pos, min, max) {
                if negative {
                    value = -value;
                }
                if token == b'y' {
                    value += if value < 70 { 2000 } else { 1900 };
                }
                if field >= 3 && !clock {
                    c.hour = 0;
                    c.minute = 0;
                    c.second = 0;
                    result.micros = 0;
                    clock = true;
                }
                seen[field] = true;
                match field {
                    0 => c.year = value,
                    1 => c.month = value,
                    2 => c.day = value,
                    3 => {
                        c.hour = value;
                        if matches!(token, b'h' | b'g') && value > 12 {
                            diagnostics.error(start, "Hour cannot be higher than 12");
                        }
                    }
                    4 => c.minute = value,
                    5 => c.second = value,
                    _ => result.micros = value as u32 * 10u32.pow(6 - (pos - start) as u32),
                }
            } else {
                diagnostics.error(start, message);
                failed_field = true;
                if pos == start {
                    while input.get(pos).is_some_and(u8::is_ascii_alphabetic) {
                        pos += 1;
                    }
                }
            }
            continue;
        }
        match token {
            b'\\' => {
                if let Some(&literal) = format.get(fi) {
                    fi += 1;
                    if input.get(pos) == Some(&literal) {
                        pos += 1;
                    } else {
                        diagnostics.error(pos, "The escaped character could not be found");
                    }
                } else {
                    diagnostics.error(pos, "The escaped character could not be found");
                }
            }
            b'F' | b'M' => {
                let short: Vec<_> = MONTHS.iter().map(|s| &s[..3]).collect();
                if let Some(m) =
                    word(input, &mut pos, MONTHS).or_else(|| word(input, &mut pos, &short))
                {
                    c.month = m as i64 + 1;
                    seen[1] = true;
                } else {
                    diagnostics.error(pos, "A textual month could not be found");
                }
            }
            b'D' | b'l' => {
                let short: Vec<_> = DAYS.iter().map(|s| &s[..3]).collect();
                if let Some(d) =
                    word(input, &mut pos, DAYS).or_else(|| word(input, &mut pos, &short))
                {
                    wanted_day = Some(d as i64);
                } else {
                    diagnostics.error(pos, "A textual day could not be found");
                }
            }
            b'S' => {
                if input.get(pos..pos + 2).is_some_and(|s| {
                    [b"st", b"nd", b"rd", b"th"]
                        .iter()
                        .any(|n| s.eq_ignore_ascii_case(*n))
                }) {
                    pos += 2;
                }
            }
            b'a' | b'A' => {
                if let Some(meridian) = word(input, &mut pos, &["am", "pm"]) {
                    c.hour = c.hour % 12 + if meridian == 1 { 12 } else { 0 };
                } else {
                    diagnostics.error(pos, "A meridian could not be found");
                }
            }
            b'z' => {
                if let Some(day) = number(input, &mut pos, 1, 3) {
                    if !seen[0] {
                        diagnostics.error(
                            start,
                            "A 'day of year' can only come after a year has been found",
                        );
                    }
                    let date = civil((days(c.year, 1, 1) + day as i128) * 86400);
                    c.year = date.year;
                    c.month = date.month;
                    c.day = date.day;
                    seen[1] = true;
                    seen[2] = true;
                } else {
                    diagnostics.error(start, "A three digit day of year could not be found");
                }
            }
            b'U' => {
                if matches!(input[pos], b'+' | b'-') {
                    pos += 1;
                }
                let digits = pos;
                while input.get(pos).is_some_and(u8::is_ascii_digit) {
                    pos += 1;
                }
                let text = std::str::from_utf8(&input[start..pos]).ok();
                if pos > digits
                    && let Some(value) = text.and_then(|s| s.parse::<i64>().ok())
                {
                    result.zone = Zone::load("+00:00").unwrap();
                    c = civil(value as i128);
                    seen.fill(true);
                    clock = true;
                } else {
                    diagnostics.error(start, "Number out of range");
                }
            }
            b'e' | b'T' | b'O' | b'P' | b'p' => {
                while input
                    .get(pos)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b"/_+-:".contains(b))
                {
                    pos += 1;
                }
                let name = std::str::from_utf8(&input[start..pos]).ok();
                if let Some(zone) = name.and_then(Zone::load) {
                    result.zone = zone;
                } else {
                    diagnostics.error(start, "The timezone could not be found in the database");
                }
            }
            b'?' => pos += 1,
            b'*' => {
                while input.get(pos).is_some_and(|b| {
                    !b.is_ascii_digit() && !b.is_ascii_whitespace() && !b";:/.,-()".contains(b)
                }) {
                    pos += 1;
                }
            }
            b'#' => {
                if b";:/.,-()".contains(&input[pos]) {
                    pos += 1;
                } else {
                    diagnostics.error(pos, "The separation symbol could not be found");
                }
            }
            _ => {
                if input.get(pos) == Some(&token) {
                    pos += 1;
                } else {
                    diagnostics.error(pos, "The format separator does not match");
                    pos += 1;
                }
            }
        }
    }
    if pos < input.len() {
        if allow_trailing {
            diagnostics.warning(pos, "Trailing data");
        } else {
            diagnostics.error(pos, "Trailing data");
        }
    }
    if c.hour > 23 || c.minute > 59 || c.second > 59 {
        diagnostics.warning(pos, "The parsed time was invalid");
    }
    if !(1..=12).contains(&c.month) || c.day < 1 || c.day > month_days(c.year, c.month) {
        diagnostics.warning(pos, "The parsed date was invalid");
    }
    if diagnostics.error_count > 0 {
        record(diagnostics);
        return None;
    }
    // U seeds the calendar fields; later fields may override them just like
    // any other token (for example `U Y` or `U e`).
    result.seconds = match result.zone.utc(timestamp(c)) {
        Some(seconds) => seconds,
        None => {
            diagnostics.error(pos, "Number out of range");
            record(diagnostics);
            return None;
        }
    };
    if let Some(wanted) = wanted_day {
        let date = result.civil();
        let delta = (wanted - weekday(date.year, date.month, date.day)).rem_euclid(7);
        let Some(t) = result.zone.utc(timestamp(Civil {
            day: date.day + delta,
            ..date
        })) else {
            diagnostics.error(pos, "Number out of range");
            record(diagnostics);
            return None;
        };
        result.seconds = t;
    }
    record(diagnostics);
    Some(result)
}
