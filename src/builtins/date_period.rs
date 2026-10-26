//! Lazy DatePeriod iteration. A period keeps its calendar cursor, while each
//! InternalIterator has its own recurrence index (including nested iterations).
use super::{Runtime, arg_int, arg_str, args, date, date_interval};
use crate::datetime::{self, DateTime, interval::Interval};
use crate::interp::{EResult, Fatal};
use crate::object::{Internal, Object};
use crate::value::Value;
use std::rc::Rc;

pub const PROPERTIES: &[&str] = &[
    "start",
    "current",
    "end",
    "interval",
    "recurrences",
    "include_start_date",
    "include_end_date",
];
#[derive(Debug, Clone)]
pub struct Period {
    start: DateTime,
    current: Option<DateTime>,
    end: Option<DateTime>,
    interval: Interval,
    recurrences: i64,
    requested: Option<i64>,
    include_start: bool,
    include_end: bool,
}
pub struct Cursor {
    pub owner: Rc<Object>,
    index: i64,
    initialized: bool,
}
fn malformed(message: impl Into<String>) -> Fatal {
    Fatal::new("DateMalformedPeriodStringException", message)
}
fn uninitialized() -> Fatal {
    Fatal::error("The DatePeriod object has not been correctly initialized by its constructor")
}
fn state(obj: &Object) -> EResult<Period> {
    let Internal::Period(slot) = &obj.internal else {
        return Err(uninitialized());
    };
    slot.borrow().clone().ok_or_else(uninitialized)
}
fn snapshot(template: &Rc<Object>, date: Option<&DateTime>, base_class: bool) -> Rc<Object> {
    let mut class = template.class.clone();
    if base_class {
        while !matches!(
            class.name,
            "DateTime" | "DateTimeImmutable" | "DateInterval"
        ) {
            let Some(parent) = class.parent.clone() else {
                break;
            };
            class = parent;
        }
    }
    let internal = match &template.internal {
        Internal::DateTime(value) => Internal::DateTime(std::cell::RefCell::new(
            date.cloned().or_else(|| value.borrow().clone()),
        )),
        Internal::Interval(value) => {
            Internal::Interval(std::cell::RefCell::new(value.borrow().clone()))
        }
        _ => Internal::None,
    };
    let copy = Object::new(class, internal);
    if !base_class {
        *copy.props.borrow_mut() = template.props.borrow().clone();
    } else if matches!(template.internal, Internal::Interval(_)) {
        *copy.props.borrow_mut() = template.props.borrow().clone();
    }
    copy
}
/// DatePeriod exposes fresh copies of native date and interval properties.
pub fn public_value(name: &str, value: Value) -> Value {
    if matches!(name, "start" | "current" | "end" | "interval")
        && let Value::Object(obj) = value
    {
        Value::Object(snapshot(&obj, None, false))
    } else {
        value
    }
}
pub fn property(obj: &Object, name: &str) -> Option<Value> {
    (matches!(obj.internal, Internal::Period(_)) && PROPERTIES.contains(&name))
        .then(|| public_value(name, obj.get(name)))
}
fn save(obj: &Object, period: Period, start: &Rc<Object>, interval: &Rc<Object>) {
    obj.set(
        "start",
        Value::Object(snapshot(start, Some(&period.start), false)),
    );
    obj.set("current", Value::Null);
    obj.set(
        "end",
        period.end.as_ref().map_or(Value::Null, |end| {
            Value::Object(snapshot(start, Some(end), false))
        }),
    );
    obj.set("interval", Value::Object(snapshot(interval, None, true)));
    obj.set("recurrences", Value::Int(period.recurrences));
    obj.set("include_start_date", Value::Bool(period.include_start));
    obj.set("include_end_date", Value::Bool(period.include_end));
    if let Internal::Period(slot) = &obj.internal {
        *slot.borrow_mut() = Some(period);
    }
}
fn configured(
    start: DateTime,
    interval: Interval,
    end: Option<DateTime>,
    count: Option<i64>,
    options: i64,
    fname: &str,
) -> EResult<Period> {
    if count.is_some_and(|n| !(1..2_147_483_640).contains(&n)) {
        return Err(malformed(format!(
            "{fname}(): Recurrence count must be greater or equal to 1 and lower than 2147483640"
        )));
    }
    let include_start = options & 1 == 0;
    let include_end = options & 2 != 0;
    let recurrences = count.unwrap_or(0) + i64::from(include_start) + i64::from(include_end);
    if recurrences >= 2_147_483_640 {
        return Err(Fatal::new(
            "DateMalformedStringException",
            format!(
                "{fname}(): Recurrence count must be greater or equal to 1 and lower than 2147483640 (including options)"
            ),
        ));
    }
    Ok(Period {
        start,
        current: None,
        end,
        interval,
        recurrences,
        requested: count,
        include_start,
        include_end,
    })
}
fn bad_arguments(fname: &str) -> Fatal {
    Fatal::type_error(format!(
        "{fname}() accepts (DateTimeInterface, DateInterval, int [, int]), or (DateTimeInterface, DateInterval, DateTime [, int]), or (string [, int]) as arguments"
    ))
}
fn iso(
    rt: &mut dyn Runtime,
    specification: &str,
    options: i64,
    fname: &str,
) -> EResult<(Period, Rc<Object>, Rc<Object>)> {
    let bad = || malformed(format!("Unknown or bad format ({specification})"));
    if !specification.is_ascii() || specification.len() > 65536 {
        return Err(bad());
    }
    let mut parts: Vec<_> = specification.split('/').collect();
    let count = if parts.first().is_some_and(|s| s.starts_with('R')) {
        let count = parts
            .remove(0)
            .strip_prefix('R')
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(bad)?;
        Some(count)
    } else {
        None
    };
    if parts.len() < 2 || parts.len() > 3 {
        return Err(bad());
    }
    let missing_start = parts.first().is_some_and(|part| part.starts_with('P'));
    let mut dates = Vec::new();
    let mut interval = None;
    for part in parts {
        if part.starts_with('P') {
            if interval.is_some() {
                return Err(bad());
            }
            interval = Some(Interval::iso(part).ok_or_else(bad)?);
        } else {
            // timelib's ISO recurrence grammar requires a full UTC date/time.
            let Some((date_text, clock)) = part.strip_suffix('Z').and_then(|s| s.split_once('T'))
            else {
                return Err(bad());
            };
            let normalized = if date_text.len() == 8
                && clock.len() == 6
                && date_text
                    .bytes()
                    .chain(clock.bytes())
                    .all(|b| b.is_ascii_digit())
            {
                format!(
                    "{}-{}-{}T{}:{}:{}Z",
                    &date_text[..4],
                    &date_text[4..6],
                    &date_text[6..8],
                    &clock[..2],
                    &clock[2..4],
                    &clock[4..6]
                )
            } else if date_text.len() == 10
                && clock.len() == 8
                && date_text.as_bytes()[4] == b'-'
                && date_text.as_bytes()[7] == b'-'
                && clock.as_bytes()[2] == b':'
                && clock.as_bytes()[5] == b':'
            {
                part.to_string()
            } else {
                return Err(bad());
            };
            let base = DateTime {
                seconds: 0,
                micros: 0,
                zone: datetime::Zone::load("UTC").unwrap(),
            };
            let diagnostics = datetime::format::last_errors();
            let parsed = datetime::parse(&normalized, &base);
            datetime::format::record(diagnostics);
            let mut parsed = parsed.ok_or_else(bad)?;
            parsed.zone = datetime::Zone::load("UTC").unwrap();
            dates.push(parsed);
        }
    }
    if missing_start || dates.is_empty() {
        return Err(malformed(format!(
            "{fname}(): ISO interval must contain a start date, \"{specification}\" given"
        )));
    }
    let Some(interval) = interval else {
        return Err(malformed(format!(
            "{fname}(): ISO interval must contain an interval, \"{specification}\" given"
        )));
    };
    if dates.len() == 1 && count.is_none() {
        return Err(malformed(format!(
            "{fname}(): Recurrence count must be greater or equal to 1 and lower than 2147483640"
        )));
    }
    let period = configured(
        dates[0].clone(),
        interval.clone(),
        dates.get(1).cloned(),
        count,
        options,
        fname,
    )?;
    let Value::Object(start) = date::make(rt, "DateTime", period.start.clone())? else {
        unreachable!()
    };
    let Value::Object(interval_obj) = date_interval::make(rt, interval)? else {
        unreachable!()
    };
    Ok((period, start, interval_obj))
}
pub fn factory(
    rt: &mut dyn Runtime,
    class: &str,
    fname: &str,
    values: Vec<Value>,
) -> EResult<Value> {
    let [specification, options] = args::<1, 2>(fname, values)?;
    let specification = arg_str(rt, fname, 1, "specification", &specification.unwrap())?;
    let options = match options {
        None => 0,
        Some(v) => arg_int(fname, 2, "options", &v)?,
    };
    let (period, start, interval) =
        iso(rt, &String::from_utf8_lossy(&specification), options, fname)?;
    let obj = rt.new_object(class).ok_or_else(uninitialized)?;
    save(&obj, period, &start, &interval);
    Ok(Value::Object(obj))
}
pub fn method(
    rt: &mut dyn Runtime,
    obj: &Rc<Object>,
    fname: &str,
    name: &str,
    values: Vec<Value>,
) -> EResult<Value> {
    if name == "__construct" {
        if let Some(Value::Object(start)) = values.first()
            && start.class.is_subclass_of("DateTimeInterface")
        {
            if !(3..=4).contains(&values.len()) {
                return Err(bad_arguments(fname));
            }
            let Value::Object(interval) = &values[1] else {
                return Err(bad_arguments(fname));
            };
            if !interval.class.is_subclass_of("DateInterval") {
                return Err(bad_arguments(fname));
            }
            let (end, count) = match &values[2] {
                Value::Object(end) if end.class.is_subclass_of("DateTimeInterface") => {
                    (Some(date::state(end)?), None)
                }
                Value::Object(_) | Value::Array(_) | Value::Resource(_) => {
                    return Err(bad_arguments(fname));
                }
                value => (
                    None,
                    Some(arg_int(fname, 3, "end", value).map_err(|_| bad_arguments(fname))?),
                ),
            };
            let options = values
                .get(3)
                .map_or(Ok(0), |v| arg_int(fname, 4, "options", v))?;
            let period = configured(
                date::state(start)?,
                date_interval::state(interval)?,
                end,
                count,
                options,
                fname,
            )?;
            save(obj, period, start, interval);
            return Ok(Value::Null);
        }
        if !(1..=2).contains(&values.len())
            || matches!(values.first(), Some(Value::Object(_) | Value::Array(_)))
        {
            return Err(bad_arguments(fname));
        }
        let specification = arg_str(rt, fname, 1, "start", &values[0])?;
        let options = values
            .get(1)
            .map_or(Ok(0), |v| arg_int(fname, 2, "options", v))?;
        crate::diag::deprecated(
            "Calling DatePeriod::__construct(string $isostr, int $options = 0) is deprecated, use DatePeriod::createFromISO8601String() instead",
        );
        let (period, start, interval) =
            iso(rt, &String::from_utf8_lossy(&specification), options, fname)?;
        save(obj, period, &start, &interval);
        return Ok(Value::Null);
    }
    args::<0, 0>(fname, values)?;
    let period = state(obj)?;
    match name {
        "getstartdate" => Ok(property(obj, "start").unwrap()),
        "getenddate" => Ok(property(obj, "end").unwrap()),
        "getdateinterval" => Ok(property(obj, "interval").unwrap()),
        "getrecurrences" => Ok(period.requested.map_or(Value::Null, Value::Int)),
        "getiterator" => {
            let iterator = rt
                .new_object("InternalIterator")
                .ok_or_else(uninitialized)?;
            let Internal::PeriodIterator(slot) = &iterator.internal else {
                return Err(uninitialized());
            };
            *slot.borrow_mut() = Some(Cursor {
                owner: obj.clone(),
                index: 0,
                initialized: false,
            });
            Ok(Value::Object(iterator))
        }
        _ => Err(Fatal::error(format!("Unknown DatePeriod method {name}"))),
    }
}
fn step(date: &DateTime, interval: &Interval) -> EResult<DateTime> {
    if interval.special {
        let saved = datetime::format::last_errors();
        let result = datetime::parse(interval.relative.as_deref().unwrap_or(""), date);
        datetime::format::record(saved);
        let mut next = result
            .ok_or_else(|| Fatal::new("DateRangeError", "DatePeriod date is out of range"))?;
        let source = date.civil();
        next.set_civil(datetime::Civil {
            hour: source.hour,
            minute: source.minute,
            second: source.second,
            ..next.civil()
        })
        .ok_or_else(|| Fatal::new("DateRangeError", "DatePeriod date is out of range"))?;
        next.micros = date.micros;
        return Ok(next);
    }
    // timelib preserves the source offset when the target wall clock falls
    // inside an autumn fold; a freshly parsed date may choose the other side.
    let source = date.civil();
    let range = || Fatal::new("DateRangeError", "DatePeriod date is out of range");
    let year = i64::try_from(source.year as i128 + interval.years as i128).map_err(|_| range())?;
    let month =
        i64::try_from(source.month as i128 + interval.months as i128).map_err(|_| range())?;
    let day = i64::try_from(source.day as i128 + interval.days as i128).map_err(|_| range())?;
    let micros = date.micros as i128 + interval.micros as i128;
    let local = datetime::timestamp(datetime::Civil {
        year,
        month,
        day,
        ..source
    }) + (interval.hours as i128 * 60 + interval.minutes as i128) * 60
        + interval.seconds as i128
        + micros.div_euclid(1_000_000);
    let offset = date.zone.offset(date.seconds);
    let preferred = i64::try_from(local - offset as i128)
        .ok()
        .filter(|t| date.zone.offset(*t) == offset);
    let seconds = preferred
        .or_else(|| date.zone.utc(local))
        .ok_or_else(range)?;
    Ok(DateTime {
        seconds,
        micros: micros.rem_euclid(1_000_000) as u32,
        zone: date.zone.clone(),
    })
}
fn publish(obj: &Object, date: &DateTime) -> EResult<()> {
    let Value::Object(template) = obj.get("start") else {
        return Err(uninitialized());
    };
    obj.set(
        "current",
        Value::Object(snapshot(&template, Some(date), false)),
    );
    Ok(())
}
fn reset(cursor: &mut Cursor) -> EResult<()> {
    let mut period = state(&cursor.owner)?;
    let current = if period.include_start {
        period.start.clone()
    } else {
        step(&period.start, &period.interval)?
    };
    period.current = Some(current.clone());
    if let Internal::Period(slot) = &cursor.owner.internal {
        *slot.borrow_mut() = Some(period);
    }
    cursor.index = 0;
    cursor.initialized = true;
    publish(&cursor.owner, &current)
}
fn advance(cursor: &mut Cursor) -> EResult<()> {
    let mut period = state(&cursor.owner)?;
    let previous = period.current.as_ref().ok_or_else(uninitialized)?;
    let current = step(previous, &period.interval)?;
    if period.end.is_some()
        && (current.seconds, current.micros) <= (previous.seconds, previous.micros)
    {
        return Err(Fatal::error(
            "DatePeriod iteration does not advance towards the end date",
        ));
    }
    cursor.index = cursor.index.checked_add(1).ok_or_else(|| {
        Fatal::new(
            "DateRangeError",
            "DatePeriod recurrence index is out of range",
        )
    })?;
    period.current = Some(current.clone());
    if let Internal::Period(slot) = &cursor.owner.internal {
        *slot.borrow_mut() = Some(period);
    }
    publish(&cursor.owner, &current)
}
pub fn iterator_method(
    obj: &Rc<Object>,
    fname: &str,
    method: &str,
    values: Vec<Value>,
) -> EResult<Value> {
    args::<0, 0>(fname, values)?;
    let Internal::PeriodIterator(slot) = &obj.internal else {
        return Err(uninitialized());
    };
    let mut slot = slot.borrow_mut();
    let cursor = slot.as_mut().ok_or_else(uninitialized)?;
    if method == "rewind" || !cursor.initialized {
        reset(cursor)?;
    }
    let period = state(&cursor.owner)?;
    match method {
        "rewind" => Ok(Value::Null),
        "next" => {
            advance(cursor)?;
            Ok(Value::Null)
        }
        "key" => Ok(Value::Int(cursor.index)),
        "current" => {
            let Value::Object(template) = cursor.owner.get("start") else {
                return Err(uninitialized());
            };
            Ok(Value::Object(snapshot(
                &template,
                period.current.as_ref(),
                true,
            )))
        }
        "valid" => {
            let valid = if let Some(end) = period.end {
                let current = period.current.ok_or_else(uninitialized)?;
                if period.include_end {
                    (current.seconds, current.micros) <= (end.seconds, end.micros)
                } else {
                    (current.seconds, current.micros) < (end.seconds, end.micros)
                }
            } else {
                cursor.index < period.recurrences
            };
            Ok(Value::Bool(valid))
        }
        _ => Err(Fatal::error(format!(
            "Unknown InternalIterator method {method}"
        ))),
    }
}
