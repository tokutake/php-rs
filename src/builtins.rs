//! Built-in functions.

use std::cmp::Ordering;
use std::rc::Rc;

use crate::array::{Key, PhpArray};
use crate::diag;
use crate::object::{self, Visibility};
use crate::interp::{to_key, to_str, EResult, Fatal};
use crate::value::{
    float_fits_int, float_to_int_implicit, format_float_repr, loose_cmp, parse_numeric, with_deref, Num, Numeric, Value,
};

/// Built-ins whose first parameter is by reference
pub const BY_REF: &[&str] = &["sort", "rsort", "array_push", "array_pop"];

pub const ARRAY_FILTER_USE_BOTH: i64 = 1;
pub const ARRAY_FILTER_USE_KEY: i64 = 2;

/// Built-ins implemented in this module or by the interpreter
const NAMES: &[&str] = &[
    "strlen", "str_repeat", "intdiv", "count", "sizeof", "is_array", "implode", "join", "explode",
    "array_keys", "array_values", "array_reverse", "array_merge", "array_sum", "in_array",
    "array_key_exists", "key_exists", "range", "sort", "rsort", "array_push", "array_pop",
    "usort", "uasort", "uksort", "print", "var_dump", "print_r", "func_get_args", "func_num_args",
    "function_exists", "is_callable", "call_user_func", "call_user_func_array", "array_map",
    "array_filter", "get_class", "is_object", "gettype",
];

pub fn exists(lname: &str) -> bool {
    NAMES.contains(&lname)
}

/// PHP refuses ranges with more elements than this (HT_MAX_SIZE on 64-bit)
const MAX_ARRAY_SIZE: u64 = 1 << 30;

pub fn arg_count(name: &str, min: usize, given: usize) -> Fatal {
    Fatal::new(
        "ArgumentCountError",
        format!("{name}() expects at least {min} argument{}, {given} given", if min == 1 { "" } else { "s" }),
    )
}

fn args<const MIN: usize, const MAX: usize>(name: &str, vals: Vec<Value>) -> EResult<[Option<Value>; MAX]> {
    let given = vals.len();
    if given < MIN || given > MAX {
        let (kind, n) = if MIN == MAX {
            ("exactly", MIN)
        } else if given < MIN {
            ("at least", MIN)
        } else {
            ("at most", MAX)
        };
        return Err(Fatal::new(
            "ArgumentCountError",
            format!("{name}() expects {kind} {n} argument{}, {given} given", if n == 1 { "" } else { "s" }),
        ));
    }
    let mut it = vals.into_iter();
    Ok(std::array::from_fn(|_| it.next()))
}

fn expect_array(fname: &str, n: usize, param: &str, v: &Value) -> EResult<Rc<PhpArray>> {
    match v {
        Value::Array(a) => Ok(a.clone()),
        other => Err(Fatal::type_error(format!(
            "{fname}(): Argument #{n} (${param}) must be of type array, {} given",
            other.type_name()
        ))),
    }
}

fn param_type_error(fname: &str, n: usize, param: &str, ty: &str, v: &Value) -> Fatal {
    Fatal::type_error(format!("{fname}(): Argument #{n} (${param}) must be of type {ty}, {} given", v.type_name()))
}

fn null_param(fname: &str, n: usize, param: &str, ty: &str) {
    diag::deprecated(format!("{fname}(): Passing null to parameter #{n} (${param}) of type {ty} is deprecated"));
}

/// Coerces an argument to an `int` parameter (non-strict mode rules)
pub fn arg_int(fname: &str, n: usize, param: &str, v: &Value) -> EResult<i64> {
    let float = |f: f64| {
        if f.is_finite() && float_fits_int(f) {
            Ok(float_to_int_implicit(f, v))
        } else {
            Err(param_type_error(fname, n, param, "int", v))
        }
    };
    match v {
        Value::Int(i) => Ok(*i),
        Value::Bool(b) => Ok(*b as i64),
        Value::Float(f) => float(*f),
        Value::Null => {
            null_param(fname, n, param, "int");
            Ok(0)
        }
        // Only fully numeric strings are accepted; "5 apples" is a TypeError
        Value::Str(s) => match parse_numeric(s) {
            (Num::Int(i), Numeric::Full) => Ok(i),
            (Num::Float(f), Numeric::Full) => float(f),
            _ => Err(param_type_error(fname, n, param, "int", v)),
        },
        Value::Array(_) | Value::Object(_) => Err(param_type_error(fname, n, param, "int", v)),
        Value::Ref(r) => with_deref(r, |inner| arg_int(fname, n, param, inner)),
    }
}

