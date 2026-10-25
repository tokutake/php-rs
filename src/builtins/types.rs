//! Type predicates and conversions: is_*(), intval() and friends, var_export().

use std::rc::Rc;

use super::{arg_int, args, Runtime};
use crate::array::Key;
use crate::interp::EResult;
use crate::object::unmangle;
use crate::value::{format_float_repr, parse_numeric, with_deref, Numeric, Value};

pub const NAMES: &[&str] = &[
    "is_int", "is_integer", "is_long", "is_float", "is_double", "is_string", "is_bool", "is_null", "is_numeric",
    "is_scalar", "is_countable", "intval", "floatval", "doubleval", "boolval", "strval", "get_debug_type",
];

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    let is = |f: fn(&Value) -> bool| -> EResult<Value> {
        let [v] = args::<1, 1>(name, vals.clone())?;
        Ok(Value::Bool(f(&v.unwrap())))
    };
    Some(match lname {
        "is_int" | "is_integer" | "is_long" => is(|v| matches!(v, Value::Int(_))),
        "is_float" | "is_double" => is(|v| matches!(v, Value::Float(_))),
        "is_string" => is(|v| matches!(v, Value::Str(_))),
        "is_bool" => is(|v| matches!(v, Value::Bool(_))),
        "is_null" => is(Value::is_null),
        "is_numeric" => is(|v| match v {
            Value::Int(_) | Value::Float(_) => true,
            Value::Str(s) => parse_numeric(s).1 == Numeric::Full,
            _ => false,
        }),
        "is_scalar" => is(|v| matches!(v, Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_))),
        "is_countable" => is(|v| matches!(v, Value::Array(_))),
        "intval" => intval(name, vals),
        "floatval" | "doubleval" => args::<1, 1>(name, vals).map(|[v]| Value::Float(v.unwrap().to_float())),
        "boolval" => args::<1, 1>(name, vals).map(|[v]| Value::Bool(v.unwrap().to_bool())),
        "strval" => args::<1, 1>(name, vals).and_then(|[v]| match v.unwrap() {
            Value::Array(_) => {
                crate::diag::warning("Array to string conversion");
                Ok(Value::str("Array"))
            }
            v => rt.stringify(&v).map(Value::str),
        }),
        "get_debug_type" => args::<1, 1>(name, vals).map(|[v]| {
            Value::str(match v.unwrap() {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Int(_) => "int",
                Value::Float(_) => "float",
                Value::Str(_) => "string",
                Value::Array(_) => "array",
                Value::Object(o) => o.class.name,
                Value::Resource(r) => {
                    if r.is_open() {
                        "resource (stream)"
                    } else {
                        "resource (closed)"
                    }
                }
                Value::Ref(_) => unreachable!("arguments are dereferenced"),
            })
        }),
        _ => return None,
    })
}

fn intval(name: &str, vals: Vec<Value>) -> EResult<Value> {
    let [v, base] = args::<1, 2>(name, vals)?;
    let v = v.unwrap();
    let base = match base {
        Some(b) => arg_int("intval", 2, "base", &b)?,
        None => 10,
    };
    let Value::Str(s) = &v else { return Ok(Value::Int(v.to_int())) };
    if base == 10 {
        return Ok(Value::Int(v.to_int()));
    }
    // strtol() gives 0 for a base it does not know
    if !(base == 0 || (2..=36).contains(&base)) {
        return Ok(Value::Int(0));
    }
    // php-src lets "0b" through for bases 0 and 2, then hands the rest to strtol
    let trimmed: &[u8] = {
        let start = s.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(s.len());
        &s[start..]
    };
    if (base == 0 || base == 2) && trimmed.len() > 2 {
        let offset = usize::from(matches!(trimmed[0], b'-' | b'+'));
        if trimmed[offset] == b'0' && matches!(trimmed.get(offset + 1), Some(b'b' | b'B')) {
            let mut bin = trimmed[..offset].to_vec();
            bin.extend_from_slice(&trimmed[offset + 2..]);
            return Ok(Value::Int(strtol(&bin, 2)));
        }
    }
    Ok(Value::Int(strtol(s, base as u32)))
}

