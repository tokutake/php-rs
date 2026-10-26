//! Date/time functions and the native DateTime, DateTimeImmutable and
//! DateTimeZone methods. They share the same calendar/parser implementation.
use super::{Runtime, arg_int, arg_str, args};
use crate::array::{Key, PhpArray};
use crate::datetime::{self, Civil, DateTime, Zone};
use crate::diag;
use crate::interp::{EResult, Fatal};
use crate::object::{Internal, Object};
use crate::value::Value;
use std::rc::Rc;

pub const NAMES: &[&str] = &[
    "time",
    "microtime",
    "gettimeofday",
    "date",
    "gmdate",
    "idate",
    "strtotime",
    "mktime",
    "gmmktime",
    "checkdate",
    "getdate",
    "localtime",
    "date_default_timezone_get",
    "date_default_timezone_set",
    "date_add",
    "date_sub",
    "date_diff",
    "date_interval_create_from_date_string",
    "date_interval_format",
    "date_get_last_errors",
    "date_create_from_format",
    "date_create_immutable_from_format",
    "date_create",
    "date_create_immutable",
    "date_format",
    "date_modify",
    "date_timestamp_get",
    "date_timestamp_set",
    "date_timezone_get",
    "date_timezone_set",
    "date_offset_get",
    "date_date_set",
    "date_time_set",
    "date_isodate_set",
    "timezone_open",
    "timezone_name_get",
    "timezone_offset_get",
];
const FORMATS: &[(&str, &str)] = &[
    ("ATOM", "Y-m-d\\TH:i:sP"),
    ("COOKIE", "l, d-M-Y H:i:s T"),
    ("ISO8601", "Y-m-d\\TH:i:sO"),
    ("ISO8601_EXPANDED", "X-m-d\\TH:i:sP"),
    ("RFC822", "D, d M y H:i:s O"),
    ("RFC850", "l, d-M-y H:i:s T"),
    ("RFC1036", "D, d M y H:i:s O"),
    ("RFC1123", "D, d M Y H:i:s O"),
    ("RFC7231", "D, d M Y H:i:s \\G\\M\\T"),
    ("RFC2822", "D, d M Y H:i:s O"),
    ("RFC3339", "Y-m-d\\TH:i:sP"),
    ("RFC3339_EXTENDED", "Y-m-d\\TH:i:s.vP"),
    ("RSS", "D, d M Y H:i:s O"),
    ("W3C", "Y-m-d\\TH:i:sP"),
];
pub fn constant(name: &str) -> Option<Value> {
    FORMATS
        .iter()
        .find(|(n, _)| name.strip_prefix("DATE_") == Some(*n))
        .map(|(_, f)| Value::str(*f))
}
pub fn class_constants() -> impl Iterator<Item = (&'static str, Value)> {
    FORMATS.iter().map(|(n, f)| (*n, Value::str(*f)))
}
fn string(rt: &mut dyn Runtime, f: &str, n: usize, p: &str, v: Value) -> EResult<String> {
    Ok(String::from_utf8_lossy(&arg_str(rt, f, n, p, &v)?).into_owned())
}
fn nullable_int(f: &str, n: usize, p: &str, v: Option<Value>, default: i64) -> EResult<i64> {
    match v {
        None | Some(Value::Null) => Ok(default),
        Some(v) => arg_int(f, n, p, &v),
    }
}
pub fn last_errors() -> Value {
    let errors = datetime::format::last_errors();
    if errors.is_empty() {
        return Value::Bool(false);
    }
    let map = |values: std::collections::BTreeMap<usize, String>| {
        Value::Array(Rc::new(
            values
                .into_iter()
                .map(|(pos, msg)| (Key::Int(pos as i64), Value::str(msg)))
                .collect(),
        ))
    };
    let mut array = PhpArray::new();
    for (name, value) in [
        ("warning_count", Value::Int(errors.warning_count as i64)),
        ("warnings", map(errors.warnings)),
        ("error_count", Value::Int(errors.error_count as i64)),
        ("errors", map(errors.errors)),
    ] {
        array.insert(Key::from_bytes(name.as_bytes()), value);
    }
    Value::Array(Rc::new(array))
}
pub fn factory(
    rt: &mut dyn Runtime,
    class: &str,
    fname: &str,
    method: &str,
    vals: Vec<Value>,
) -> EResult<Value> {
    if method == "getlasterrors" {
        args::<0, 0>(fname, vals)?;
        return Ok(last_errors());
    }
    let [format, text, zone] = args::<2, 3>(fname, vals)?;
    let format = arg_str(rt, fname, 1, "format", &format.unwrap())?;
    let text = arg_str(rt, fname, 2, "datetime", &text.unwrap())?;
    if text.contains(&0) {
        return Err(Fatal::value_error(format!(
            "{fname}(): Argument #2 ($datetime) must not contain any null bytes"
        )));
    }
    let mut b = base();
    if let Some(z) = zone
        && !matches!(z, Value::Null)
    {
        b.zone = zone_arg(fname, 3, "timezone", &z)?;
    }
    match datetime::format::parse(&format, &text, &b) {
        Some(date) => make(rt, class, date),
        None => Ok(Value::Bool(false)),
    }
}
fn base() -> DateTime {
    DateTime::now(datetime::default_zone())
}
fn out_of_range() -> Fatal {
    Fatal::new("DateRangeError", "Epoch doesn't fit in a PHP integer")
}
pub(super) fn state(obj: &Object) -> EResult<DateTime> {
    match &obj.internal {
        Internal::DateTime(d) => d.borrow().clone().ok_or_else(|| {
            Fatal::error(
                "The DateTime object has not been correctly initialized by its constructor",
            )
        }),
        _ => Err(Fatal::type_error("Expected a DateTimeInterface object")),
    }
}
fn zone(obj: &Object) -> EResult<Rc<Zone>> {
    match &obj.internal {
        Internal::TimeZone(z) => z.borrow().clone().ok_or_else(|| {
            Fatal::error(
                "The DateTimeZone object has not been correctly initialized by its constructor",
            )
        }),
        _ => Err(Fatal::type_error("Expected a DateTimeZone object")),
    }
}
fn zone_arg(f: &str, n: usize, p: &str, v: &Value) -> EResult<Rc<Zone>> {
    if let Value::Object(o) = v
        && o.class.is_subclass_of("DateTimeZone")
    {
        return zone(o);
    }
    Err(Fatal::type_error(format!(
        "{f}(): Argument #{n} (${p}) must be of type DateTimeZone, {} given",
        v.value_name()
    )))
}
fn object_arg(f: &str, n: usize, p: &str, v: Value, class: &str) -> EResult<Rc<Object>> {
    match v {
        Value::Object(o) if o.class.is_subclass_of(class) => Ok(o),
        v => Err(Fatal::type_error(format!(
            "{f}(): Argument #{n} (${p}) must be of type {class}, {} given",
            v.value_name()
        ))),
    }
}
fn store(obj: &Object, value: DateTime) {
    if let Internal::DateTime(d) = &obj.internal {
        *d.borrow_mut() = Some(value);
    }
}
pub(super) fn make(rt: &mut dyn Runtime, class: &str, value: DateTime) -> EResult<Value> {
    let obj = rt
        .new_object(class)
        .ok_or_else(|| Fatal::error("DateTime class is unavailable"))?;
    store(&obj, value);
    Ok(Value::Object(obj))
}
fn make_zone(rt: &mut dyn Runtime, value: Rc<Zone>) -> EResult<Value> {
    let obj = rt
        .new_object("DateTimeZone")
        .ok_or_else(|| Fatal::error("DateTimeZone class is unavailable"))?;
    if let Internal::TimeZone(z) = &obj.internal {
        *z.borrow_mut() = Some(value);
    }
    Ok(Value::Object(obj))
}
fn parsed(text: &str, b: &DateTime) -> EResult<DateTime> {
    datetime::parse(text, b).ok_or_else(|| {
        let (pos, ch, reason) = if text.is_empty() {
            (0, ' ', "Empty string")
        } else {
            let ch = text.chars().next().unwrap_or(' ');
            (
                0,
                ch,
                if ch.is_ascii_alphabetic() {
                    "The timezone could not be found in the database"
                } else {
                    "Unexpected character"
                },
            )
        };
        Fatal::new(
            "DateMalformedStringException",
            format!("Failed to parse time string ({text}) at position {pos} ({ch}): {reason}"),
        )
    })
}
fn date_array(d: &DateTime) -> Value {
    let c = d.civil();
    let wd = datetime::weekday(c.year, c.month, c.day);
    let values = [
        ("seconds", Value::Int(c.second)),
        ("minutes", Value::Int(c.minute)),
        ("hours", Value::Int(c.hour)),
        ("mday", Value::Int(c.day)),
        ("wday", Value::Int(wd)),
        ("mon", Value::Int(c.month)),
        ("year", Value::Int(c.year)),
        (
            "yday",
            Value::Int(
                (datetime::days(c.year, c.month, c.day) - datetime::days(c.year, 1, 1)) as i64,
            ),
        ),
        ("weekday", Value::str(d.format(b"l"))),
        ("month", Value::str(d.format(b"F"))),
    ];
    let mut a = PhpArray::new();
    for (k, v) in values {
        a.insert(Key::from_bytes(k.as_bytes()), v);
    }
    a.insert(Key::Int(0), Value::Int(d.seconds));
    Value::Array(Rc::new(a))
}
pub fn call(
    rt: &mut dyn Runtime,
    name: &str,
    lname: &str,
    vals: Vec<Value>,
) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}
fn arity(name: &str, min: usize, max: usize, given: usize) -> EResult<()> {
    if given >= min && given <= max {
        return Ok(());
    }
    let (kind, n) = if min == max {
        ("exactly", min)
    } else if given < min {
        ("at least", min)
    } else {
        ("at most", max)
    };
    Err(Fatal::new(
        "ArgumentCountError",
        format!(
            "{name}() expects {kind} {n} argument{}, {given} given",
            if n == 1 { "" } else { "s" }
        ),
    ))
}
fn procedural_error(mut e: Fatal, name: &str) -> Fatal {
    let prefix = format!("{name}(): Argument #");
    if let Some(tail) = e.msg.strip_prefix(&prefix) {
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        if let Ok(n) = tail[..digits].parse::<usize>() {
            e.msg = format!("{prefix}{}{}", n + 1, &tail[digits..]);
        }
    }
    e
}
fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "date_interval_create_from_date_string" => {
            super::date_interval::from_string(rt, name, vals)
        }
        "date_interval_format" => {
            arity(name, 2, 2, vals.len())?;
            let mut vals = vals;
            let obj = object_arg(name, 1, "object", vals.remove(0), "DateInterval")?;
            super::date_interval::method(rt, &obj, name, "format", vals)
                .map_err(|e| procedural_error(e, name))
        }
        "date_get_last_errors" => {
            args::<0, 0>(name, vals)?;
            Ok(last_errors())
        }
        "date_create_from_format" | "date_create_immutable_from_format" => factory(
            rt,
            if lname == "date_create_from_format" {
                "DateTime"
            } else {
                "DateTimeImmutable"
            },
            name,
            "createfromformat",
            vals,
        ),
        "time" => {
            args::<0, 0>(name, vals)?;
            Ok(Value::Int(datetime::now().0))
        }
        "microtime" | "gettimeofday" => {
            let [as_float] = args::<0, 1>(name, vals)?;
            let as_float = match as_float {
                None => false,
                Some(v) => super::arg_bool(lname, 1, "as_float", &v)?,
            };
            let (seconds, micros) = datetime::now();
            if as_float {
                return Ok(Value::Float(seconds as f64 + micros as f64 / 1_000_000.0));
            }
            if lname == "microtime" {
                return Ok(Value::str(format!("0.{micros:06}00 {seconds}")));
            }
            let mut a = PhpArray::new();
            for (k, v) in [
                ("sec", seconds),
                ("usec", micros as i64),
                ("minuteswest", 0),
                ("dsttime", 0),
            ] {
                a.insert(Key::from_bytes(k.as_bytes()), Value::Int(v));
            }
            Ok(Value::Array(Rc::new(a)))
        }
        "date_default_timezone_get" => {
            args::<0, 0>(name, vals)?;
            Ok(Value::str(datetime::default_zone().name.clone()))
        }
        "date_default_timezone_set" => {
            let [z] = args::<1, 1>(name, vals)?;
            let z = string(rt, lname, 1, "timezoneId", z.unwrap())?;
            match Zone::identifier(&z) {
                Some(z) => {
                    datetime::set_default_zone(z);
                    Ok(Value::Bool(true))
                }
                None => {
                    diag::notice(format!(
                        "date_default_timezone_set(): Timezone ID '{z}' is invalid"
                    ));
                    Ok(Value::Bool(false))
                }
            }
        }
        "date" | "gmdate" | "idate" => {
            let [fmt, t] = args::<1, 2>(name, vals)?;
            let fmt = arg_str(rt, lname, 1, "format", &fmt.unwrap())?;
            let t = nullable_int(lname, 2, "timestamp", t, datetime::now().0)?;
            let z = if lname == "gmdate" {
                Zone::load("UTC").unwrap()
            } else {
                datetime::default_zone()
            };
            let d = DateTime {
                seconds: t,
                micros: 0,
                zone: z,
            };
            if lname != "idate" {
                if lname == "gmdate" {
                    return Ok(Value::str(d.format_gmt(&fmt)));
                }
                return Ok(Value::str(d.format(&fmt)));
            }
            if fmt.len() != 1 {
                diag::warning("idate(): idate format is one char");
                return Ok(Value::Bool(false));
            }
            let ch = fmt[0];
            let mapped = match ch {
                b'B' | b'd' | b'h' | b'H' | b'i' | b'I' | b'L' | b'm' | b'N' | b'o' | b's'
                | b't' | b'U' | b'w' | b'W' | b'y' | b'Y' | b'z' | b'Z' => ch,
                _ => {
                    diag::warning("idate(): Unrecognized date format token");
                    return Ok(Value::Bool(false));
                }
            };
            let s = d.format(&[mapped]);
            Ok(Value::Int(
                std::str::from_utf8(&s)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
            ))
        }
        "strtotime" => {
            let [text, t] = args::<1, 2>(name, vals)?;
            let text = string(rt, lname, 1, "datetime", text.unwrap())?;
            let mut b = base();
            b.seconds = nullable_int(lname, 2, "baseTimestamp", t, b.seconds)?;
            b.micros = 0;
            if text.trim().is_empty() {
                return Ok(Value::Bool(false));
            }
            Ok(datetime::parse(&text, &b).map_or(Value::Bool(false), |d| Value::Int(d.seconds)))
        }
        "checkdate" => {
            let [m, d, y] = args::<3, 3>(name, vals)?;
            let m = arg_int(lname, 1, "month", &m.unwrap())?;
            let d = arg_int(lname, 2, "day", &d.unwrap())?;
            let y = arg_int(lname, 3, "year", &y.unwrap())?;
            Ok(Value::Bool(
                (1..=32767).contains(&y)
                    && (1..=12).contains(&m)
                    && (1..=datetime::month_days(y, m)).contains(&d),
            ))
        }
        "mktime" | "gmmktime" => {
            let [h, m, s, mo, d, y] = args::<1, 6>(name, vals)?;
            let mut b = base();
            if lname == "gmmktime" {
                b.zone = Zone::load("UTC").unwrap();
            }
            let c = b.civil();
            let mut year = nullable_int(lname, 6, "year", y, c.year)?;
            if (0..70).contains(&year) {
                year += 2000;
            } else if (70..100).contains(&year) {
                year += 1900;
            }
            let c = Civil {
                year,
                month: nullable_int(lname, 4, "month", mo, c.month)?,
                day: nullable_int(lname, 5, "day", d, c.day)?,
                hour: nullable_int(lname, 1, "hour", h, c.hour)?,
                minute: nullable_int(lname, 2, "minute", m, c.minute)?,
                second: nullable_int(lname, 3, "second", s, c.second)?,
            };
            Ok(b.zone
                .utc(datetime::timestamp(c))
                .map_or(Value::Bool(false), Value::Int))
        }
        "getdate" | "localtime" => {
            if lname == "getdate" {
                let [t] = args::<0, 1>(name, vals)?;
                let mut d = base();
                d.seconds = nullable_int(lname, 1, "timestamp", t, d.seconds)?;
                return Ok(date_array(&d));
            }
            let [t, assoc] = args::<0, 2>(name, vals)?;
            let mut d = base();
            d.seconds = nullable_int(lname, 1, "timestamp", t, d.seconds)?;
            let assoc = match assoc {
                None => false,
                Some(v) => super::arg_bool(lname, 2, "associative", &v)?,
            };
            let c = d.civil();
            let fields = [
                ("tm_sec", c.second),
                ("tm_min", c.minute),
                ("tm_hour", c.hour),
                ("tm_mday", c.day),
                ("tm_mon", c.month - 1),
                ("tm_year", c.year - 1900),
                ("tm_wday", datetime::weekday(c.year, c.month, c.day)),
                (
                    "tm_yday",
                    (datetime::days(c.year, c.month, c.day) - datetime::days(c.year, 1, 1)) as i64,
                ),
                (
                    "tm_isdst",
                    String::from_utf8_lossy(&d.format(b"I"))
                        .parse()
                        .unwrap_or(0),
                ),
            ];
            let mut a = PhpArray::new();
            for (i, (k, v)) in fields.iter().enumerate() {
                a.insert(
                    if assoc {
                        Key::from_bytes(k.as_bytes())
                    } else {
                        Key::Int(i as i64)
                    },
                    Value::Int(*v),
                );
            }
            Ok(Value::Array(Rc::new(a)))
        }
        "date_create" | "date_create_immutable" => {
            let [text, z] = args::<0, 2>(name, vals)?;
            let text = match text {
                None => "now".into(),
                Some(v) => string(rt, lname, 1, "datetime", v)?,
            };
            let mut b = base();
            if let Some(v) = z
                && !matches!(v, Value::Null)
            {
                b.zone = zone_arg(lname, 2, "timezone", &v)?;
            }
            match datetime::parse(&text, &b) {
                Some(d) => make(
                    rt,
                    if lname == "date_create" {
                        "DateTime"
                    } else {
                        "DateTimeImmutable"
                    },
                    d,
                ),
                None => Ok(Value::Bool(false)),
            }
        }
        "timezone_open" => {
            let [z] = args::<1, 1>(name, vals)?;
            let z = string(rt, lname, 1, "timezone", z.unwrap())?;
            match Zone::load(&z) {
                Some(z) => make_zone(rt, z),
                None => {
                    diag::warning(format!("timezone_open(): Unknown or bad timezone ({z})"));
                    Ok(Value::Bool(false))
                }
            }
        }
        "timezone_name_get" | "timezone_offset_get" => {
            let n = if lname == "timezone_name_get" { 1 } else { 2 };
            arity(name, n, n, vals.len())?;
            let mut vals = vals;
            let obj = object_arg(lname, 1, "object", vals.remove(0), "DateTimeZone")?;
            timezone_method(
                rt,
                &obj,
                name,
                if lname == "timezone_name_get" {
                    "getname"
                } else {
                    "getoffset"
                },
                vals,
            )
            .map_err(|e| procedural_error(e, name))
        }
        _ => {
            let method = match lname {
                "date_add" => "add",
                "date_sub" => "sub",
                "date_diff" => "diff",
                "date_format" => "format",
                "date_modify" => "modify",
                "date_timestamp_get" => "gettimestamp",
                "date_timestamp_set" => "settimestamp",
                "date_timezone_get" => "gettimezone",
                "date_timezone_set" => "settimezone",
                "date_offset_get" => "getoffset",
                "date_date_set" => "setdate",
                "date_time_set" => "settime",
                "date_isodate_set" => "setisodate",
                _ => return Err(Fatal::error(format!("Call to undefined function {name}()"))),
            };
            let (min, max) = match method {
                "format" | "modify" | "settimestamp" | "settimezone" => (2, 2),
                "add" | "sub" => (2, 2),
                "diff" => (2, 3),
                "setdate" => (4, 4),
                "settime" => (3, 5),
                "setisodate" => (3, 4),
                _ => (1, 1),
            };
            arity(name, min, max, vals.len())?;
            let mut vals = vals;
            let class = if matches!(
                method,
                "format" | "gettimestamp" | "gettimezone" | "getoffset" | "diff"
            ) {
                "DateTimeInterface"
            } else {
                "DateTime"
            };
            let obj = object_arg(lname, 1, "object", vals.remove(0), class)?;
            method_call(rt, &obj, name, method, vals).map_err(|e| procedural_error(e, name))
        }
    }
}

