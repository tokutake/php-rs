//! Native DateInterval objects. Public components live in property slots so
//! references and compound assignments use the interpreter's regular paths.
use super::{Runtime, arg_str, args};
use crate::datetime::{self, interval::Interval};
use crate::interp::{EResult, Fatal};
use crate::object::{Internal, Object};
use crate::value::Value;

const FIELDS: &[&str] = &[
    "y",
    "m",
    "d",
    "h",
    "i",
    "s",
    "f",
    "invert",
    "days",
    "from_string",
    "date_string",
];
pub fn store(obj: &Object, value: Interval) {
    for (name, v) in [
        ("y", Value::Int(value.years)),
        ("m", Value::Int(value.months)),
        ("d", Value::Int(value.days)),
        ("h", Value::Int(value.hours)),
        ("i", Value::Int(value.minutes)),
        ("s", Value::Int(value.seconds)),
        ("f", Value::Float(value.micros as f64 / 1_000_000.0)),
        ("invert", Value::Int(value.invert)),
        (
            "days",
            value.total_days.map_or(Value::Bool(false), Value::Int),
        ),
        ("from_string", Value::Bool(value.relative.is_some())),
    ] {
        obj.set(name, v);
    }
    if let Some(text) = &value.relative {
        obj.set("date_string", Value::str(text.as_str()));
    }
    if let Internal::Interval(slot) = &obj.internal {
        *slot.borrow_mut() = Some(value);
    }
}
pub fn state(obj: &Object) -> EResult<Interval> {
    let Internal::Interval(slot) = &obj.internal else {
        return Err(Fatal::type_error("Expected a DateInterval object"));
    };
    let mut value = slot.borrow().clone().ok_or_else(|| {
        Fatal::error(
            "The DateInterval object has not been correctly initialized by its constructor",
        )
    })?;
    value.years = obj.get("y").to_int();
    value.months = obj.get("m").to_int();
    value.days = obj.get("d").to_int();
    value.hours = obj.get("h").to_int();
    value.minutes = obj.get("i").to_int();
    value.seconds = obj.get("s").to_int();
    let f = obj.get("f").to_float() * 1_000_000.0;
    if !f.is_finite() || !crate::value::float_fits_int(f) {
        return Err(Fatal::new(
            "DateRangeError",
            "Interval microseconds are out of range",
        ));
    }
    if obj.get("f").to_float() != value.micros as f64 / 1_000_000.0 {
        value.micros = f as i64;
    }
    value.invert = obj.get("invert").to_int();
    Ok(value)
}
pub fn make(rt: &mut dyn Runtime, value: Interval) -> EResult<Value> {
    let obj = rt
        .new_object("DateInterval")
        .ok_or_else(|| Fatal::error("DateInterval class is unavailable"))?;
    store(&obj, value);
    Ok(Value::Object(obj))
}
pub fn method(
    rt: &mut dyn Runtime,
    obj: &Object,
    fname: &str,
    name: &str,
    vals: Vec<Value>,
) -> EResult<Value> {
    match name {
        "__construct" => {
            let [text] = args::<1, 1>(fname, vals)?;
            let text = arg_str(rt, fname, 1, "duration", &text.unwrap())?;
            let text = String::from_utf8_lossy(&text);
            let value = Interval::iso(&text).ok_or_else(|| {
                Fatal::new(
                    "DateMalformedIntervalStringException",
                    format!("Unknown or bad format ({text})"),
                )
            })?;
            store(obj, value);
            Ok(Value::Null)
        }
        "format" => {
            let [format] = args::<1, 1>(fname, vals)?;
            let format = arg_str(rt, fname, 1, "format", &format.unwrap())?;
            Ok(Value::str(state(obj)?.format(&format)))
        }
        _ => Err(Fatal::error(format!("Unknown DateInterval method {name}"))),
    }
}
pub fn from_string(rt: &mut dyn Runtime, fname: &str, vals: Vec<Value>) -> EResult<Value> {
    let [text] = args::<1, 1>(fname, vals)?;
    let text = arg_str(rt, fname, 1, "datetime", &text.unwrap())?;
    let text = String::from_utf8_lossy(&text);
    if let Some(interval) = Interval::relative(&text) {
        return make(rt, interval);
    }
    let b = datetime::DateTime {
        seconds: 0,
        micros: 0,
        zone: datetime::default_zone(),
    };
    let saved = datetime::format::last_errors();
    let parsed = !text.trim().is_empty() && datetime::parse(&text, &b).is_some();
    datetime::format::record(saved);
    let lower = text.to_ascii_lowercase();
    let relative = lower.starts_with("next ")
        || lower.starts_with("last ")
        || lower.starts_with("this ")
        || lower.starts_with("first day of ");
    if parsed && relative {
        return make(
            rt,
            Interval {
                relative: Some(text.into_owned()),
                special: true,
                ..Interval::default()
            },
        );
    }
    let message = if parsed {
        format!("String '{text}' contains non-relative elements")
    } else {
        let pos = text
            .split_whitespace()
            .scan(0, |position, word| {
                let start = text[*position..].find(word).unwrap_or(0) + *position;
                *position = start + word.len();
                Some((start, word))
            })
            .enumerate()
            .find_map(|(i, (start, word))| {
                let valid = if i % 2 == 0 {
                    word.parse::<i64>().is_ok() || matches!(word, "next" | "last" | "this")
                } else {
                    matches!(
                        word.trim_end_matches('s'),
                        "year"
                            | "month"
                            | "week"
                            | "day"
                            | "hour"
                            | "minute"
                            | "min"
                            | "second"
                            | "sec"
                    )
                };
                (!valid).then_some(start)
            })
            .unwrap_or(0);
        let ch = text[pos..].chars().next().unwrap_or(' ');
        format!(
            "Unknown or bad format ({text}) at position {pos} ({ch}): {}",
            if text.is_empty() {
                "Empty string"
            } else {
                "The timezone could not be found in the database"
            }
        )
    };
    if fname == "date_interval_create_from_date_string" {
        crate::diag::warning(format!("{fname}(): {message}"));
        Ok(Value::Bool(false))
    } else {
        Err(Fatal::new("DateMalformedIntervalStringException", message))
    }
}

