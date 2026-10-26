//! Safe Gregorian calendar arithmetic and IANA TZif time zones. No process-global
//! TZ changes, libc calls, or subprocesses are needed by the interpreter.
pub mod format;
pub mod interval;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
struct Offset {
    seconds: i32,
    dst: bool,
    name: String,
}
#[derive(Debug, Clone)]
pub struct Zone {
    pub name: String,
    pub kind: i64,
    offsets: Vec<Offset>,
    transitions: Vec<(i64, usize)>,
    future: Option<Posix>,
}
#[derive(Debug, Clone)]
struct Rule {
    month: i64,
    week: i64,
    weekday: i64,
    seconds: i64,
}
#[derive(Debug, Clone)]
struct Posix {
    standard: Offset,
    daylight: Offset,
    start: Rule,
    end: Rule,
}
#[derive(Debug, Clone)]
pub struct DateTime {
    pub seconds: i64,
    pub micros: u32,
    pub zone: Rc<Zone>,
}
#[derive(Debug, Clone, Copy)]
pub struct Civil {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
}

thread_local! {
    static DEFAULT_ZONE: RefCell<Rc<Zone>> = RefCell::new(Zone::load("UTC").expect("UTC is built in"));
    static ZONES: RefCell<std::collections::HashMap<String, Rc<Zone>>> = RefCell::new(std::collections::HashMap::new());
}
pub fn default_zone() -> Rc<Zone> {
    DEFAULT_ZONE.with(|z| z.borrow().clone())
}
pub fn set_default_zone(zone: Rc<Zone>) {
    DEFAULT_ZONE.with(|z| *z.borrow_mut() = zone);
}
pub fn now() -> (i64, u32) {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_micros()),
        Err(e) => {
            let d = e.duration();
            if d.subsec_nanos() == 0 {
                (-(d.as_secs() as i64), 0)
            } else {
                (-(d.as_secs() as i64) - 1, 999_999 - d.subsec_micros())
            }
        }
    }
}
pub fn leap(y: i64) -> bool {
    y.rem_euclid(4) == 0 && (y.rem_euclid(100) != 0 || y.rem_euclid(400) == 0)
}
pub fn month_days(y: i64, m: i64) -> i64 {
    match m {
        2 => {
            if leap(y) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
// i128 intermediates also make the full signed 64-bit timestamp range safe.
pub fn days(y: i64, m: i64, d: i64) -> i128 {
    let month = m as i128 - 1;
    let mut y = y as i128 + month.div_euclid(12);
    let m = month.rem_euclid(12) + 1;
    y -= (m <= 2) as i128;
    let era = y.div_euclid(400);
    let yo = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    era * 146097 + yo * 365 + yo / 4 - yo / 100 + (153 * mp + 2) / 5 + d as i128 - 1 - 719468
}
pub fn civil(seconds: i128) -> Civil {
    let z = seconds.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yo = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yo + era * 400;
    let doy = doe - (365 * yo + yo / 4 - yo / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += (m <= 2) as i128;
    let clock = seconds.rem_euclid(86400) as i64;
    Civil {
        year: y as i64,
        month: m as i64,
        day: d as i64,
        hour: clock / 3600,
        minute: clock / 60 % 60,
        second: clock % 60,
    }
}
pub fn timestamp(c: Civil) -> i128 {
    days(c.year, c.month, c.day) * 86400
        + c.hour as i128 * 3600
        + c.minute as i128 * 60
        + c.second as i128
}
pub fn weekday(y: i64, m: i64, d: i64) -> i64 {
    (days(y, m, d) + 4).rem_euclid(7) as i64
}
fn fixed(seconds: i32, name: String, kind: i64) -> Rc<Zone> {
    Rc::new(Zone {
        name: name.clone(),
        kind,
        offsets: vec![Offset {
            seconds,
            dst: false,
            name,
        }],
        transitions: vec![],
        future: None,
    })
}
fn offset_text(s: &str) -> Option<i32> {
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let s = &s[1..];
    let (h, m) = if let Some((h, m)) = s.split_once(':') {
        (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?)
    } else if s.len() == 4 {
        (s[..2].parse().ok()?, s[2..].parse().ok()?)
    } else if s.len() <= 2 {
        (s.parse().ok()?, 0)
    } else {
        return None;
    };
    (h <= 99 && (0..60).contains(&m)).then_some(sign * (h * 3600 + m * 60))
}
impl Zone {
    pub fn load(name: &str) -> Option<Rc<Self>> {
        if !name.is_ascii() || name.len() > 255 {
            return None;
        }
        if let Some(z) = ZONES.with(|z| z.borrow().get(name).cloned()) {
            return Some(z);
        }
        if name.eq_ignore_ascii_case("UTC")
            || name.eq_ignore_ascii_case("GMT")
            || name.eq_ignore_ascii_case("Z")
        {
            return Some(fixed(
                0,
                name.to_ascii_uppercase(),
                if name.eq_ignore_ascii_case("UTC") {
                    3
                } else {
                    2
                },
            ));
        }
        if let Some(n) = offset_text(name) {
            return Some(fixed(n, format_offset(n, true), 1));
        }
        let abbr = match name.to_ascii_uppercase().as_str() {
            "EST" => Some(-18000),
            "EDT" => Some(-14400),
            "CST" => Some(-21600),
            "CDT" => Some(-18000),
            "MST" => Some(-25200),
            "MDT" => Some(-21600),
            "PST" => Some(-28800),
            "PDT" => Some(-25200),
            "JST" => Some(32400),
            "CET" => Some(3600),
            "CEST" => Some(7200),
            _ => None,
        };
        if let Some(n) = abbr {
            let mut z = fixed(n, name.to_ascii_uppercase(), 2);
            Rc::make_mut(&mut z).offsets[0].dst = matches!(
                name.to_ascii_uppercase().as_str(),
                "EDT" | "CDT" | "MDT" | "PDT" | "CEST"
            );
            return Some(z);
        }
        Self::identifier(name)
    }
    /// The default timezone must be a database identifier; accepting an
    /// abbreviation here would silently turn a DST zone into a fixed offset.
    pub fn identifier(name: &str) -> Option<Rc<Self>> {
        if !name.is_ascii() || name.len() > 255 {
            return None;
        }
        let cache_key = format!("id:{name}");
        if let Some(z) = ZONES.with(|z| z.borrow().get(&cache_key).cloned()) {
            return Some(z);
        }
        if name.eq_ignore_ascii_case("UTC") {
            return Some(fixed(0, name.into(), 3));
        }
        if name.split('/').any(|s| {
            s.is_empty()
                || s == "."
                || s == ".."
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
        }) {
            return None;
        }
        let mut path = std::path::PathBuf::from("/usr/share/zoneinfo");
        for part in name.split('/') {
            let exact = path.join(part);
            if exact.exists() {
                path = exact;
            } else {
                let entry =
                    std::fs::read_dir(&path)
                        .ok()?
                        .filter_map(Result::ok)
                        .find(|entry| {
                            entry
                                .file_name()
                                .to_str()
                                .is_some_and(|s| s.eq_ignore_ascii_case(part))
                        })?;
                path = entry.path();
            }
        }
        if std::fs::metadata(&path).ok()?.len() > 4 * 1024 * 1024 {
            return None;
        }
        let data = std::fs::read(path).ok()?;
        let zone = Rc::new(Self::tzif(name, &data)?);
        ZONES.with(|z| {
            z.borrow_mut().insert(cache_key, zone.clone());
        });
        Some(zone)
    }
    fn tzif(name: &str, data: &[u8]) -> Option<Self> {
        fn header(b: &[u8], p: usize) -> Option<[usize; 6]> {
            if b.get(p..p + 4)? != b"TZif" {
                return None;
            }
            let mut n = [0; 6];
            for (i, v) in n.iter_mut().enumerate() {
                *v = u32::from_be_bytes(b.get(p + 20 + i * 4..p + 24 + i * 4)?.try_into().ok()?)
                    as usize;
                if *v > 1_000_000 {
                    return None;
                }
            }
            Some(n)
        }
        fn size(n: [usize; 6], w: usize) -> usize {
            n[3] * (w + 1) + n[4] * 6 + n[5] + n[2] * (w + 4) + n[0] + n[1]
        }
        let first = header(data, 0)?;
        let wide = data[4] != 0;
        let p = if wide { 44 + size(first, 4) } else { 0 };
        let n = if wide { header(data, p)? } else { first };
        let w = if wide { 8 } else { 4 };
        let start = p + 44;
        let end = start + size(n, w);
        data.get(..end)?;
        if n[4] == 0 || n[4] > 256 {
            return None;
        }
        let mut offsets = Vec::new();
        let info = start + n[3] * (w + 1);
        let abbr = info + n[4] * 6;
        for i in 0..n[4] {
            let at = info + i * 6;
            let seconds = i32::from_be_bytes(data[at..at + 4].try_into().ok()?);
            let label = data.get(abbr + data[at + 5] as usize..abbr + n[5])?;
            let label = &label[..label.iter().position(|b| *b == 0).unwrap_or(label.len())];
            offsets.push(Offset {
                seconds,
                dst: data[at + 4] != 0,
                name: String::from_utf8_lossy(label).into_owned(),
            });
        }
        let mut transitions = Vec::new();
        for i in 0..n[3] {
            let at = start + i * w;
            let t = if wide {
                i64::from_be_bytes(data[at..at + 8].try_into().ok()?)
            } else {
                i32::from_be_bytes(data[at..at + 4].try_into().ok()?) as i64
            };
            let ty = data[start + n[3] * w + i] as usize;
            if ty >= offsets.len() || transitions.last().is_some_and(|(prev, _)| *prev >= t) {
                return None;
            }
            transitions.push((t, ty));
        }
        let future = std::str::from_utf8(&data[end..])
            .ok()
            .and_then(|s| Posix::parse(s.trim()));
        Some(Self {
            name: name.into(),
            kind: 3,
            offsets,
            transitions,
            future,
        })
    }
    fn at(&self, t: i64) -> Offset {
        if self.transitions.last().is_some_and(|(last, _)| t > *last)
            && let Some(f) = &self.future
        {
            return f.at(t);
        }
        let i = self.transitions.partition_point(|(s, _)| *s <= t);
        self.offsets[if i == 0 { 0 } else { self.transitions[i - 1].1 }].clone()
    }
    pub fn offset(&self, t: i64) -> i32 {
        self.at(t).seconds
    }
    pub fn utc(&self, local: i128) -> Option<i64> {
        let guess = local.clamp(i64::MIN as i128 + 172800, i64::MAX as i128 - 172800) as i64;
        let offsets = [-172800i64, 0, 172800].map(|delta| self.offset(guess.saturating_add(delta)));
        let mut valid = Vec::new();
        for offset in offsets {
            let Ok(t) = i64::try_from(local - offset as i128) else {
                continue;
            };
            if self.offset(t) == offset {
                valid.push(t);
            }
        }
        let preferred = local - self.offset(guess) as i128;
        valid
            .iter()
            .copied()
            .find(|t| *t as i128 == preferred)
            .or_else(|| valid.into_iter().min())
            .or_else(|| i64::try_from(local - *offsets.iter().min()? as i128).ok())
    }
}
impl Rule {
    fn parse(s: &str) -> Option<Self> {
        let (date, time) = s.split_once('/').unwrap_or((s, "2"));
        let nums: Vec<i64> = date
            .strip_prefix('M')?
            .split('.')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        if nums.len() != 3
            || !(1..=12).contains(&nums[0])
            || !(1..=5).contains(&nums[1])
            || !(0..=6).contains(&nums[2])
        {
            return None;
        }
        let seconds = posix_offset(time)? as i64;
        Some(Self {
            month: nums[0],
            week: nums[1],
            weekday: nums[2],
            seconds,
        })
    }
    fn local(&self, y: i64) -> i128 {
        let mut day =
            1 + (self.weekday - weekday(y, self.month, 1)).rem_euclid(7) + (self.week - 1) * 7;
        if day > month_days(y, self.month) {
            day -= 7;
        }
        days(y, self.month, day) * 86400 + self.seconds as i128
    }
}
fn posix_offset(s: &str) -> Option<i32> {
    let negative = s.starts_with('-');
    let fields: Vec<i32> = s
        .trim_start_matches(['+', '-'])
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if fields.is_empty()
        || fields.len() > 3
        || fields[0] > 167
        || fields.iter().skip(1).any(|v| !(0..60).contains(v))
    {
        return None;
    }
    Some(
        (fields[0] * 3600
            + fields.get(1).copied().unwrap_or(0) * 60
            + fields.get(2).copied().unwrap_or(0))
            * if negative { -1 } else { 1 },
    )
}
impl Posix {
    fn parse(s: &str) -> Option<Self> {
        fn label(s: &str) -> Option<(&str, &str)> {
            if let Some(rest) = s.strip_prefix('<') {
                let (name, rest) = rest.split_once('>')?;
                Some((name, rest))
            } else {
                let n = s.bytes().take_while(u8::is_ascii_alphabetic).count();
                if n < 3 {
                    None
                } else {
                    Some((&s[..n], &s[n..]))
                }
            }
        }
        let (standard, s) = label(s)?;
        let n = s
            .bytes()
            .take_while(|b| b.is_ascii_digit() || b"+-:".contains(b))
            .count();
        let seconds = -posix_offset(&s[..n])?;
        let (daylight, s) = label(&s[n..])?;
        let (dst, startend) = s.split_once(',')?;
        let dst = if dst.is_empty() {
            seconds + 3600
        } else {
            -posix_offset(dst)?
        };
        let (start, end) = startend.split_once(',')?;
        Some(Self {
            standard: Offset {
                seconds,
                dst: false,
                name: standard.into(),
            },
            daylight: Offset {
                seconds: dst,
                dst: true,
                name: daylight.into(),
            },
            start: Rule::parse(start)?,
            end: Rule::parse(end)?,
        })
    }
    fn at(&self, t: i64) -> Offset {
        let y = civil(t as i128 + self.standard.seconds as i128).year;
        let start = self.start.local(y) - self.standard.seconds as i128;
        let end = self.end.local(y) - self.daylight.seconds as i128;
        let dst = if start < end {
            (t as i128) >= start && (t as i128) < end
        } else {
            (t as i128) >= start || (t as i128) < end
        };
        if dst {
            self.daylight.clone()
        } else {
            self.standard.clone()
        }
    }
}
pub fn format_offset(n: i32, colon: bool) -> String {
    let sign = if n < 0 { '-' } else { '+' };
    let n = n.unsigned_abs();
    format!(
        "{sign}{:02}{}{:02}",
        n / 3600,
        if colon { ":" } else { "" },
        n / 60 % 60
    )
}
const MONTHS: [&str; 12] = [
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
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
impl DateTime {
    pub fn now(zone: Rc<Zone>) -> Self {
        let (seconds, micros) = now();
        Self {
            seconds,
            micros,
            zone,
        }
    }
    pub fn civil(&self) -> Civil {
        civil(self.seconds as i128 + self.zone.offset(self.seconds) as i128)
    }
    pub fn set_civil(&mut self, c: Civil) -> Option<()> {
        self.seconds = self.zone.utc(timestamp(c))?;
        Some(())
    }
    pub fn format_gmt(&self, format: &[u8]) -> Vec<u8> {
        let mut d = self.clone();
        let mut zone = (*d.zone).clone();
        zone.offsets[0].name = "GMT".into();
        d.zone = Rc::new(zone);
        d.format(format)
    }
    pub fn format(&self, format: &[u8]) -> Vec<u8> {
        let c = self.civil();
        let offset = self.zone.at(self.seconds);
        let wd = weekday(c.year, c.month, c.day);
        let yd = (days(c.year, c.month, c.day) - days(c.year, 1, 1)) as i64;
        let iso_wd = if wd == 0 { 7 } else { wd };
        let thursday = days(c.year, c.month, c.day) + (4 - iso_wd) as i128;
        let iso_y = civil(thursday * 86400).year;
        let iso_week = (thursday - days(iso_y, 1, 1)).div_euclid(7) + 1;
        let year = if c.year < 0 {
            format!("-{:04}", c.year.unsigned_abs())
        } else {
            format!("{:04}", c.year)
        };
        let mut out = Vec::new();
        let mut escaped = false;
        for &ch in format {
            if escaped {
                out.push(ch);
                escaped = false;
                continue;
            }
            if ch == b'\\' {
                escaped = true;
                continue;
            }
            let s = match ch {
                b'd' => format!("{:02}", c.day),
                b'j' => c.day.to_string(),
                b'D' => WEEKDAYS[wd as usize][..3].into(),
                b'l' => WEEKDAYS[wd as usize].into(),
                b'N' => iso_wd.to_string(),
                b'w' => wd.to_string(),
                b'z' => yd.to_string(),
                b'W' => format!("{iso_week:02}"),
                b'o' => iso_y.to_string(),
                b'S' => match c.day {
                    1 | 21 | 31 => "st",
                    2 | 22 => "nd",
                    3 | 23 => "rd",
                    _ => "th",
                }
                .into(),
                b'F' => MONTHS[(c.month - 1) as usize].into(),
                b'M' => MONTHS[(c.month - 1) as usize][..3].into(),
                b'm' => format!("{:02}", c.month),
                b'n' => c.month.to_string(),
                b't' => month_days(c.year, c.month).to_string(),
                b'L' => (leap(c.year) as u8).to_string(),
                b'Y' => year.clone(),
                b'y' => format!("{:02}", c.year % 100),
                b'X' => format!("{}{year}", if c.year >= 0 { "+" } else { "" }),
                b'x' => {
                    if (0..=9999).contains(&c.year) {
                        year.clone()
                    } else {
                        format!("{}{year}", if c.year >= 0 { "+" } else { "" })
                    }
                }
                b'a' => if c.hour < 12 { "am" } else { "pm" }.into(),
                b'A' => if c.hour < 12 { "AM" } else { "PM" }.into(),
                b'g' => (1 + (c.hour + 11) % 12).to_string(),
                b'h' => format!("{:02}", 1 + (c.hour + 11) % 12),
                b'G' => c.hour.to_string(),
                b'H' => format!("{:02}", c.hour),
                b'i' => format!("{:02}", c.minute),
                b's' => format!("{:02}", c.second),
                b'u' => format!("{:06}", self.micros),
                b'v' => format!("{:03}", self.micros / 1000),
                b'e' => self.zone.name.clone(),
                b'I' => (offset.dst as u8).to_string(),
                b'O' => format_offset(offset.seconds, false),
                b'P' => format_offset(offset.seconds, true),
                b'p' => {
                    if offset.seconds == 0 {
                        "Z".into()
                    } else {
                        format_offset(offset.seconds, true)
                    }
                }
                b'T' => {
                    if self.zone.kind == 1 {
                        format!("GMT{}", format_offset(offset.seconds, false))
                    } else {
                        offset.name.clone()
                    }
                }
                b'Z' => offset.seconds.to_string(),
                b'U' => self.seconds.to_string(),
                b'B' => format!(
                    "{:03}",
                    (self.seconds as i128 + 3600).rem_euclid(86400) * 1000 / 86400
                ),
                b'c' => format!(
                    "{year}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
                    c.month,
                    c.day,
                    c.hour,
                    c.minute,
                    c.second,
                    format_offset(offset.seconds, true)
                ),
                b'r' => format!(
                    "{}, {:02} {} {year} {:02}:{:02}:{:02} {}",
                    &WEEKDAYS[wd as usize][..3],
                    c.day,
                    &MONTHS[(c.month - 1) as usize][..3],
                    c.hour,
                    c.minute,
                    c.second,
                    format_offset(offset.seconds, false)
                ),
                _ => {
                    out.push(ch);
                    continue;
                }
            };
            out.extend_from_slice(s.as_bytes());
        }
        // PHP treats a final escape as a NUL byte.
        if escaped {
            out.push(0);
        }
        out
    }
}

/// The supported parser accepts ISO dates/times, Unix timestamps, common
/// English date spellings and relative calendar adjustments. All arithmetic
/// is checked; unknown syntax returns failure rather than guessing a date.
pub fn parse(input: &str, base: &DateTime) -> Option<DateTime> {
    let result = parse_inner(input, base);
    let mut diagnostics = format::Diagnostics::default();
    if result.is_none() {
        let message = if input.is_empty() {
            "Empty string"
        } else if input
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        {
            "The timezone could not be found in the database"
        } else {
            "Unexpected character"
        };
        diagnostics.error(0, message);
    } else {
        // The free-form parser normalizes invalid calendar dates too.
        let date = input.split([' ', 'T']).next().unwrap_or("");
        let parts: Vec<_> = date.split('-').collect();
        if parts.len() == 3
            && let (Ok(y), Ok(m), Ok(d)) = (
                parts[0].parse::<i64>(),
                parts[1].parse::<i64>(),
                parts[2].parse::<i64>(),
            )
        {
            if !(1..=12).contains(&m) || d < 1 || d > month_days(y, m) {
                diagnostics.warning(input.len() + 1, "The parsed date was invalid");
            }
        }
    }
    format::record(diagnostics);
    result
}
fn parse_inner(input: &str, base: &DateTime) -> Option<DateTime> {
    if !input.is_ascii() || input.len() > 65536 || input.contains('\0') {
        return None;
    }
    let mut text = input.trim().to_string();
    if let Some(t) = text.strip_prefix('@') {
        let (whole, frac) = t.split_once('.').unwrap_or((t, ""));
        let mut seconds = whole.parse::<i64>().ok()?;
        if frac.len() > 6 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut micros = if frac.is_empty() {
            0
        } else {
            frac.parse::<u32>().ok()? * 10u32.pow(6 - frac.len() as u32)
        };
        if t.starts_with('-') && micros > 0 {
            seconds = seconds.checked_sub(1)?;
            micros = 1_000_000 - micros;
        }
        return Some(DateTime {
            seconds,
            micros,
            zone: Zone::load("+00:00")?,
        });
    }
    let mut result = base.clone();
    if text.eq_ignore_ascii_case("now") || text.is_empty() {
        return Some(result);
    }
    // An explicit zone overrides the supplied/default one.
    if let Some((head, tail)) = text.rsplit_once(' ')
        && let Some(zone) = Zone::load(tail)
    {
        result.zone = zone;
        text = head.into();
    }
    if text.ends_with('Z') && text.contains(':') {
        result.zone = Zone::load("Z")?;
        text.pop();
    }
    if let Some(i) = text
        .char_indices()
        .rev()
        .find_map(|(i, c)| ((c == '+' || c == '-') && text[..i].contains(':')).then_some(i))
    {
        if let Some(offset) = offset_text(&text[i..]) {
            result.zone = fixed(offset, format_offset(offset, true), 1);
            text.truncate(i);
        }
    }
    let lower = text.to_ascii_lowercase();
    let mut c = result.civil();
    let mut words: Vec<&str> = lower.split_whitespace().collect();
    // RFC 2822 dates may start with an abbreviated weekday and comma.
    if words
        .first()
        .is_some_and(|s| s.ends_with(',') && day_name(s.trim_end_matches(',')).is_some())
    {
        words.remove(0);
    }
    let mut changed_clock = false;
    if let Some(&first) = words.first() {
        match first {
            "today" | "midnight" | "tomorrow" | "yesterday" | "noon" => {
                c.hour = if first == "noon" { 12 } else { 0 };
                c.minute = 0;
                c.second = 0;
                result.micros = 0;
                changed_clock = true;
                if first == "tomorrow" {
                    c.day = c.day.checked_add(1)?;
                } else if first == "yesterday" {
                    c.day = c.day.checked_sub(1)?;
                }
                words.remove(0);
            }
            "now" => {
                words.remove(0);
            }
            _ => {}
        }
    }
    // ISO date and optional T-separated clock, or slash-separated US date.
    if let Some(first) = words.first().copied() {
        let (date, clock) = first.split_once('t').unwrap_or((first, ""));
        let negative_year = date.starts_with('-');
        let parts: Vec<&str> = date.trim_start_matches(['-', '+']).split('-').collect();
        let slash: Vec<&str> = date.split('/').collect();
        if parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            c.year = parts[0].parse::<i64>().ok()?;
            if negative_year {
                c.year = c.year.checked_neg()?;
            } else if parts[0].len() <= 2 {
                c.year = short_year(c.year);
            }
            c.month = parts[1].parse().ok()?;
            c.day = parts[2].parse().ok()?;
            if !(0..=12).contains(&c.month) || !(0..=31).contains(&c.day) {
                return None;
            }
            c.hour = 0;
            c.minute = 0;
            c.second = 0;
            result.micros = 0;
            words.remove(0);
            if !clock.is_empty() {
                words.insert(0, clock);
            }
        } else if slash.len() == 3 && slash.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())) {
            c.month = slash[0].parse().ok()?;
            c.day = slash[1].parse().ok()?;
            c.year = short_year(slash[2].parse().ok()?);
            if !(0..=12).contains(&c.month) || !(0..=31).contains(&c.day) {
                return None;
            }
            c.hour = 0;
            c.minute = 0;
            c.second = 0;
            result.micros = 0;
            words.remove(0);
        } else if words.len() >= 3 {
            let (d, m) = if let Some(m) = month_name(words[1]) {
                (words[0].parse::<i64>().ok(), Some(m))
            } else if let Some(m) = month_name(words[0]) {
                (words[1].trim_end_matches(',').parse::<i64>().ok(), Some(m))
            } else {
                (None, None)
            };
            if let (Some(day), Some(month)) = (d, m) {
                c.day = day;
                c.month = month;
                c.year = words[2].parse().ok()?;
                if !(1..=31).contains(&c.day) {
                    return None;
                }
                c.hour = 0;
                c.minute = 0;
                c.second = 0;
                result.micros = 0;
                words.drain(..3);
            }
        }
    }
    if let Some(first) = words.first().copied()
        && first.contains(':')
    {
        let (clock, frac) = first.split_once('.').unwrap_or((first, ""));
        let clock = clock.trim_end_matches("am").trim_end_matches("pm");
        let parts: Vec<i64> = clock
            .split(':')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        if !(2..=3).contains(&parts.len())
            || !(0..=24).contains(&parts[0])
            || !(0..60).contains(&parts[1])
            || parts.get(2).is_some_and(|v| !(0..=60).contains(v))
        {
            return None;
        }
        c.hour = parts[0];
        c.minute = parts[1];
        c.second = parts.get(2).copied().unwrap_or(0);
        if frac.len() > 6 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        result.micros = if frac.is_empty() {
            0
        } else {
            frac.parse::<u32>().ok()? * 10u32.pow(6 - frac.len() as u32)
        };
        words.remove(0);
        let ampm = if first.ends_with("am") {
            Some("am")
        } else if first.ends_with("pm") {
            Some("pm")
        } else if matches!(words.first(), Some(&"am" | &"pm")) {
            Some(words.remove(0))
        } else {
            None
        };
        if let Some(ampm) = ampm {
            if !(1..=12).contains(&c.hour) {
                return None;
            }
            c.hour = c.hour % 12 + if ampm == "pm" { 12 } else { 0 };
        }
        changed_clock = true;
    }
    let ago = words.last() == Some(&"ago");
    if ago {
        words.pop();
    }
    let mut i = 0;
    while i < words.len() {
        if words[i] == "first" || words[i] == "last" {
            if words.get(i + 1) == Some(&"day") && words.get(i + 2) == Some(&"of") {
                let last = words[i] == "last";
                i += 3;
                let n = match words.get(i) {
                    Some(&"next") => 1,
                    Some(&"last") => -1,
                    Some(&"this") => 0,
                    _ => return None,
                };
                if words.get(i + 1) != Some(&"month") {
                    return None;
                }
                c.month = c.month.checked_add(n)?;
                let normalized = civil(timestamp(Civil { day: 1, ..c }));
                c.year = normalized.year;
                c.month = normalized.month;
                c.day = if last { month_days(c.year, c.month) } else { 1 };
                i += 2;
                continue;
            }
        }
        let token = words[i];
        if matches!(token, "next" | "last" | "this")
            && let Some(day) = words.get(i + 1).and_then(|s| day_name(s))
        {
            let current = weekday(c.year, c.month, c.day);
            let delta = if token == "last" {
                -((current - day - 1).rem_euclid(7) + 1)
            } else if token == "next" {
                (day - current - 1).rem_euclid(7) + 1
            } else {
                (day - current).rem_euclid(7)
            };
            c.day = c.day.checked_add(delta)?;
            if !changed_clock {
                c.hour = 0;
                c.minute = 0;
                c.second = 0;
                result.micros = 0;
            }
            i += 2;
            continue;
        }
        let n = match token {
            "next" => 1,
            "last" => -1,
            "this" => 0,
            _ => token.parse::<i64>().ok()?,
        };
        let n = if ago { n.checked_neg()? } else { n };
        let unit = words.get(i + 1)?.trim_end_matches('s');
        match unit {
            "year" => c.year = c.year.checked_add(n)?,
            "month" => c.month = c.month.checked_add(n)?,
            "week" => c.day = c.day.checked_add(n.checked_mul(7)?)?,
            "day" => c.day = c.day.checked_add(n)?,
            "hour" => c.hour = c.hour.checked_add(n)?,
            "minute" | "min" => c.minute = c.minute.checked_add(n)?,
            "second" | "sec" => c.second = c.second.checked_add(n)?,
            _ => return None,
        }
        i += 2;
    }
    result.set_civil(c)?;
    Some(result)
}
fn short_year(y: i64) -> i64 {
    if (0..70).contains(&y) {
        2000 + y
    } else if (70..100).contains(&y) {
        1900 + y
    } else {
        y
    }
}
fn month_name(s: &str) -> Option<i64> {
    MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(s) || m[..3].eq_ignore_ascii_case(s))
        .map(|i| i as i64 + 1)
}
fn day_name(s: &str) -> Option<i64> {
    WEEKDAYS
        .iter()
        .position(|d| d.eq_ignore_ascii_case(s) || d[..3].eq_ignore_ascii_case(s))
        .map(|i| i as i64)
}