pub fn method_call(
    rt: &mut dyn Runtime,
    obj: &Rc<Object>,
    fname: &str,
    method: &str,
    vals: Vec<Value>,
) -> EResult<Value> {
    if method == "__construct" {
        let [text, z] = args::<0, 2>(fname, vals)?;
        let text = match text {
            None => "now".into(),
            Some(v) => string(rt, fname, 1, "datetime", v)?,
        };
        let mut b = base();
        if let Some(v) = z
            && !matches!(v, Value::Null)
        {
            b.zone = zone_arg(fname, 2, "timezone", &v)?;
        }
        let d = parsed(&text, &b)?;
        store(obj, d);
        return Ok(Value::Null);
    }
    let mut d = state(obj)?;
    match method {
        "add" | "sub" => {
            let [interval] = args::<1, 1>(fname, vals)?;
            let obj = object_arg(fname, 1, "interval", interval.unwrap(), "DateInterval")?;
            let interval = super::date_interval::state(&obj)?;
            // Relative strings can contain weekday rules in addition to fields.
            d = if interval.special {
                let text = interval.relative.as_deref().unwrap_or("");
                if method == "sub" {
                    return Err(Fatal::new(
                        "DateInvalidOperationException",
                        format!(
                            "{fname}(): Only non-special relative time specifications are supported for subtraction"
                        ),
                    ));
                }
                let mut next = datetime::parse(text, &d).ok_or_else(out_of_range)?;
                let source = d.civil();
                next.set_civil(Civil {
                    hour: source.hour,
                    minute: source.minute,
                    second: source.second,
                    ..next.civil()
                })
                .ok_or_else(out_of_range)?;
                next.micros = d.micros;
                next
            } else {
                interval
                    .apply(&d, method == "sub")
                    .ok_or_else(out_of_range)?
            };
        }
        "diff" => {
            let [target, absolute] = args::<1, 2>(fname, vals)?;
            let target = object_arg(
                fname,
                1,
                "targetObject",
                target.unwrap(),
                "DateTimeInterface",
            )?;
            let absolute = match absolute {
                None => false,
                Some(v) => super::arg_bool(fname, 2, "absolute", &v)?,
            };
            let interval = datetime::interval::Interval::diff(&d, &state(&target)?, absolute)
                .ok_or_else(out_of_range)?;
            return super::date_interval::make(rt, interval);
        }
        "format" => {
            let [f] = args::<1, 1>(fname, vals)?;
            let f = arg_str(rt, fname, 1, "format", &f.unwrap())?;
            return Ok(Value::str(d.format(&f)));
        }
        "gettimestamp" => {
            args::<0, 0>(fname, vals)?;
            return Ok(Value::Int(d.seconds));
        }
        "getoffset" => {
            args::<0, 0>(fname, vals)?;
            return Ok(Value::Int(d.zone.offset(d.seconds) as i64));
        }
        "gettimezone" => {
            args::<0, 0>(fname, vals)?;
            return make_zone(rt, d.zone);
        }
        "getmicrosecond" => {
            args::<0, 0>(fname, vals)?;
            return Ok(Value::Int(d.micros as i64));
        }
        "modify" => {
            let [text] = args::<1, 1>(fname, vals)?;
            let text = string(rt, fname, 1, "modifier", text.unwrap())?;
            match parsed(&text, &d) {
                Ok(value) => d = value,
                Err(e) if fname == "date_modify" => {
                    diag::warning(format!("date_modify(): {}", e.msg));
                    return Ok(Value::Bool(false));
                }
                Err(mut e) => {
                    e.msg = format!("{fname}(): {}", e.msg);
                    return Err(e);
                }
            }
        }
        "settimestamp" => {
            let [t] = args::<1, 1>(fname, vals)?;
            d.seconds = arg_int(fname, 1, "timestamp", &t.unwrap())?;
            d.micros = 0;
        }
        "settimezone" => {
            let [z] = args::<1, 1>(fname, vals)?;
            d.zone = zone_arg(fname, 1, "timezone", &z.unwrap())?;
        }
        "setdate" => {
            let [y, m, day] = args::<3, 3>(fname, vals)?;
            let c = Civil {
                year: arg_int(fname, 1, "year", &y.unwrap())?,
                month: arg_int(fname, 2, "month", &m.unwrap())?,
                day: arg_int(fname, 3, "day", &day.unwrap())?,
                ..d.civil()
            };
            d.set_civil(c).ok_or_else(out_of_range)?;
        }
        "settime" => {
            let [h, m, s, u] = args::<2, 4>(fname, vals)?;
            let micros = match u {
                None => 0,
                Some(v) => arg_int(fname, 4, "microsecond", &v)?,
            };
            let second = match s {
                None => 0,
                Some(v) => arg_int(fname, 3, "second", &v)?,
            };
            let c = Civil {
                hour: arg_int(fname, 1, "hour", &h.unwrap())?,
                minute: arg_int(fname, 2, "minute", &m.unwrap())?,
                second,
                ..d.civil()
            };
            let local = datetime::timestamp(c) + micros.div_euclid(1_000_000) as i128;
            d.seconds = d.zone.utc(local).ok_or_else(out_of_range)?;
            d.micros = micros.rem_euclid(1_000_000) as u32;
        }
        "setmicrosecond" => {
            let [u] = args::<1, 1>(fname, vals)?;
            let u = arg_int(fname, 1, "microsecond", &u.unwrap())?;
            if !(0..=999999).contains(&u) {
                return Err(Fatal::new(
                    "DateRangeError",
                    format!(
                        "{fname}(): Argument #1 ($microsecond) must be between 0 and 999999, {u} given"
                    ),
                ));
            }
            d.micros = u as u32;
        }
        "setisodate" => {
            let [y, w, day] = args::<2, 3>(fname, vals)?;
            let y = arg_int(fname, 1, "year", &y.unwrap())?;
            let w = arg_int(fname, 2, "week", &w.unwrap())?;
            let day = match day {
                None => 1,
                Some(v) => arg_int(fname, 3, "dayOfWeek", &v)?,
            };
            let wd = datetime::weekday(y, 1, 4);
            let wd = if wd == 0 { 7 } else { wd };
            let local =
                (datetime::days(y, 1, 4) - (wd - 1) as i128 + (w as i128 - 1) * 7 + day as i128
                    - 1)
                    * 86400;
            let date = datetime::civil(local);
            let c = Civil {
                year: date.year,
                month: date.month,
                day: date.day,
                ..d.civil()
            };
            d.set_civil(c).ok_or_else(out_of_range)?;
        }
        _ => return Err(Fatal::error(format!("Unknown DateTime method {method}"))),
    }
    if obj.class.is_subclass_of("DateTimeImmutable") {
        let copy = rt
            .new_object(obj.class.name)
            .ok_or_else(|| Fatal::error("DateTime class is unavailable"))?;
        *copy.props.borrow_mut() = obj.props.borrow().clone();
        store(&copy, d);
        Ok(Value::Object(copy))
    } else {
        store(obj, d);
        Ok(Value::Object(obj.clone()))
    }
}
pub fn timezone_method(
    rt: &mut dyn Runtime,
    obj: &Rc<Object>,
    fname: &str,
    method: &str,
    vals: Vec<Value>,
) -> EResult<Value> {
    if method == "__construct" {
        let [z] = args::<1, 1>(fname, vals)?;
        let name = string(rt, fname, 1, "timezone", z.unwrap())?;
        let value = Zone::load(&name).ok_or_else(|| {
            Fatal::new(
                "DateInvalidTimeZoneException",
                format!("{fname}(): Unknown or bad timezone ({name})"),
            )
        })?;
        if let Internal::TimeZone(z) = &obj.internal {
            *z.borrow_mut() = Some(value);
        }
        return Ok(Value::Null);
    }
    let z = zone(obj)?;
    match method {
        "getname" => {
            args::<0, 0>(fname, vals)?;
            Ok(Value::str(z.name.clone()))
        }
        "getoffset" => {
            let [d] = args::<1, 1>(fname, vals)?;
            let d = object_arg(fname, 1, "datetime", d.unwrap(), "DateTimeInterface")?;
            Ok(Value::Int(z.offset(state(&d)?.seconds) as i64))
        }
        _ => {
            let _ = rt;
            Err(Fatal::error(format!(
                "Unknown DateTimeZone method {method}"
            )))
        }
    }
}
/// Virtual debug properties are generated from native state, never trusted as
/// storage. Assigning user properties cannot change the underlying timestamp.
pub fn debug_properties(obj: &Object) -> Option<Vec<(&'static str, Value)>> {
    match &obj.internal {
        Internal::DateTime(d) => d.borrow().as_ref().map(|d| {
            vec![
                ("date", Value::str(d.format(b"Y-m-d H:i:s.u"))),
                ("timezone_type", Value::Int(d.zone.kind)),
                ("timezone", Value::str(d.zone.name.clone())),
            ]
        }),
        Internal::TimeZone(z) => z.borrow().as_ref().map(|z| {
            vec![
                ("timezone_type", Value::Int(z.kind)),
                ("timezone", Value::str(z.name.clone())),
            ]
        }),
        _ => None,
    }
}
