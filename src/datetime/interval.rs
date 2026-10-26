//! Calendar intervals. Date components use local calendar arithmetic, while
//! ISO duration time components count elapsed seconds across DST transitions.
use super::{Civil, DateTime, civil, days, month_days, timestamp};

#[derive(Debug, Clone, Default)]
pub struct Interval {
    pub years: i64,
    pub months: i64,
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
    pub micros: i64,
    pub invert: i64,
    pub total_days: Option<i64>,
    pub relative: Option<String>,
    pub shadow_days: bool,
    pub special: bool,
}
impl Interval {
    pub fn iso(text: &str) -> Option<Self> {
        let text = text.strip_prefix('P')?;
        if text.is_empty() || !text.is_ascii() || text.len() > 4096 {
            return None;
        }
        let mut out = Self::default();
        // ISO's alternative fixed-width duration notation.
        if text.contains('-') {
            let (date, time) = text.split_once('T').unwrap_or((text, ""));
            let fields: Vec<i64> = date
                .split('-')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            if fields.len() != 3 {
                return None;
            }
            out.years = fields[0];
            out.months = fields[1];
            out.days = fields[2];
            if !time.is_empty() {
                let fields: Vec<i64> = time
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .ok()?;
                if fields.len() != 3 {
                    return None;
                }
                out.hours = fields[0];
                out.minutes = fields[1];
                out.seconds = fields[2];
            }
            return Some(out);
        }
        let b = text.as_bytes();
        let mut p = 0;
        let mut time = false;
        let mut previous = 0;
        let mut time_fields = 0;
        while p < b.len() {
            if b[p] == b'T' {
                if time {
                    return None;
                }
                time = true;
                p += 1;
                continue;
            }
            let start = p;
            while b.get(p).is_some_and(u8::is_ascii_digit) {
                p += 1;
            }
            if p == start {
                return None;
            }
            let n = text[start..p].parse::<i64>().ok()?;
            let token = *b.get(p)?;
            p += 1;
            let order = match (time, token) {
                (false, b'Y') => 1,
                (false, b'M') => 2,
                (false, b'W') => 3,
                (false, b'D') => 4,
                (true, b'H') => 5,
                (true, b'M') => 6,
                (true, b'S') => 7,
                _ => return None,
            };
            if order <= previous {
                return None;
            }
            previous = order;
            match order {
                1 => out.years = n,
                2 => out.months = n,
                3 => out.days = out.days.checked_add(n.checked_mul(7)?)?,
                4 => out.days = out.days.checked_add(n)?,
                5 => out.hours = n,
                6 => out.minutes = n,
                _ => out.seconds = n,
            }
            if time {
                time_fields += 1;
            }
        }
        if previous == 0 || time && time_fields == 0 {
            return None;
        }
        Some(out)
    }
    pub fn relative(text: &str) -> Option<Self> {
        if !text.is_ascii() || text.len() > 65536 {
            return None;
        }
        let lower = text.to_ascii_lowercase();
        let mut words: Vec<_> = lower.split_whitespace().collect();
        if words.is_empty() {
            return None;
        }
        let initial = match words.first() {
            Some(&"tomorrow") => 1,
            Some(&"yesterday") => -1,
            Some(&"today") => 0,
            _ => i64::MIN,
        };
        if initial != i64::MIN {
            words.remove(0);
        }
        let ago = words.last() == Some(&"ago");
        if ago {
            words.pop();
        }
        let mut out = Self {
            days: if initial == i64::MIN { 0 } else { initial },
            relative: Some(text.into()),
            ..Self::default()
        };
        if words.len() % 2 != 0 {
            return None;
        }
        for pair in words.chunks(2) {
            let n = match pair[0] {
                "next" => 1,
                "last" => -1,
                "this" => 0,
                s => s.parse::<i64>().ok()?,
            };
            let n = if ago { n.checked_neg()? } else { n };
            let target = match pair[1].trim_end_matches('s') {
                "year" => &mut out.years,
                "month" => &mut out.months,
                "week" => {
                    out.days = out.days.checked_add(n.checked_mul(7)?)?;
                    continue;
                }
                "day" => &mut out.days,
                "hour" => &mut out.hours,
                "minute" | "min" => &mut out.minutes,
                "second" | "sec" => &mut out.seconds,
                _ => return None,
            };
            *target = target.checked_add(n)?;
        }
        Some(out)
    }
    pub fn apply(&self, date: &DateTime, subtract: bool) -> Option<DateTime> {
        let sign = if (self.invert != 0) ^ subtract {
            -1i128
        } else {
            1
        };
        let c = date.civil();
        let year = i64::try_from(c.year as i128 + self.years as i128 * sign).ok()?;
        let month = i64::try_from(c.month as i128 + self.months as i128 * sign).ok()?;
        let day = i64::try_from(c.day as i128 + self.days as i128 * sign).ok()?;
        let date_local = timestamp(Civil {
            year,
            month,
            day,
            ..c
        });
        let time_delta =
            ((self.hours as i128 * 60 + self.minutes as i128) * 60 + self.seconds as i128) * sign;
        let same_date = self.years == 0 && self.months == 0 && self.days == 0;
        let seconds = if same_date && (self.relative.is_none() || time_delta == 0) {
            // Preserve which occurrence of a repeated local time the source
            // represents; re-resolving its wall clock could switch DST folds.
            date.seconds as i128 + time_delta
        } else if self.relative.is_some() {
            date.zone.utc(date_local + time_delta)? as i128
        } else {
            date.zone.utc(date_local)? as i128 + time_delta
        };
        let micros = date.micros as i128 + self.micros as i128 * sign;
        Some(DateTime {
            seconds: i64::try_from(seconds + micros.div_euclid(1_000_000)).ok()?,
            micros: micros.rem_euclid(1_000_000) as u32,
            zone: date.zone.clone(),
        })
    }
    pub fn diff(a: &DateTime, b: &DateTime, absolute: bool) -> Option<Self> {
        let invert = (a.seconds, a.micros) > (b.seconds, b.micros);
        let (lo, hi) = if invert { (b, a) } else { (a, b) };
        let same_zone = a.zone.name == b.zone.name;
        let (lc, hc) = if same_zone {
            (lo.civil(), hi.civil())
        } else {
            (civil(lo.seconds as i128), civil(hi.seconds as i128))
        };
        let low_clock = lc.hour * 3600 + lc.minute * 60 + lc.second;
        let high_clock = hc.hour * 3600 + hc.minute * 60 + hc.second;
        let mut total = days(hc.year, hc.month, hc.day) - days(lc.year, lc.month, lc.day);
        if (high_clock, hi.micros) < (low_clock, lo.micros) {
            total -= 1;
        }
        total = total.max(0);
        let mut time =
            (high_clock - low_clock) as i128 * 1_000_000 + hi.micros as i128 - lo.micros as i128;
        if same_zone && total == 0 {
            time += (lo.zone.offset(lo.seconds) - hi.zone.offset(hi.seconds)) as i128 * 1_000_000;
        }
        let carry = time.div_euclid(86_400_000_000);
        time = time.rem_euclid(86_400_000_000);
        let mut day = hc.day - lc.day + carry as i64;
        let mut month = hc.month - lc.month;
        let mut year = hc.year - lc.year;
        let (mut by, mut bm) = if invert {
            (lc.year, lc.month)
        } else {
            (hc.year, hc.month - 1)
        };
        while day < 0 {
            if bm == 0 {
                by -= 1;
                bm = 12;
            }
            if bm == 13 {
                by += 1;
                bm = 1;
            }
            day += month_days(by, bm);
            month -= 1;
            bm += if invert { 1 } else { -1 };
        }
        if month < 0 {
            year -= 1;
            month += 12;
        }
        Some(Self {
            years: year,
            months: month,
            days: day,
            hours: (time / 3_600_000_000) as i64,
            minutes: (time / 60_000_000 % 60) as i64,
            seconds: (time / 1_000_000 % 60) as i64,
            micros: (time % 1_000_000) as i64,
            invert: (invert && !absolute) as i64,
            total_days: Some(i64::try_from(total).ok()?),
            relative: None,
            shadow_days: false,
            special: false,
        })
    }
    pub fn format(&self, format: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < format.len() {
            if format[i] == b'%' && i + 1 == format.len() {
                break;
            }
            if format[i] != b'%' {
                out.push(format[i]);
                i += 1;
                continue;
            }
            let token = format[i + 1];
            i += 2;
            let value = match token {
                b'y' | b'Y' => Some(self.years),
                b'm' | b'M' => Some(self.months),
                b'd' | b'D' => Some(self.days),
                b'h' | b'H' => Some(self.hours),
                b'i' | b'I' => Some(self.minutes),
                b's' | b'S' => Some(self.seconds),
                b'f' | b'F' => Some(self.micros),
                _ => None,
            };
            let s = if let Some(n) = value {
                if token == b'F' {
                    format!("{n:06}")
                } else if token.is_ascii_uppercase() {
                    format!("{n:02}")
                } else {
                    n.to_string()
                }
            } else {
                match token {
                    b'R' => if self.invert != 0 { "-" } else { "+" }.into(),
                    b'r' => if self.invert != 0 { "-" } else { "" }.into(),
                    b'a' => self
                        .total_days
                        .map_or_else(|| "(unknown)".into(), |n| n.to_string()),
                    b'%' => "%".into(),
                    _ => format!("%{}", token as char),
                }
            };
            out.extend_from_slice(s.as_bytes());
        }
        out
    }
}