/// Coerces an argument to a `string` parameter
pub fn arg_str(fname: &str, n: usize, param: &str, v: &Value) -> EResult<Vec<u8>> {
    match v {
        Value::Null => {
            null_param(fname, n, param, "string");
            Ok(Vec::new())
        }
        // Stringable objects convert; others are a type error
        Value::Object(o) => object::to_string(o).ok_or_else(|| param_type_error(fname, n, param, "string", v)),
        Value::Array(_) => Err(param_type_error(fname, n, param, "string", v)),
        Value::Ref(r) => with_deref(r, |inner| arg_str(fname, n, param, inner)),
        _ => Ok(v.to_bytes()),
    }
}

/// Coerces an argument to a `bool` parameter
fn arg_bool(fname: &str, n: usize, param: &str, v: &Value) -> EResult<bool> {
    match v {
        Value::Null => {
            null_param(fname, n, param, "bool");
            Ok(false)
        }
        Value::Array(_) | Value::Object(_) => Err(param_type_error(fname, n, param, "bool", v)),
        _ => Ok(v.to_bool()),
    }
}

fn list(values: impl IntoIterator<Item = Value>) -> Value {
    Value::Array(Rc::new(PhpArray::from_values(values)))
}

pub fn call(name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "strlen" => {
            let [s] = args::<1, 1>(name, vals)?;
            Ok(Value::Int(arg_str(lname, 1, "string", &s.unwrap())?.len() as i64))
        }
        "str_repeat" => {
            let [s, n] = args::<2, 2>(name, vals)?;
            let s = arg_str(lname, 1, "string", &s.unwrap())?;
            let n = arg_int(lname, 2, "times", &n.unwrap())?;
            if n < 0 {
                return Err(Fatal::value_error("str_repeat(): Argument #2 ($times) must be greater than or equal to 0"));
            }
            // checked so a huge count is an error instead of a capacity overflow panic
            let len = s.len().checked_mul(n as usize).filter(|&l| l <= isize::MAX as usize);
            if len.is_none() {
                return Err(Fatal::error("Possible integer overflow in memory allocation"));
            }
            Ok(Value::str(s.repeat(n as usize)))
        }
        "intdiv" => {
            let [a, b] = args::<2, 2>(name, vals)?;
            let a = arg_int(lname, 1, "num1", &a.unwrap())?;
            let b = arg_int(lname, 2, "num2", &b.unwrap())?;
            if b == 0 {
                return Err(Fatal::new("DivisionByZeroError", "Division by zero"));
            }
            a.checked_div(b)
                .map(Value::Int)
                .ok_or_else(|| Fatal::new("ArithmeticError", "Division of PHP_INT_MIN by -1 is not an integer"))
        }
        "count" | "sizeof" => {
            let [v, _mode] = args::<1, 2>(name, vals)?;
            match v.unwrap() {
                Value::Array(a) => Ok(Value::Int(a.len() as i64)),
                other => Err(Fatal::type_error(format!(
                    "{lname}(): Argument #1 ($value) must be of type Countable|array, {} given",
                    other.type_name()
                ))),
            }
        }
        "get_class" => {
            let [v] = args::<1, 1>(name, vals)?;
            match v.unwrap() {
                Value::Object(o) => Ok(Value::str(o.class.name)),
                other => Err(Fatal::type_error(format!(
                    "get_class(): Argument #1 ($object) must be of type object, {} given",
                    other.type_name()
                ))),
            }
        }
        "is_object" => {
            let [v] = args::<1, 1>(name, vals)?;
            Ok(Value::Bool(matches!(v.unwrap(), Value::Object(_))))
        }
        "gettype" => {
            let [v] = args::<1, 1>(name, vals)?;
            Ok(Value::str(match v.unwrap() {
                Value::Null => "NULL",
                Value::Bool(_) => "boolean",
                Value::Int(_) => "integer",
                Value::Float(_) => "double",
                Value::Str(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
                Value::Ref(_) => unreachable!("arguments are dereferenced"),
            }))
        }
        "is_array" => {
            let [v] = args::<1, 1>(name, vals)?;
            Ok(Value::Bool(matches!(v.unwrap(), Value::Array(_))))
        }
        "implode" | "join" => {
            let [a, b] = args::<1, 2>(name, vals)?;
            let (sep, arr) = match (a.unwrap(), b) {
                (Value::Array(arr), None) => (Vec::new(), arr),
                (sep, Some(Value::Array(arr))) => (arg_str(lname, 1, "separator", &sep)?, arr),
                (Value::Array(arr), Some(sep)) => (arg_str(lname, 2, "array", &sep)?, arr),
                (_, None) => {
                    return Err(Fatal::type_error(format!(
                        "{lname}(): Argument #1 ($array) must be of type array, string given"
                    )))
                }
                (_, Some(other)) => {
                    return Err(Fatal::type_error(format!(
                        "{lname}(): Argument #2 ($array) must be of type ?array, {} given",
                        other.type_name()
                    )))
                }
            };
            let mut out = Vec::new();
            for (i, v) in arr.values().enumerate() {
                if i > 0 {
                    out.extend_from_slice(&sep);
                }
                out.extend_from_slice(&to_str(v)?);
            }
            Ok(Value::str(out))
        }
        "explode" => {
            let [sep, s, limit] = args::<2, 3>(name, vals)?;
            let sep = arg_str(lname, 1, "separator", &sep.unwrap())?;
            let s = arg_str(lname, 2, "string", &s.unwrap())?;
            let limit = match limit {
                Some(l) => arg_int(lname, 3, "limit", &l)?,
                None => i64::MAX,
            };
            if sep.is_empty() {
                return Err(Fatal::value_error("explode(): Argument #1 ($separator) must not be empty"));
            }
            let mut parts: Vec<&[u8]> = split_bytes(&s, &sep);
            if limit > 0 && parts.len() as u64 > limit as u64 {
                // The last element keeps the rest of the string
                let keep = limit as usize - 1;
                let rest_start = parts[..keep].iter().map(|p| p.len() + sep.len()).sum::<usize>();
                parts.truncate(keep);
                parts.push(&s[rest_start..]);
            } else if limit < 0 {
                let drop = limit.unsigned_abs().min(parts.len() as u64) as usize;
                parts.truncate(parts.len() - drop);
            } else if limit == 0 {
                parts = vec![&s[..]];
            }
            Ok(list(parts.into_iter().map(Value::str)))
        }
        "array_keys" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            Ok(list(arr.keys().map(Key::to_value)))
        }
        "array_values" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            Ok(list(arr.values().cloned()))
        }
        "array_reverse" => {
            let [a, preserve] = args::<1, 2>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let preserve = match preserve {
                Some(p) => arg_bool(lname, 2, "preserve_keys", &p)?,
                None => false,
            };
            let mut out = PhpArray::new();
            for (k, v) in arr.iter().rev() {
                match k {
                    Key::Int(_) if !preserve => {
                        out.push(v.clone()).map_err(|_| Fatal::error("array index overflow"))?;
                    }
                    _ => out.insert(k.clone(), v.clone()),
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_merge" => {
            let mut out = PhpArray::new();
            for (i, v) in vals.iter().enumerate() {
                let arr = expect_array(lname, i + 1, "arrays", v)?;
                for (k, v) in arr.iter() {
                    match k {
                        // Int keys are renumbered, string keys overwrite
                        Key::Int(_) => {
                            out.push(v.clone()).map_err(|_| Fatal::error("array index overflow"))?;
                        }
                        Key::Str(_) => out.insert(k.clone(), v.clone()),
                    }
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_sum" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let mut sum = Value::Int(0);
            for v in arr.values() {
                // array_sum skips what `+` would reject, with a warning instead of a TypeError
                match crate::value::add(&sum, v) {
                    Ok(s) => sum = s,
                    Err(_) => diag::warning(format!("array_sum(): Addition is not supported on type {}", v.type_name())),
                }
            }
            Ok(sum)
        }
        "in_array" => {
            let [needle, hay, strict] = args::<2, 3>(name, vals)?;
            let needle = needle.unwrap();
            let hay = expect_array(lname, 2, "haystack", &hay.unwrap())?;
            let strict = match strict {
                Some(s) => arg_bool(lname, 3, "strict", &s)?,
                None => false,
            };
            Ok(Value::Bool(hay.values().any(|v| if strict { v.strict_eq(&needle) } else { v.loose_eq(&needle) })))
        }
        "array_key_exists" | "key_exists" => {
            let [k, a] = args::<2, 2>(name, vals)?;
            let arr = expect_array(lname, 2, "array", &a.unwrap())?;
            let key = match k.unwrap() {
                v @ (Value::Int(_) | Value::Str(_) | Value::Null | Value::Bool(_) | Value::Float(_)) => to_key(&v, "array")?,
                other => {
                    return Err(Fatal::type_error(format!("Cannot access offset of type {} on array", other.type_name())))
                }
            };
            Ok(Value::Bool(arr.contains_key(&key)))
        }
        "range" => {
            let [start, end, step] = args::<2, 3>(name, vals)?;
            range(start.unwrap(), end.unwrap(), step.unwrap_or(Value::Int(1)))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}

pub fn call_by_ref(lname: &str, target: &mut Value, vals: Vec<Value>) -> EResult<Value> {
    let Value::Array(arr) = target else {
        return Err(Fatal::type_error(format!(
            "{lname}(): Argument #1 ($array) must be of type array, {} given",
            target.type_name()
        )));
    };
    match lname {
        "sort" | "rsort" => {
            let mut values: Vec<Value> = arr.values().cloned().collect();
            let reverse = lname == "rsort";
            // Uncomparable pairs count as "greater", as zend_compare reports 1 for them
            merge_sort(&mut values, |a, b| {
                let ord = loose_cmp(a, b).unwrap_or(Ordering::Greater);
                if reverse { ord.reverse() } else { ord }
            });
            *arr = Rc::new(PhpArray::from_values(values));
            Ok(Value::Bool(true))
        }
        "array_push" => {
            let arr = Rc::make_mut(arr);
            for v in vals {
                arr.push(v).map_err(|_| {
                    Fatal::error("Cannot add element to the array as the next element is already occupied")
                })?;
            }
            Ok(Value::Int(arr.len() as i64))
        }
        "array_pop" => Ok(Rc::make_mut(arr).pop().unwrap_or(Value::Null)),
        _ => unreachable!("{lname} is not a by-ref builtin"),
    }
}

fn split_bytes<'a>(s: &'a [u8], sep: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + sep.len() <= s.len() {
        if &s[i..i + sep.len()] == sep {
            parts.push(&s[start..i]);
            i += sep.len();
            start = i;
        } else {
            i += 1;
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Stable merge sort that tolerates inconsistent comparators. std's sort_by may
/// panic when the comparison is not a total order, and PHP's loose comparison
/// is not one (e.g. "abc" < "abd", "abd" == 0, 0 < "abc" ...)
fn merge_sort<T: Clone>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> Ordering) {
    let _ = try_merge_sort(v, |a, b| Ok(cmp(a, b)));
}

/// merge_sort with a comparator that can fail (a user callback that throws)
pub fn try_merge_sort<T: Clone>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> EResult<Ordering>) -> EResult<()> {
    fn go<T: Clone>(v: &mut [T], buf: &mut Vec<T>, cmp: &mut impl FnMut(&T, &T) -> EResult<Ordering>) -> EResult<()> {
        let n = v.len();
        if n <= 1 {
            return Ok(());
        }
        let mid = n / 2;
        go(&mut v[..mid], buf, cmp)?;
        go(&mut v[mid..], buf, cmp)?;
        buf.clear();
        let (mut i, mut j) = (0, mid);
        while i < mid && j < n {
            if cmp(&v[j], &v[i])? == Ordering::Less {
                buf.push(v[j].clone());
                j += 1;
            } else {
                buf.push(v[i].clone());
                i += 1;
            }
        }
        buf.extend_from_slice(&v[i..mid]);
        buf.extend_from_slice(&v[j..n]);
        v.clone_from_slice(buf);
        Ok(())
    }
    let mut buf = Vec::with_capacity(v.len());
    go(v, &mut buf, &mut cmp)
}

fn range(start: Value, end: Value, step: Value) -> EResult<Value> {
    let range_err = |msg: &str| Fatal::value_error(format!("range(): {msg}"));
    // Single-byte strings give a character range
    if let (Value::Str(a), Value::Str(b)) = (&start, &end)
        && a.len() == 1 && b.len() == 1 && !a[0].is_ascii_digit() && !b[0].is_ascii_digit() {
            let step = step.to_int().unsigned_abs();
            if step == 0 {
                return Err(range_err("Argument #3 ($step) cannot be 0"));
            }
            let (a, b) = (a[0], b[0]);
            let step = step.min(255) as usize;
            let chars: Vec<u8> = if a <= b {
                (a..=b).step_by(step).collect()
            } else {
                (b..=a).rev().step_by(step).collect()
            };
            return Ok(list(chars.into_iter().map(|c| Value::str(vec![c]))));
        }
    let (s, e, st) = (start.to_num(), end.to_num(), step.to_num());
    if let (Num::Int(s), Num::Int(e), Num::Int(st)) = (s, e, st) {
        let step = st.unsigned_abs();
        if step == 0 {
            return Err(range_err("Argument #3 ($step) cannot be 0"));
        }
        // i128 so that e.g. range(PHP_INT_MIN, PHP_INT_MAX) cannot overflow the size computation
        let span = (e as i128 - s as i128).unsigned_abs();
        let size = span / step as u128 + 1;
        if size > MAX_ARRAY_SIZE as u128 {
            return Err(Fatal::value_error(format!(
                "The supplied range exceeds the maximum array size by {} elements: start={s}, end={e}, step={step}. Calculated size: {}. Maximum size: {MAX_ARRAY_SIZE}.",
                size - MAX_ARRAY_SIZE as u128,
                size.min(i64::MAX as u128),
            )));
        }
        let dir: i128 = if e >= s { 1 } else { -1 };
        return Ok(list((0..size as i128).map(|i| Value::Int((s as i128 + dir * i * step as i128) as i64))));
    }
    let (s, e, step) = (s.to_f64(), e.to_f64(), st.to_f64().abs());
    if step == 0.0 || !step.is_finite() {
        return Err(range_err("Argument #3 ($step) cannot be 0"));
    }
    if !s.is_finite() || !e.is_finite() {
        return Err(range_err("Argument #1 ($start) must be a finite number, INF provided"));
    }
    let size = ((e - s).abs() / step).floor() + 1.0;
    if size > MAX_ARRAY_SIZE as f64 {
        return Err(Fatal::value_error("The supplied range exceeds the maximum array size"));
    }
    let dir = if e >= s { 1.0 } else { -1.0 };
    Ok(list((0..size as u64).map(|i| Value::Float(s + dir * i as f64 * step))))
}

pub fn var_dump(out: &mut Vec<u8>, v: &Value, level: usize) {
    var_dump_at(out, v, level, &mut Vec::new());
}

/// Address of the array or object behind a value, to detect cycles
fn container_addr(v: &Value) -> Option<usize> {
    match v {
        Value::Array(a) => Some(Rc::as_ptr(a) as usize),
        Value::Object(o) => Some(Rc::as_ptr(o) as usize),
        _ => None,
    }
}

fn dump_key(out: &mut Vec<u8>, pad: &str, key: &[u8], vis: Option<&Visibility>) {
    out.extend_from_slice(pad.as_bytes());
    out.extend_from_slice(b"[\"");
    out.extend_from_slice(key);
    out.push(b'"');
    match vis {
        Some(Visibility::Protected) => out.extend_from_slice(b":protected"),
        Some(Visibility::Private(class)) => out.extend_from_slice(format!(":\"{class}\":private").as_bytes()),
        _ => {}
    }
    out.extend_from_slice(b"]=>\n");
}

/// `visiting` holds the containers being printed, to cut cycles made through
/// references and objects
fn var_dump_at(out: &mut Vec<u8>, v: &Value, level: usize, visiting: &mut Vec<usize>) {
    let pad = " ".repeat(level - 1);
    if let Value::Ref(r) = v {
        return with_deref(r, |inner| {
            if container_addr(inner).is_some_and(|a| visiting.contains(&a)) {
                out.extend_from_slice(format!("{pad}*RECURSION*\n").as_bytes());
                return;
            }
            // A reference only shows as such while something else shares it
            if Rc::strong_count(r) > 1 {
                out.extend_from_slice(pad.as_bytes());
                out.push(b'&');
                var_dump_at(out, inner, 1, visiting);
                // The nested lines of an array still need this level's indentation
                if matches!(inner, Value::Array(_) | Value::Object(_)) {
                    fix_nested_indent(out, level);
                }
            } else {
                var_dump_at(out, inner, level, visiting);
            }
        });
    }
    if container_addr(v).is_some_and(|a| visiting.contains(&a)) {
        out.extend_from_slice(format!("{pad}*RECURSION*\n").as_bytes());
        return;
    }
    out.extend_from_slice(pad.as_bytes());
    let key_pad = " ".repeat(level + 1);
    match v {
        Value::Null => out.extend_from_slice(b"NULL\n"),
        Value::Bool(b) => out.extend_from_slice(format!("bool({b})\n").as_bytes()),
        Value::Int(i) => out.extend_from_slice(format!("int({i})\n").as_bytes()),
        Value::Float(f) => out.extend_from_slice(format!("float({})\n", format_float_repr(*f)).as_bytes()),
        Value::Str(s) => {
            out.extend_from_slice(format!("string({}) \"", s.len()).as_bytes());
            out.extend_from_slice(s);
            out.extend_from_slice(b"\"\n");
        }
        Value::Array(arr) => {
            out.extend_from_slice(format!("array({}) {{\n", arr.len()).as_bytes());
            visiting.push(Rc::as_ptr(arr) as usize);
            for (k, v) in arr.iter() {
                match k {
                    Key::Int(i) => out.extend_from_slice(format!("{key_pad}[{i}]=>\n").as_bytes()),
                    Key::Str(s) => dump_key(out, &key_pad, s, None),
                }
                var_dump_at(out, v, level + 2, visiting);
            }
            visiting.pop();
            out.extend_from_slice(pad.as_bytes());
            out.extend_from_slice(b"}\n");
        }
        Value::Object(o) => {
            let props = object::debug_props(o);
            out.extend_from_slice(format!("object({})#{} ({}) {{\n", o.class.name, o.id, props.len()).as_bytes());
            visiting.push(Rc::as_ptr(o) as usize);
            for (name, vis, v) in &props {
                dump_key(out, &key_pad, name.as_bytes(), Some(vis));
                var_dump_at(out, v, level + 2, visiting);
            }
            visiting.pop();
            out.extend_from_slice(pad.as_bytes());
            out.extend_from_slice(b"}\n");
        }
        Value::Ref(_) => unreachable!("handled above"),
    }
}

/// `&array(...)` is printed at level 1 to get the `&` right after the padding;
/// re-indent its body and closing brace to the real level
fn fix_nested_indent(out: &mut Vec<u8>, level: usize) {
    let Some(start) = out.iter().rposition(|&b| b == b'&') else { return };
    let tail = out.split_off(start);
    let extra = " ".repeat(level - 1);
    for (i, line) in tail.split_inclusive(|&b| b == b'\n').enumerate() {
        if i > 0 {
            out.extend_from_slice(extra.as_bytes());
        }
        out.extend_from_slice(line);
    }
}

pub fn print_r(out: &mut Vec<u8>, v: &Value, indent: usize) {
    print_r_at(out, v, indent, &mut Vec::new());
}

fn print_r_at(out: &mut Vec<u8>, v: &Value, indent: usize, visiting: &mut Vec<usize>) {
    let entries: Vec<(Vec<u8>, Value)> = match v {
        Value::Ref(r) => return with_deref(r, |inner| print_r_at(out, inner, indent, visiting)),
        Value::Array(arr) => {
            out.extend_from_slice(b"Array\n");
            arr.iter()
                .map(|(k, v)| {
                    let key = match k {
                        Key::Int(i) => i.to_string().into_bytes(),
                        Key::Str(s) => s.to_vec(),
                    };
                    (key, v.clone())
                })
                .collect()
        }
        Value::Object(o) => {
            out.extend_from_slice(format!("{} Object\n", o.class.name).as_bytes());
            object::debug_props(o)
                .into_iter()
                .map(|(name, vis, v)| {
                    let key = match vis {
                        Visibility::Public => name,
                        Visibility::Protected => format!("{name}:protected"),
                        Visibility::Private(class) => format!("{name}:{class}:private"),
                    };
                    (key.into_bytes(), v)
                })
                .collect()
        }
        _ => {
            out.extend_from_slice(&v.to_bytes());
            return;
        }
    };
    let addr = container_addr(v).unwrap_or(0);
    if visiting.contains(&addr) {
        out.extend_from_slice(b" *RECURSION*");
        return;
    }
    visiting.push(addr);
    let pad = " ".repeat(indent);
    out.extend_from_slice(pad.as_bytes());
    out.extend_from_slice(b"(\n");
    for (k, v) in &entries {
        out.extend_from_slice(pad.as_bytes());
        out.extend_from_slice(b"    [");
        out.extend_from_slice(k);
        out.extend_from_slice(b"] => ");
        print_r_at(out, v, indent + 8, visiting);
        out.push(b'\n');
    }
    visiting.pop();
    out.extend_from_slice(pad.as_bytes());
    out.extend_from_slice(b")\n");
}
