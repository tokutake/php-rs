//! Array functions. Those that take the array by reference are in call_by_ref,
//! those that call back into PHP code live in the interpreter.

use std::cmp::Ordering;
use std::rc::Rc;

use super::{arg_bool, arg_int, args, expect_array, list, merge_sort, Runtime};
use crate::array::{Key, PhpArray};
use crate::diag;
use crate::interp::{to_key, EResult, Fatal};
use crate::value::{loose_cmp, Value};

pub const NAMES: &[&str] = &[
    "array_slice", "array_search", "array_unique", "array_combine", "array_fill", "array_fill_keys", "array_flip",
    "array_key_first", "array_key_last", "array_column", "array_chunk", "array_pad", "array_count_values",
    "array_diff", "array_diff_key", "array_diff_assoc", "array_intersect", "array_intersect_key",
    "array_intersect_assoc", "array_replace", "array_is_list", "array_merge_recursive", "current", "pos", "key",
];

pub const BY_REF: &[&str] =
    &["asort", "arsort", "ksort", "krsort", "array_shift", "array_unshift", "array_splice", "natsort", "natcasesort"];

pub const SORT_REGULAR: i64 = 0;
pub const SORT_NUMERIC: i64 = 1;
pub const SORT_STRING: i64 = 2;
pub const SORT_LOCALE_STRING: i64 = 5;
pub const SORT_NATURAL: i64 = 6;
pub const SORT_FLAG_CASE: i64 = 8;

pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "SORT_REGULAR" => SORT_REGULAR,
        "SORT_NUMERIC" => SORT_NUMERIC,
        "SORT_STRING" => SORT_STRING,
        "SORT_LOCALE_STRING" => SORT_LOCALE_STRING,
        "SORT_NATURAL" => SORT_NATURAL,
        "SORT_FLAG_CASE" => SORT_FLAG_CASE,
        "SORT_ASC" => 4,
        "SORT_DESC" => 3,
        "COUNT_NORMAL" => 0,
        "COUNT_RECURSIVE" => 1,
        _ => return None,
    }))
}

/// Appends with a fresh int key, or keeps a string key
fn push_renumbered(out: &mut PhpArray, k: &Key, v: Value) -> EResult<()> {
    match k {
        Key::Int(_) => out.push(v).map(|_| ()).map_err(|_| super::next_occupied()),
        Key::Str(_) => {
            out.insert(k.clone(), v);
            Ok(())
        }
    }
}

/// A value used as a key by array_combine() and array_fill_keys(): ints stay,
/// anything else goes through its string form
fn value_key(rt: &mut dyn Runtime, v: &Value) -> EResult<Key> {
    match v {
        Value::Int(i) => Ok(Key::Int(*i)),
        v => Ok(Key::from_bytes(&rt.stringify(v)?)),
    }
}

/// The string form used by array_diff(), array_intersect() and array_unique()
fn str_form(rt: &mut dyn Runtime, v: &Value) -> EResult<Vec<u8>> {
    match v {
        Value::Array(_) => {
            diag::warning("Array to string conversion");
            Ok(b"Array".to_vec())
        }
        v => rt.stringify(v),
    }
}