pub fn write(obj: &Object, name: &str, value: &Value) -> bool {
    if !matches!(obj.internal, Internal::Interval(_)) || !FIELDS.contains(&name) {
        return false;
    }
    match name {
        "days" => {
            if let Internal::Interval(slot) = &obj.internal
                && let Some(interval) = slot.borrow_mut().as_mut()
            {
                if !interval.shadow_days {
                    crate::diag::deprecated(format!(
                        "Creation of dynamic property {}::$days is deprecated",
                        obj.class.name
                    ));
                    interval.shadow_days = true;
                    let key = crate::array::Key::from_bytes(b"days");
                    let mut props = obj.props.borrow_mut();
                    let mut reordered = crate::array::PhpArray::new();
                    reordered.insert(
                        key.clone(),
                        interval.total_days.map_or(Value::Bool(false), Value::Int),
                    );
                    for (k, v) in props.iter() {
                        if k != &key {
                            reordered.insert(k.clone(), v.clone());
                        }
                    }
                    *props = reordered;
                }
            }
        }
        "from_string" | "date_string" => {}
        "f" => {
            let f = value.to_float() * 1_000_000.0;
            let micros = f as i64;
            obj.set(name, Value::Float(micros as f64 / 1_000_000.0));
            if let Internal::Interval(slot) = &obj.internal
                && let Some(interval) = slot.borrow_mut().as_mut()
            {
                interval.micros = micros;
            }
        }
        _ => obj.set(name, Value::Int(value.to_int())),
    }
    true
}
/// Components created from a relative string remain accessible but PHP only
/// enumerates the source string, not its normalized component slots.
pub fn visible(obj: &Object, name: &str) -> bool {
    if let Internal::Interval(slot) = &obj.internal
        && let Some(value) = slot.borrow().as_ref()
    {
        return if value.relative.is_some() {
            !FIELDS.contains(&name) || matches!(name, "from_string" | "date_string")
        } else {
            name != "date_string"
        };
    }
    true
}