/// C's strtol: leading whitespace and a sign, base 0 detects 0x / 0 prefixes,
/// stops at the first invalid digit and saturates on overflow
pub fn strtol(s: &[u8], mut base: u32) -> i64 {
    let mut i = s.iter().position(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)).unwrap_or(s.len());
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let hex_prefix = s.get(i) == Some(&b'0')
        && matches!(s.get(i + 1), Some(b'x' | b'X'))
        && s.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
    if (base == 0 || base == 16) && hex_prefix {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if s.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    let mut acc: i128 = 0;
    let limit = if neg { -(i64::MIN as i128) } else { i64::MAX as i128 };
    while let Some(d) = s.get(i).and_then(|c| (*c as char).to_digit(base)) {
        acc = (acc * base as i128 + d as i128).min(limit);
        i += 1;
    }
    if neg { (-acc) as i64 } else { acc as i64 }
}

/// var_export() output
pub fn var_export(out: &mut Vec<u8>, v: &Value, level: usize) {
    var_export_at(out, v, level, &mut Vec::new());
}

fn export_str(out: &mut Vec<u8>, s: &[u8]) {
    out.push(b'\'');
    for &c in s {
        match c {
            b'\'' | b'\\' => {
                out.push(b'\\');
                out.push(c);
            }
            0 => out.extend_from_slice(b"' . \"\\0\" . '"),
            c => out.push(c),
        }
    }
    out.push(b'\'');
}

fn spaces(out: &mut Vec<u8>, n: usize) {
    out.extend(std::iter::repeat_n(b' ', n));
}

fn var_export_at(out: &mut Vec<u8>, v: &Value, level: usize, visiting: &mut Vec<usize>) {
    match v {
        Value::Ref(r) => with_deref(r, |inner| var_export_at(out, inner, level, visiting)),
        // Resources have no literal form
        Value::Null | Value::Resource(_) => out.extend_from_slice(b"NULL"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        // PHP_INT_MIN as a literal would be a float
        Value::Int(i64::MIN) => out.extend_from_slice(b"-9223372036854775807-1"),
        Value::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Value::Float(f) => {
            let s = format_float_repr(*f);
            out.extend_from_slice(s.as_bytes());
            if f.is_finite() && !s.contains('.') {
                out.extend_from_slice(b".0");
            }
        }
        Value::Str(s) => export_str(out, s),
        Value::Array(arr) => {
            let addr = Rc::as_ptr(arr) as usize;
            if visiting.contains(&addr) {
                out.extend_from_slice(b"NULL");
                crate::diag::warning("var_export does not handle circular references");
                return;
            }
            visiting.push(addr);
            if level > 1 {
                out.push(b'\n');
                spaces(out, level - 1);
            }
            out.extend_from_slice(b"array (\n");
            for (k, v) in arr.iter() {
                spaces(out, level + 1);
                match k {
                    Key::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
                    Key::Str(s) => export_str(out, s),
                }
                out.extend_from_slice(b" => ");
                var_export_at(out, v, level + 2, visiting);
                out.extend_from_slice(b",\n");
            }
            visiting.pop();
            if level > 1 {
                spaces(out, level - 1);
            }
            out.push(b')');
        }
        Value::Object(o) => {
            let addr = Rc::as_ptr(o) as usize;
            if visiting.contains(&addr) {
                out.extend_from_slice(b"NULL");
                crate::diag::warning("var_export does not handle circular references");
                return;
            }
            visiting.push(addr);
            if level > 1 {
                out.push(b'\n');
                spaces(out, level - 1);
            }
            let std_class = o.class.name == "stdClass";
            if std_class {
                out.extend_from_slice(b"(object) array(\n");
            } else {
                out.push(b'\\');
                out.extend_from_slice(o.class.name.as_bytes());
                if o.class.is_enum {
                    out.extend_from_slice(b"::");
                    out.extend_from_slice(&o.get("name").to_bytes());
                    visiting.pop();
                    return;
                }
                out.extend_from_slice(b"::__set_state(array(\n");
            }
            for (k, v) in o.ordered_props() {
                spaces(out, level + 2);
                match &k {
                    Key::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
                    Key::Str(_) => {
                        let (name, _, _) = unmangle(&k);
                        export_str(out, name.as_bytes());
                    }
                }
                out.extend_from_slice(b" => ");
                var_export_at(out, &v, level + 2, visiting);
                out.extend_from_slice(b",\n");
            }
            visiting.pop();
            if level > 1 {
                spaces(out, level - 1);
            }
            out.extend_from_slice(if std_class { b")" } else { b"))" });
        }
    }
}