/// Position bounds as array_slice() and array_splice() compute them
fn slice_bounds(len: usize, offset: i64, length: Option<i64>) -> (usize, usize) {
    let n = len as i64;
    let start = if offset < 0 { (n + offset).max(0) } else { offset.min(n) };
    let end = match length {
        None => n,
        Some(l) if l < 0 => (n + l).max(start),
        Some(l) => start.saturating_add(l).min(n),
    };
    (start as usize, end.max(start) as usize)
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "current" | "pos" | "key" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            Ok(match (arr.current(), lname) {
                (Some((k, _)), "key") => k.to_value(),
                (Some((_, v)), _) => v.copy_out(),
                (None, "key") => Value::Null,
                (None, _) => Value::Bool(false),
            })
        }
        "array_slice" => {
            let [a, off, len, preserve] = args::<2, 4>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let off = arg_int(lname, 2, "offset", &off.unwrap())?;
            let len = match len {
                None | Some(Value::Null) => None,
                Some(l) => Some(arg_int(lname, 3, "length", &l)?),
            };
            let preserve = match preserve {
                Some(p) => arg_bool(lname, 4, "preserve_keys", &p)?,
                None => false,
            };
            let (start, end) = slice_bounds(arr.len(), off, len);
            let mut out = PhpArray::new();
            for (k, v) in arr.iter().skip(start).take(end - start) {
                if preserve {
                    out.insert(k.clone(), v.clone());
                } else {
                    push_renumbered(&mut out, k, v.clone())?;
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_search" => {
            let [needle, hay, strict] = args::<2, 3>(name, vals)?;
            let needle = needle.unwrap();
            let hay = expect_array(lname, 2, "haystack", &hay.unwrap())?;
            let strict = match strict {
                Some(s) => arg_bool(lname, 3, "strict", &s)?,
                None => false,
            };
            let found = hay.iter().find(|(_, v)| if strict { v.strict_eq(&needle) } else { v.loose_eq(&needle) });
            Ok(found.map_or(Value::Bool(false), |(k, _)| k.to_value()))
        }
        "array_unique" => {
            let [a, flags] = args::<1, 2>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let flags = match flags {
                Some(f) => arg_int(lname, 2, "flags", &f)?,
                None => SORT_STRING,
            };
            let mut out = PhpArray::new();
            if flags == SORT_STRING || flags == SORT_LOCALE_STRING {
                let mut seen = std::collections::HashSet::new();
                for (k, v) in arr.iter() {
                    if seen.insert(str_form(rt, &v.copy_out())?) {
                        out.insert(k.clone(), v.clone());
                    }
                }
            } else {
                let mut kept: Vec<Value> = Vec::new();
                for (k, v) in arr.iter() {
                    let v = v.copy_out();
                    let dup = kept.iter().any(|x| sort_cmp(rt, flags, x, &v).is_ok_and(|o| o == Ordering::Equal));
                    if !dup {
                        kept.push(v.clone());
                        out.insert(k.clone(), v);
                    }
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_combine" => {
            let [k, v] = args::<2, 2>(name, vals)?;
            let keys = expect_array(lname, 1, "keys", &k.unwrap())?;
            let values = expect_array(lname, 2, "values", &v.unwrap())?;
            if keys.len() != values.len() {
                return Err(Fatal::value_error(
                    "array_combine(): Argument #1 ($keys) and argument #2 ($values) must have the same number of elements",
                ));
            }
            let mut out = PhpArray::new();
            for (k, v) in keys.values().zip(values.values()) {
                out.insert(value_key(rt, &k.copy_out())?, v.clone());
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_fill" => {
            let [start, count, v] = args::<3, 3>(name, vals)?;
            let start = arg_int(lname, 1, "start_index", &start.unwrap())?;
            let count = arg_int(lname, 2, "count", &count.unwrap())?;
            if count < 0 {
                return Err(Fatal::value_error("array_fill(): Argument #2 ($count) must be greater than or equal to 0"));
            }
            if count as u64 > super::MAX_ARRAY_SIZE {
                return Err(Fatal::value_error("array_fill(): Argument #2 ($count) is too large"));
            }
            let v = v.unwrap();
            let mut out = PhpArray::new();
            for i in 0..count {
                out.insert(Key::Int(start.wrapping_add(i)), v.clone());
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_fill_keys" => {
            let [k, v] = args::<2, 2>(name, vals)?;
            let keys = expect_array(lname, 1, "keys", &k.unwrap())?;
            let v = v.unwrap();
            let mut out = PhpArray::new();
            for k in keys.values() {
                out.insert(value_key(rt, &k.copy_out())?, v.clone());
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_flip" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let mut out = PhpArray::new();
            for (k, v) in arr.iter() {
                match v.copy_out() {
                    Value::Int(i) => out.insert(Key::Int(i), k.to_value()),
                    Value::Str(s) => out.insert(Key::from_bytes(&s), k.to_value()),
                    _ => diag::warning("array_flip(): Can only flip string and integer values, entry skipped"),
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_key_first" | "array_key_last" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let k = if lname == "array_key_first" { arr.keys().next() } else { arr.iter().next_back().map(|(k, _)| k) };
            Ok(k.map_or(Value::Null, Key::to_value))
        }
        "array_column" => {
            let [a, col, idx] = args::<2, 3>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let col = col.unwrap();
            let idx = idx.unwrap_or(Value::Null);
            let field = |row: &Value, key: &Value| -> EResult<Option<Value>> {
                match row {
                    Value::Array(r) => Ok(r.get(&to_key(key, "array")?).map(Value::copy_out)),
                    Value::Object(o) => {
                        let name = key.to_bytes();
                        Ok(o.props.try_borrow().ok().and_then(|p| p.get(&Key::from_bytes(&name)).map(Value::copy_out)))
                    }
                    _ => Ok(None),
                }
            };
            let mut out = PhpArray::new();
            for row in arr.values() {
                let row = row.copy_out();
                let v = if col.is_null() {
                    if !matches!(row, Value::Array(_) | Value::Object(_)) {
                        continue;
                    }
                    row.clone()
                } else {
                    match field(&row, &col)? {
                        Some(v) => v,
                        None => continue,
                    }
                };
                match if idx.is_null() { None } else { field(&row, &idx)? } {
                    Some(k @ (Value::Int(_) | Value::Str(_))) => out.insert(to_key(&k, "array")?, v),
                    Some(k) => out.insert(value_key(rt, &k)?, v),
                    None => {
                        out.push(v).map_err(|_| super::next_occupied())?;
                    }
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_chunk" => {
            let [a, size, preserve] = args::<2, 3>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let size = arg_int(lname, 2, "length", &size.unwrap())?;
            let preserve = match preserve {
                Some(p) => arg_bool(lname, 3, "preserve_keys", &p)?,
                None => false,
            };
            if size < 1 {
                return Err(Fatal::value_error("array_chunk(): Argument #2 ($length) must be greater than 0"));
            }
            let mut chunks = Vec::new();
            let mut cur = PhpArray::new();
            for (k, v) in arr.iter() {
                if preserve {
                    cur.insert(k.clone(), v.clone());
                } else {
                    cur.push(v.clone()).map_err(|_| super::next_occupied())?;
                }
                if cur.len() as i64 == size {
                    chunks.push(Value::Array(Rc::new(std::mem::take(&mut cur))));
                }
            }
            if !cur.is_empty() {
                chunks.push(Value::Array(Rc::new(cur)));
            }
            Ok(list(chunks))
        }
        "array_pad" => {
            let [a, size, v] = args::<3, 3>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let size = arg_int(lname, 2, "length", &size.unwrap())?;
            let v = v.unwrap();
            let target = size.unsigned_abs();
            if target <= arr.len() as u64 {
                return Ok(Value::Array(arr));
            }
            if target > super::MAX_ARRAY_SIZE {
                return Err(Fatal::value_error("array_pad(): Argument #2 ($length) must not exceed the maximum allowed array size"));
            }
            let pad = target as usize - arr.len();
            let mut out = PhpArray::new();
            if size < 0 {
                for _ in 0..pad {
                    out.push(v.clone()).map_err(|_| super::next_occupied())?;
                }
            }
            for (k, x) in arr.iter() {
                push_renumbered(&mut out, k, x.clone())?;
            }
            if size > 0 {
                for _ in 0..pad {
                    out.push(v.clone()).map_err(|_| super::next_occupied())?;
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_count_values" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let mut out = PhpArray::new();
            for v in arr.values() {
                let key = match v.copy_out() {
                    Value::Int(i) => Key::Int(i),
                    Value::Str(s) => Key::from_bytes(&s),
                    _ => {
                        diag::warning("array_count_values(): Can only count string and integer values, entry skipped");
                        continue;
                    }
                };
                let slot = out.entry(key);
                *slot = Value::Int(slot.to_int() + 1);
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_diff" | "array_diff_key" | "array_diff_assoc" | "array_intersect" | "array_intersect_key"
        | "array_intersect_assoc" => {
            let diff = lname.starts_with("array_diff");
            let mut arrays = Vec::with_capacity(vals.len());
            for (i, v) in vals.iter().enumerate() {
                let p = if i == 0 { "array" } else { "arrays" };
                arrays.push(expect_array(lname, i + 1, p, v)?);
            }
            let Some((first, others)) = arrays.split_first() else {
                return Err(super::arg_count(lname, 1, 0));
            };
            let by_key = lname.ends_with("_key");
            let by_value = !by_key;
            let with_key = !lname.ends_with("diff") && !lname.ends_with("intersect");
            // String forms of the other arrays' values, for value comparisons
            let mut other_vals: Vec<Vec<Vec<u8>>> = Vec::new();
            if by_value && !with_key {
                for o in others {
                    let mut forms = Vec::with_capacity(o.len());
                    for v in o.values() {
                        forms.push(str_form(rt, &v.copy_out())?);
                    }
                    other_vals.push(forms);
                }
            }
            let mut out = PhpArray::new();
            for (k, v) in first.iter() {
                let form = if by_value { Some(str_form(rt, &v.copy_out())?) } else { None };
                let mut in_all = true;
                let mut in_any = false;
                for (i, o) in others.iter().enumerate() {
                    let present = if by_key {
                        o.contains_key(k)
                    } else if with_key {
                        match o.get(k) {
                            Some(ov) => str_form(rt, &ov.copy_out())? == *form.as_ref().unwrap(),
                            None => false,
                        }
                    } else {
                        other_vals[i].iter().any(|f| Some(f) == form.as_ref())
                    };
                    in_all &= present;
                    in_any |= present;
                }
                if (diff && !in_any) || (!diff && in_all) {
                    out.insert(k.clone(), v.clone());
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_replace" => {
            let mut out = PhpArray::new();
            for (i, v) in vals.iter().enumerate() {
                let arr = expect_array(lname, i + 1, if i == 0 { "array" } else { "replacements" }, v)?;
                for (k, v) in arr.iter() {
                    out.insert(k.clone(), v.clone());
                }
            }
            if vals.is_empty() {
                return Err(super::arg_count(lname, 1, 0));
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_is_list" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            Ok(Value::Bool(arr.keys().enumerate().all(|(i, k)| *k == Key::Int(i as i64))))
        }
        "array_merge_recursive" => {
            let mut out = PhpArray::new();
            for (i, v) in vals.iter().enumerate() {
                let arr = expect_array(lname, i + 1, "arrays", v)?;
                merge_recursive(&mut out, &arr, 0)?;
            }
            Ok(Value::Array(Rc::new(out)))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}

fn merge_recursive(out: &mut PhpArray, src: &PhpArray, depth: usize) -> EResult<()> {
    if depth > 256 {
        return Err(Fatal::error("array_merge_recursive(): Nesting level too deep"));
    }
    for (k, v) in src.iter() {
        match k {
            Key::Int(_) => {
                out.push(v.clone()).map_err(|_| super::next_occupied())?;
            }
            Key::Str(_) => match out.get(k).cloned() {
                None => out.insert(k.clone(), v.clone()),
                Some(existing) => {
                    // Both values end up in one array
                    let mut merged = match existing.copy_out() {
                        Value::Array(a) => Rc::unwrap_or_clone(a),
                        other => PhpArray::from_values([other]),
                    };
                    match v.copy_out() {
                        Value::Array(a) => merge_recursive(&mut merged, &a, depth + 1)?,
                        other => {
                            merged.push(other).map_err(|_| super::next_occupied())?;
                        }
                    }
                    out.insert(k.clone(), Value::Array(Rc::new(merged)));
                }
            },
        }
    }
    Ok(())
}

/// Comparison for the sort flags (SORT_REGULAR, SORT_STRING, ...)
pub fn sort_cmp(rt: &mut dyn Runtime, flags: i64, a: &Value, b: &Value) -> EResult<Ordering> {
    let case = flags & SORT_FLAG_CASE != 0;
    Ok(match flags & !SORT_FLAG_CASE {
        SORT_NUMERIC => a.to_float().partial_cmp(&b.to_float()).unwrap_or(Ordering::Equal),
        SORT_STRING | SORT_LOCALE_STRING => {
            let (mut x, mut y) = (str_form(rt, a)?, str_form(rt, b)?);
            if case {
                x.make_ascii_lowercase();
                y.make_ascii_lowercase();
            }
            x.cmp(&y)
        }
        SORT_NATURAL => {
            let (x, y) = (str_form(rt, a)?, str_form(rt, b)?);
            super::string::strnatcmp(&x, &y, case).cmp(&0)
        }
        // Uncomparable pairs count as "greater", as zend_compare reports 1 for them
        _ => loose_cmp(a, b).unwrap_or(Ordering::Greater),
    })
}

fn sort_flags(lname: &str, n: usize, v: Option<&Value>) -> EResult<i64> {
    v.map_or(Ok(SORT_REGULAR), |f| arg_int(lname, n, "flags", f))
}

/// The by-reference array functions; `arr` is the array in its variable
pub fn call_by_ref(rt: &mut dyn Runtime, lname: &str, arr: &mut Rc<PhpArray>, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "sort" | "rsort" | "asort" | "arsort" | "ksort" | "krsort" => {
            let flags = sort_flags(lname, 2, vals.first())?;
            let reverse = matches!(lname, "rsort" | "arsort" | "krsort");
            let by_key = lname.starts_with('k');
            let mut entries: Vec<(Key, Value)> = arr.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let mut err = None;
            merge_sort(&mut entries, |a, b| {
                let (x, y) = if by_key { (a.0.to_value(), b.0.to_value()) } else { (a.1.copy_out(), b.1.copy_out()) };
                match sort_cmp(rt, flags, &x, &y) {
                    Ok(o) => {
                        if reverse {
                            o.reverse()
                        } else {
                            o
                        }
                    }
                    Err(e) => {
                        err.get_or_insert(e);
                        Ordering::Equal
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
            *arr = Rc::new(if matches!(lname, "sort" | "rsort") {
                PhpArray::from_values(entries.into_iter().map(|(_, v)| v))
            } else {
                entries.into_iter().collect()
            });
            Ok(Value::Bool(true))
        }
        "natsort" | "natcasesort" => {
            let ci = lname == "natcasesort";
            let mut entries: Vec<(Key, Value)> = arr.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let mut err = None;
            merge_sort(&mut entries, |a, b| {
                let x = rt.stringify(&a.1.copy_out());
                let y = rt.stringify(&b.1.copy_out());
                match (x, y) {
                    (Ok(x), Ok(y)) => super::string::strnatcmp(&x, &y, ci).cmp(&0),
                    (Err(e), _) | (_, Err(e)) => {
                        err.get_or_insert(e);
                        Ordering::Equal
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
            *arr = Rc::new(entries.into_iter().collect());
            Ok(Value::Bool(true))
        }
        "array_shift" => {
            let Some(first) = arr.keys().next().cloned() else { return Ok(Value::Null) };
            let mut rest = (**arr).clone();
            let v = rest.remove(&first).unwrap_or(Value::Null);
            let mut out = PhpArray::new();
            for (k, x) in rest.iter() {
                push_renumbered(&mut out, k, x.clone())?;
            }
            *arr = Rc::new(out);
            Ok(v.copy_out())
        }
        "array_unshift" => {
            let mut out = PhpArray::new();
            for v in vals {
                out.push(v).map_err(|_| super::next_occupied())?;
            }
            for (k, x) in arr.iter() {
                push_renumbered(&mut out, k, x.clone())?;
            }
            let n = out.len();
            *arr = Rc::new(out);
            Ok(Value::Int(n as i64))
        }
        "array_splice" => {
            let mut vals = vals.into_iter();
            let Some(off) = vals.next() else { return Err(super::arg_count(lname, 2, 1)) };
            let off = arg_int(lname, 2, "offset", &off)?;
            let len = match vals.next() {
                None | Some(Value::Null) => None,
                Some(l) => Some(arg_int(lname, 3, "length", &l)?),
            };
            let replacement: Vec<Value> = match vals.next() {
                None => Vec::new(),
                Some(Value::Array(a)) => a.values().cloned().collect(),
                Some(other) => vec![other],
            };
            let (start, end) = slice_bounds(arr.len(), off, len);
            let mut removed = PhpArray::new();
            let mut out = PhpArray::new();
            for (i, (k, v)) in arr.iter().enumerate() {
                if i == start {
                    for r in &replacement {
                        out.push(r.clone()).map_err(|_| super::next_occupied())?;
                    }
                }
                if (start..end).contains(&i) {
                    push_renumbered(&mut removed, k, v.clone())?;
                } else {
                    push_renumbered(&mut out, k, v.clone())?;
                }
            }
            if start >= arr.len() {
                for r in replacement {
                    out.push(r).map_err(|_| super::next_occupied())?;
                }
            }
            *arr = Rc::new(out);
            Ok(Value::Array(Rc::new(removed)))
        }
        _ => unreachable!("{lname} is not a by-ref array function"),
    }
}
