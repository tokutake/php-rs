//! preg_match() and the other preg_* functions, after php-src's ext/pcre,
//! on top of the engine in crate::regex.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use super::{arg_int, arg_str, args, expect_array, null_param, param_type_error, Runtime};
use crate::array::{Key, PhpArray};
use crate::diag;
use crate::interp::{strict_types, EResult, Fatal};
use crate::regex::{self, Captures, ExecError, MatchOptions, Options, Regex};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "preg_match",
    "preg_match_all",
    "preg_replace",
    "preg_filter",
    "preg_replace_callback",
    "preg_replace_callback_array",
    "preg_split",
    "preg_quote",
    "preg_grep",
    "preg_last_error",
    "preg_last_error_msg",
];

const PATTERN_ORDER: i64 = 1;
const SET_ORDER: i64 = 2;
const OFFSET_CAPTURE: i64 = 1 << 8;
const UNMATCHED_AS_NULL: i64 = 1 << 9;
const SPLIT_NO_EMPTY: i64 = 1;
const SPLIT_DELIM_CAPTURE: i64 = 2;
const SPLIT_OFFSET_CAPTURE: i64 = 4;
const GREP_INVERT: i64 = 1;

const NO_ERROR: i64 = 0;
const INTERNAL_ERROR: i64 = 1;
const BACKTRACK_LIMIT_ERROR: i64 = 2;
const RECURSION_LIMIT_ERROR: i64 = 3;
const BAD_UTF8_ERROR: i64 = 4;
const BAD_UTF8_OFFSET_ERROR: i64 = 5;
const JIT_STACKLIMIT_ERROR: i64 = 6;

/// php-src's PCRE_CACHE_SIZE
const CACHE_SIZE: usize = 4096;

pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "PREG_PATTERN_ORDER" => PATTERN_ORDER,
        "PREG_SET_ORDER" => SET_ORDER,
        "PREG_OFFSET_CAPTURE" => OFFSET_CAPTURE,
        "PREG_UNMATCHED_AS_NULL" => UNMATCHED_AS_NULL,
        "PREG_SPLIT_NO_EMPTY" => SPLIT_NO_EMPTY,
        "PREG_SPLIT_DELIM_CAPTURE" => SPLIT_DELIM_CAPTURE,
        "PREG_SPLIT_OFFSET_CAPTURE" => SPLIT_OFFSET_CAPTURE,
        "PREG_GREP_INVERT" => GREP_INVERT,
        "PREG_NO_ERROR" => NO_ERROR,
        "PREG_INTERNAL_ERROR" => INTERNAL_ERROR,
        "PREG_BACKTRACK_LIMIT_ERROR" => BACKTRACK_LIMIT_ERROR,
        "PREG_RECURSION_LIMIT_ERROR" => RECURSION_LIMIT_ERROR,
        "PREG_BAD_UTF8_ERROR" => BAD_UTF8_ERROR,
        "PREG_BAD_UTF8_OFFSET_ERROR" => BAD_UTF8_OFFSET_ERROR,
        "PREG_JIT_STACKLIMIT_ERROR" => JIT_STACKLIMIT_ERROR,
        "PCRE_VERSION_MAJOR" => 10,
        "PCRE_VERSION_MINOR" => 42,
        "PCRE_VERSION" => return Some(Value::str("10.42 2022-12-11")),
        "PCRE_JIT_SUPPORT" => return Some(Value::Bool(false)),
        _ => return None,
    }))
}

thread_local! {
    static LAST_ERROR: Cell<i64> = const { Cell::new(NO_ERROR) };
    static CACHE: RefCell<HashMap<Vec<u8>, Rc<Regex>>> = RefCell::new(HashMap::new());
}

fn set_error(code: i64) {
    LAST_ERROR.with(|e| e.set(code));
}

fn exec_error(e: ExecError) -> i64 {
    match e {
        ExecError::BacktrackLimit => BACKTRACK_LIMIT_ERROR,
        ExecError::RecursionLimit => RECURSION_LIMIT_ERROR,
        ExecError::StackLimit => JIT_STACKLIMIT_ERROR,
        ExecError::RecursionLoop => INTERNAL_ERROR,
    }
}

/// The by-reference parameter of each function, as (position, name)
pub fn out_param(lname: &str) -> Option<(usize, &'static str)> {
    match lname {
        "preg_match" | "preg_match_all" => Some((2, "matches")),
        "preg_replace" | "preg_filter" | "preg_replace_callback" => Some((4, "count")),
        "preg_replace_callback_array" => Some((3, "count")),
        _ => None,
    }
}

/// Compiles a delimited pattern (or takes it from the cache), with php-src's
/// warnings when it is bad
fn compile(fname: &str, regex: &[u8]) -> Option<Rc<Regex>> {
    if let Some(re) = CACHE.with(|c| c.borrow().get(regex).cloned()) {
        return Some(re);
    }
    let fail = |msg: String| {
        diag::warning(format!("{fname}(): {msg}"));
        set_error(INTERNAL_ERROR);
        None
    };
    let mut p = regex.iter().position(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)).unwrap_or(regex.len());
    if p >= regex.len() {
        return fail("Empty regular expression".into());
    }
    let start_delim = regex[p];
    if start_delim.is_ascii_alphanumeric() || start_delim == b'\\' || start_delim == 0 {
        return fail("Delimiter must not be alphanumeric, backslash, or NUL byte".into());
    }
    p += 1;
    let end_delim = match start_delim {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        d => d,
    };
    let body_start = p;
    let mut depth = 1;
    while p < regex.len() {
        let c = regex[p];
        if c == b'\\' && p + 1 < regex.len() {
            p += 1;
        } else if c == end_delim && start_delim == end_delim {
            break;
        } else if c == end_delim {
            depth -= 1;
            if depth <= 0 {
                break;
            }
        } else if c == start_delim {
            depth += 1;
        }
        p += 1;
    }
    if p >= regex.len() {
        let which = if start_delim == end_delim { "" } else { "matching " };
        return fail(format!("No ending {which}delimiter '{}' found", end_delim as char));
    }
    let pattern = &regex[body_start..p];
    let mut opts = Options::default();
    for &m in &regex[p + 1..] {
        match m {
            b'i' => opts.caseless = true,
            b'm' => opts.multiline = true,
            b'n' => opts.no_auto_capture = true,
            b's' => opts.dotall = true,
            b'x' => opts.extended = true,
            b'A' => opts.anchored = true,
            b'D' => opts.dollar_endonly = true,
            b'r' | b'S' | b'X' => {}
            b'U' => opts.ungreedy = true,
            b'u' => {
                opts.utf = true;
                opts.ucp = true;
            }
            b'J' => opts.dupnames = true,
            b' ' | b'\n' | b'\r' => {}
            0 => return fail("NUL byte is not a valid modifier".into()),
            m => return fail(format!("Unknown modifier '{}'", m as char)),
        }
    }
    let re = match Regex::new(pattern, &opts) {
        Ok(re) => Rc::new(re),
        Err(e) => return fail(format!("Compilation failed: {} at offset {}", e.message(), e.offset)),
    };
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= CACHE_SIZE {
            c.clear();
        }
        c.insert(regex.to_vec(), re.clone());
    });
    Some(re)
}

/// One match attempt. `check_utf` validates the subject (and the offset) in
/// UTF mode, as pcre2_match does without PCRE2_NO_UTF_CHECK
fn exec(re: &Regex, s: &[u8], start: usize, retry_empty: bool, check_utf: bool) -> Result<Option<Captures>, i64> {
    if start > s.len() {
        return Err(INTERNAL_ERROR);
    }
    if check_utf && re.is_utf() {
        if regex::validate_utf8(s).is_err() {
            return Err(BAD_UTF8_ERROR);
        }
        if start < s.len() && s[start] & 0xc0 == 0x80 {
            return Err(BAD_UTF8_OFFSET_ERROR);
        }
    }
    let opts = MatchOptions { notempty_atstart: retry_empty, anchored: retry_empty };
    re.exec(s, start, opts).map_err(exec_error)
}

/// How far to move on after an empty match that cannot be repeated
fn unit_len(re: &Regex, s: &[u8], pos: usize) -> usize {
    if !re.is_utf() {
        return 1;
    }
    let mut n = 1;
    while pos + n < s.len() && s[pos + n] & 0xc0 == 0x80 {
        n += 1;
    }
    n
}

/// Group names by number
fn subpat_names(re: &Regex) -> Option<Vec<Option<Rc<[u8]>>>> {
    if re.names.is_empty() {
        return None;
    }
    let mut names = vec![None; re.capture_count + 1];
    for (name, i) in &re.names {
        if names[*i].is_none() {
            names[*i] = Some(Rc::from(name.as_slice()));
        }
    }
    Some(names)
}

fn substr(s: &[u8], range: Option<(usize, usize)>, unmatched_as_null: bool) -> Value {
    match range {
        Some((a, b)) => Value::str(s.get(a..b).unwrap_or_default().to_vec()),
        None if unmatched_as_null => Value::Null,
        None => Value::str(""),
    }
}

fn offset_pair(s: &[u8], range: Option<(usize, usize)>, unmatched_as_null: bool) -> Value {
    let (text, offset) = match range {
        Some((a, b)) => (Value::str(s.get(a..b).unwrap_or_default().to_vec()), Value::Int(a as i64)),
        None => (if unmatched_as_null { Value::Null } else { Value::str("") }, Value::Int(-1)),
    };
    Value::Array(Rc::new(PhpArray::from_values([text, offset])))
}

fn name_key(name: &[u8]) -> Key {
    Key::Str(name.into())
}

/// A name entry: an unmatched group does not replace one that matched (with
/// duplicate names)
fn add_named(out: &mut PhpArray, name: &[u8], v: Value, unmatched: bool) {
    let key = name_key(name);
    if unmatched && out.contains_key(&key) {
        return;
    }
    out.insert(key, v);
}

fn push(out: &mut PhpArray, v: Value) {
    let _ = out.push(v);
}

/// php-src's populate_subpat_array: the groups of one match
fn subpat_array(s: &[u8], caps: &Captures, names: &Option<Vec<Option<Rc<[u8]>>>>, num: usize, flags: i64) -> PhpArray {
    let offset_capture = flags & OFFSET_CAPTURE != 0;
    let as_null = flags & UNMATCHED_AS_NULL != 0;
    let count = caps.count();
    let mut out = PhpArray::new();
    for i in 0..num {
        if i >= count && !as_null {
            break;
        }
        let range = if i < count { caps.get(i) } else { None };
        let v = if offset_capture { offset_pair(s, range, as_null) } else { substr(s, range, as_null) };
        if let Some(Some(name)) = names.as_ref().map(|n| &n[i]) {
            add_named(&mut out, name, v.clone(), range.is_none());
        }
        push(&mut out, v);
    }
    if let Some(mark) = &caps.mark {
        out.insert(name_key(b"MARK"), Value::str(mark.clone()));
    }
    out
}

fn array_value(a: PhpArray) -> Value {
    Value::Array(Rc::new(a))
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_out(rt, name, lname, vals).map(|(v, _)| v))
}

/// Runs a function, giving its result and the value for its by-reference
/// parameter (None: leave it alone)
pub fn call_out(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<(Value, Option<Value>)> {
    match lname {
        "preg_match" | "preg_match_all" => {
            let [regex, subject, _, flags, offset] = args::<2, 5>(name, vals)?;
            let regex = arg_str(rt, lname, 1, "pattern", &regex.unwrap())?;
            let subject = arg_str(rt, lname, 2, "subject", &subject.unwrap())?;
            let flags = match flags {
                Some(f) => arg_int(lname, 4, "flags", &f)?,
                None => 0,
            };
            let offset = match offset {
                Some(o) => arg_int(lname, 5, "offset", &o)?,
                None => 0,
            };
            let Some(re) = compile(lname, &regex) else { return Ok((Value::Bool(false), None)) };
            if offset == i64::MIN {
                return Err(Fatal::value_error(format!(
                    "{lname}(): Argument #5 ($offset) must be greater than {}",
                    i64::MIN
                )));
            }
            match_impl(&re, &subject, lname == "preg_match_all", flags, offset)
        }
        "preg_replace" | "preg_filter" => {
            let [regex, replace, subject, limit, _] = args::<3, 5>(name, vals)?;
            let regex = array_or_str(rt, lname, 1, "pattern", regex.unwrap())?;
            let replace = array_or_str(rt, lname, 2, "replacement", replace.unwrap())?;
            let subject = array_or_str(rt, lname, 3, "subject", subject.unwrap())?;
            let limit = match limit {
                Some(l) => arg_int(lname, 4, "limit", &l)?,
                None => -1,
            };
            if matches!(replace, ArrOrStr::Arr(_)) && matches!(regex, ArrOrStr::Str(_)) {
                return Err(Fatal::type_error(format!(
                    "{lname}(): Argument #1 ($pattern) must be of type array when argument #2 ($replacement) is an array, string given"
                )));
            }
            let filter = lname == "preg_filter";
            let mut count = 0;
            let one = |rt: &mut dyn Runtime, subject: &[u8], count: &mut usize| -> EResult<Option<Vec<u8>>> {
                replace_in_subject(rt, lname, &regex, &replace, subject, limit, count)
            };
            let result = match subject {
                ArrOrStr::Str(s) => {
                    let before = count;
                    match one(rt, &s, &mut count)? {
                        Some(r) if !filter || count > before => Value::str(r),
                        _ => Value::Null,
                    }
                }
                ArrOrStr::Arr(a) => {
                    let mut out = PhpArray::new();
                    for (k, v) in a.iter() {
                        let s = rt.stringify(&v.copy_out())?;
                        let before = count;
                        if let Some(r) = one(rt, &s, &mut count)?
                            && (!filter || count > before)
                        {
                            out.insert(k.clone(), Value::str(r));
                        }
                    }
                    array_value(out)
                }
            };
            Ok((result, Some(Value::Int(count as i64))))
        }
        "preg_replace_callback" => {
            let [regex, callback, subject, limit, _, flags] = args::<3, 6>(name, vals)?;
            let regex = array_or_str(rt, lname, 1, "pattern", regex.unwrap())?;
            let callback = callback.unwrap();
            rt.check_callback(lname, 2, &callback)?;
            let subject = array_or_str(rt, lname, 3, "subject", subject.unwrap())?;
            let limit = match limit {
                Some(l) => arg_int(lname, 4, "limit", &l)?,
                None => -1,
            };
            let flags = match flags {
                Some(f) => arg_int(lname, 6, "flags", &f)?,
                None => 0,
            };
            let patterns = match regex {
                ArrOrStr::Str(s) => vec![s],
                ArrOrStr::Arr(a) => {
                    let mut v = Vec::new();
                    for p in a.values() {
                        v.push(rt.stringify(&p.copy_out())?);
                    }
                    v
                }
            };
            let mut count = 0;
            let result = replace_callback_subjects(rt, lname, &patterns, &callback, subject, limit, flags, &mut count)?;
            Ok((result, Some(Value::Int(count as i64))))
        }
        "preg_replace_callback_array" => {
            let [map, subject, limit, _, flags] = args::<2, 5>(name, vals)?;
            let map = expect_array(lname, 1, "pattern", &map.unwrap())?;
            let mut subject = array_or_str(rt, lname, 2, "subject", subject.unwrap())?;
            let limit = match limit {
                Some(l) => arg_int(lname, 3, "limit", &l)?,
                None => -1,
            };
            let flags = match flags {
                Some(f) => arg_int(lname, 5, "flags", &f)?,
                None => 0,
            };
            let mut count = 0;
            for (k, cb) in map.iter() {
                let Key::Str(pattern) = k else {
                    return Err(Fatal::type_error(format!("{lname}(): Argument #1 ($pattern) must contain only string patterns as keys")));
                };
                let cb = cb.copy_out();
                if !rt.is_callable(&cb) {
                    return Err(Fatal::type_error(format!("{lname}(): Argument #1 ($pattern) must contain only valid callbacks")));
                }
                let result =
                    replace_callback_subjects(rt, lname, &[pattern.to_vec()], &cb, subject, limit, flags, &mut count)?;
                subject = match result {
                    Value::Array(a) => ArrOrStr::Arr(a),
                    Value::Str(s) => ArrOrStr::Str(s.to_vec()),
                    _ => return Ok((Value::Null, None)),
                };
            }
            let result = match subject {
                ArrOrStr::Arr(a) => Value::Array(a),
                ArrOrStr::Str(s) => Value::str(s),
            };
            Ok((result, Some(Value::Int(count as i64))))
        }
        "preg_split" => {
            let [regex, subject, limit, flags] = args::<2, 4>(name, vals)?;
            let regex = arg_str(rt, lname, 1, "pattern", &regex.unwrap())?;
            let subject = arg_str(rt, lname, 2, "subject", &subject.unwrap())?;
            let limit = match limit {
                Some(Value::Null) => -1,
                Some(l) => arg_int(lname, 3, "limit", &l)?,
                None => -1,
            };
            let flags = match flags {
                Some(f) => arg_int(lname, 4, "flags", &f)?,
                None => 0,
            };
            let Some(re) = compile(lname, &regex) else { return Ok((Value::Bool(false), None)) };
            Ok((split(&re, &subject, limit, flags), None))
        }
        "preg_quote" => {
            let [s, delim] = args::<1, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "str", &s.unwrap())?;
            let delim = match delim {
                None | Some(Value::Null) => None,
                Some(d) => arg_str(rt, lname, 2, "delimiter", &d)?.first().copied(),
            };
            Ok((Value::str(quote(&s, delim)), None))
        }
        "preg_grep" => {
            let [regex, input, flags] = args::<2, 3>(name, vals)?;
            let regex = arg_str(rt, lname, 1, "pattern", &regex.unwrap())?;
            let input = expect_array(lname, 2, "array", &input.unwrap())?;
            let flags = match flags {
                Some(f) => arg_int(lname, 3, "flags", &f)?,
                None => 0,
            };
            let Some(re) = compile(lname, &regex) else { return Ok((Value::Bool(false), None)) };
            let invert = flags & GREP_INVERT != 0;
            set_error(NO_ERROR);
            let mut out = PhpArray::new();
            for (k, v) in input.iter() {
                let entry = v.copy_out();
                let s = rt.stringify(&entry)?;
                match exec(&re, &s, 0, false, true) {
                    Ok(m) => {
                        if m.is_some() != invert {
                            out.insert(k.clone(), entry);
                        }
                    }
                    Err(e) => {
                        set_error(e);
                        break;
                    }
                }
            }
            Ok((array_value(out), None))
        }
        "preg_last_error" => {
            args::<0, 0>(name, vals)?;
            Ok((Value::Int(LAST_ERROR.with(Cell::get)), None))
        }
        "preg_last_error_msg" => {
            args::<0, 0>(name, vals)?;
            let msg = match LAST_ERROR.with(Cell::get) {
                NO_ERROR => "No error",
                INTERNAL_ERROR => "Internal error",
                BAD_UTF8_ERROR => "Malformed UTF-8 characters, possibly incorrectly encoded",
                BAD_UTF8_OFFSET_ERROR => "The offset did not correspond to the beginning of a valid UTF-8 code point",
                BACKTRACK_LIMIT_ERROR => "Backtrack limit exhausted",
                RECURSION_LIMIT_ERROR => "Recursion limit exhausted",
                JIT_STACKLIMIT_ERROR => "JIT stack limit exhausted",
                _ => "Unknown error",
            };
            Ok((Value::str(msg), None))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}

enum ArrOrStr {
    Arr(Rc<PhpArray>),
    Str(Vec<u8>),
}

/// An `array|string` parameter
fn array_or_str(rt: &mut dyn Runtime, fname: &str, n: usize, param: &str, v: Value) -> EResult<ArrOrStr> {
    match v {
        Value::Array(a) => Ok(ArrOrStr::Arr(a)),
        Value::Str(s) => Ok(ArrOrStr::Str(s.to_vec())),
        Value::Null if !strict_types() => {
            null_param(fname, n, param, "array|string");
            Ok(ArrOrStr::Str(Vec::new()))
        }
        Value::Object(ref o) if o.class.find_method("__tostring").is_some() => Ok(ArrOrStr::Str(rt.stringify(&v)?)),
        Value::Int(_) | Value::Float(_) | Value::Bool(_) if !strict_types() => Ok(ArrOrStr::Str(v.to_bytes())),
        other => Err(param_type_error(fname, n, param, "array|string", &other)),
    }
}

fn match_impl(re: &Regex, s: &[u8], global: bool, flags: i64, offset: i64) -> EResult<(Value, Option<Value>)> {
    let fname = if global { "preg_match_all" } else { "preg_match" };
    let mut order = if global { PATTERN_ORDER } else { 0 };
    if flags & 0xff != 0 {
        order = flags & 0xff;
        if global && !(PATTERN_ORDER..=SET_ORDER).contains(&order) || !global && order != 0 {
            return Err(Fatal::value_error(format!("{fname}(): Argument #4 ($flags) must be a PREG_* constant")));
        }
    }
    let offset_capture = flags & OFFSET_CAPTURE != 0;
    let as_null = flags & UNMATCHED_AS_NULL != 0;
    let mut start = if offset < 0 {
        let back = offset.unsigned_abs() as usize;
        s.len().saturating_sub(back)
    } else {
        offset as usize
    };
    let mut subpats = PhpArray::new();
    if start > s.len() {
        set_error(INTERNAL_ERROR);
        return Ok((Value::Bool(false), Some(array_value(subpats))));
    }
    let num = re.capture_count + 1;
    let names = subpat_names(re);
    let mut sets: Vec<PhpArray> = if global && order == PATTERN_ORDER { (0..num).map(|_| PhpArray::new()).collect() } else { Vec::new() };
    let mut marks: Option<PhpArray> = None;
    let mut matched = 0;
    set_error(NO_ERROR);
    let mut result = exec(re, s, start, false, true);
    loop {
        let caps = match result {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                set_error(e);
                break;
            }
        };
        let mut caps = caps;
        loop {
            if caps.slots[1] < caps.slots[0] {
                // \K in a lookahead can end the match before its start
                diag::warning(format!("{fname}(): Get subpatterns list failed"));
                return Ok((Value::Bool(false), Some(array_value(subpats))));
            }
            matched += 1;
            let count = caps.count();
            if global && order == PATTERN_ORDER {
                for (i, set) in sets.iter_mut().enumerate().take(count) {
                    let range = caps.get(i);
                    push(set, if offset_capture { offset_pair(s, range, as_null) } else { substr(s, range, as_null) });
                }
                if let Some(mark) = &caps.mark {
                    marks.get_or_insert_with(PhpArray::new).insert(Key::Int(matched - 1), Value::str(mark.clone()));
                }
                for set in sets.iter_mut().skip(count) {
                    push(set, if offset_capture { offset_pair(s, None, as_null) } else { substr(s, None, as_null) });
                }
            } else if global {
                push(&mut subpats, array_value(subpat_array(s, &caps, &names, num, flags)));
            } else {
                subpats = subpat_array(s, &caps, &names, num, flags);
            }
            if !global {
                break;
            }
            let (m_start, m_end) = (caps.slots[0], caps.slots[1]);
            start = m_end;
            if m_end == m_start {
                // An empty match: try a non-empty one here, else move on a character
                match exec(re, s, start, true, false) {
                    Ok(Some(c)) => {
                        caps = c;
                        continue;
                    }
                    Ok(None) => {
                        if start >= s.len() {
                            start = usize::MAX;
                        } else {
                            start += unit_len(re, s, start);
                        }
                    }
                    Err(e) => {
                        set_error(e);
                        start = usize::MAX;
                    }
                }
            }
            break;
        }
        if !global || start == usize::MAX {
            break;
        }
        result = exec(re, s, start, false, false);
    }
    if global && order == PATTERN_ORDER {
        for (i, set) in sets.into_iter().enumerate() {
            let v = array_value(set);
            if let Some(Some(name)) = names.as_ref().map(|n| &n[i]) {
                subpats.insert(name_key(name), v.clone());
            }
            push(&mut subpats, v);
        }
        if let Some(marks) = marks {
            subpats.insert(name_key(b"MARK"), array_value(marks));
        }
    }
    let ret = if LAST_ERROR.with(Cell::get) == NO_ERROR { Value::Int(matched) } else { Value::Bool(false) };
    Ok((ret, Some(array_value(subpats))))
}

/// php-src's preg_get_backref: `\n`, `$n` or `${n}` with one or two digits
fn backref_at(r: &[u8], i: usize) -> Option<(usize, usize)> {
    let mut j = i;
    let braced = r[j] == b'$' && r.get(j + 1) == Some(&b'{');
    r.get(j + 1)?;
    if braced {
        j += 1;
    }
    j += 1;
    let d = *r.get(j).filter(|d| d.is_ascii_digit())?;
    let mut n = (d - b'0') as usize;
    j += 1;
    if let Some(d) = r.get(j).filter(|d| d.is_ascii_digit()) {
        n = n * 10 + (d - b'0') as usize;
        j += 1;
    }
    if braced {
        if r.get(j) != Some(&b'}') {
            return None;
        }
        j += 1;
    }
    Some((n, j))
}

/// The replacement for one match, with its back references filled in
fn expand(out: &mut Vec<u8>, s: &[u8], caps: &Captures, replacement: &[u8]) {
    let count = caps.count();
    let mut i = 0;
    let mut last = 0u8;
    while i < replacement.len() {
        let c = replacement[i];
        if c == b'\\' || c == b'$' {
            if last == b'\\' {
                // An escaped backslash or dollar replaces the backslash before it
                *out.last_mut().unwrap() = c;
                i += 1;
                last = 0;
                continue;
            }
            if let Some((n, next)) = backref_at(replacement, i) {
                if n < count
                    && let Some((a, b)) = caps.get(n)
                {
                    out.extend_from_slice(&s[a..b]);
                }
                i = next;
                continue;
            }
        }
        out.push(c);
        i += 1;
        last = c;
    }
}

enum Replacement<'a> {
    Str(&'a [u8]),
    Callback(&'a Value, i64),
}

/// php_pcre_replace_impl and php_pcre_replace_func_impl: every match of `re`
/// in `s` replaced, or None on an error
fn replace_one(
    rt: &mut dyn Runtime,
    re: &Regex,
    s: &[u8],
    with: &Replacement,
    mut limit: i64,
    count: &mut usize,
) -> EResult<Option<Vec<u8>>> {
    let num = re.capture_count + 1;
    let names = if matches!(with, Replacement::Callback(..)) { subpat_names(re) } else { None };
    let mut out = Vec::new();
    let mut last_end = 0;
    let mut start = 0;
    set_error(NO_ERROR);
    let mut result = exec(re, s, start, false, true);
    loop {
        let mut caps = match result {
            Ok(Some(c)) if limit != 0 => c,
            Ok(_) => {
                out.extend_from_slice(&s[last_end..]);
                return Ok(Some(out));
            }
            Err(e) => {
                set_error(e);
                return Ok(None);
            }
        };
        loop {
            let (m_start, m_end) = (caps.slots[0], caps.slots[1]);
            if m_end < m_start {
                set_error(INTERNAL_ERROR);
                return Ok(None);
            }
            *count += 1;
            out.extend_from_slice(&s[last_end..m_start]);
            match with {
                Replacement::Str(r) => expand(&mut out, s, &caps, r),
                Replacement::Callback(cb, flags) => {
                    let groups = array_value(subpat_array(s, &caps, &names, num, *flags));
                    let v = rt.call_callback(cb, vec![groups])?;
                    out.extend_from_slice(&rt.stringify(&v)?);
                }
            }
            limit -= 1;
            start = m_end;
            last_end = m_end;
            if m_end == m_start {
                let retry = exec(re, s, start, true, false);
                match retry {
                    Ok(Some(c)) if limit != 0 => {
                        caps = c;
                        continue;
                    }
                    Ok(_) => {
                        if start < s.len() {
                            start += unit_len(re, s, start);
                        } else {
                            out.extend_from_slice(&s[last_end..]);
                            return Ok(Some(out));
                        }
                    }
                    Err(e) => {
                        set_error(e);
                        return Ok(None);
                    }
                }
            }
            break;
        }
        result = exec(re, s, start, false, false);
    }
}

/// preg_replace on one subject, with one pattern or several in turn
fn replace_in_subject(
    rt: &mut dyn Runtime,
    fname: &str,
    regex: &ArrOrStr,
    replace: &ArrOrStr,
    subject: &[u8],
    limit: i64,
    count: &mut usize,
) -> EResult<Option<Vec<u8>>> {
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = match (regex, replace) {
        (ArrOrStr::Str(p), ArrOrStr::Str(r)) => vec![(p.clone(), r.clone())],
        (ArrOrStr::Arr(ps), ArrOrStr::Str(r)) => {
            let mut v = Vec::new();
            for p in ps.values() {
                v.push((rt.stringify(&p.copy_out())?, r.clone()));
            }
            v
        }
        (ArrOrStr::Arr(ps), ArrOrStr::Arr(rs)) => {
            let mut reps = rs.values();
            let mut v = Vec::new();
            for p in ps.values() {
                let p = rt.stringify(&p.copy_out())?;
                let r = match reps.next() {
                    Some(r) => rt.stringify(&r.copy_out())?,
                    None => Vec::new(),
                };
                v.push((p, r));
            }
            v
        }
        (ArrOrStr::Str(_), ArrOrStr::Arr(_)) => unreachable!("checked by the caller"),
    };
    let mut current = subject.to_vec();
    for (p, r) in pairs {
        let Some(re) = compile(fname, &p) else { return Ok(None) };
        match replace_one(rt, &re, &current, &Replacement::Str(&r), limit, count)? {
            Some(next) => current = next,
            None => return Ok(None),
        }
    }
    Ok(Some(current))
}

#[allow(clippy::too_many_arguments)]
fn replace_callback_subjects(
    rt: &mut dyn Runtime,
    fname: &str,
    patterns: &[Vec<u8>],
    callback: &Value,
    subject: ArrOrStr,
    limit: i64,
    flags: i64,
    count: &mut usize,
) -> EResult<Value> {
    let one = |rt: &mut dyn Runtime, s: &[u8], count: &mut usize| -> EResult<Option<Vec<u8>>> {
        let mut current = s.to_vec();
        for p in patterns {
            let Some(re) = compile(fname, p) else { return Ok(None) };
            match replace_one(rt, &re, &current, &Replacement::Callback(callback, flags), limit, count)? {
                Some(next) => current = next,
                None => return Ok(None),
            }
        }
        Ok(Some(current))
    };
    Ok(match subject {
        ArrOrStr::Str(s) => match one(rt, &s, count)? {
            Some(r) => Value::str(r),
            None => Value::Null,
        },
        ArrOrStr::Arr(a) => {
            let mut out = PhpArray::new();
            for (k, v) in a.iter() {
                let s = rt.stringify(&v.copy_out())?;
                if let Some(r) = one(rt, &s, count)? {
                    out.insert(k.clone(), Value::str(r));
                }
            }
            array_value(out)
        }
    })
}

/// php_pcre_split_impl
fn split(re: &Regex, s: &[u8], mut limit: i64, flags: i64) -> Value {
    let no_empty = flags & SPLIT_NO_EMPTY != 0;
    let delim_capture = flags & SPLIT_DELIM_CAPTURE != 0;
    let offset_capture = flags & SPLIT_OFFSET_CAPTURE != 0;
    let mut out = PhpArray::new();
    let piece = |out: &mut PhpArray, a: usize, b: usize| {
        if offset_capture {
            push(out, offset_pair(s, Some((a, b)), false));
        } else {
            push(out, Value::str(s[a..b].to_vec()));
        }
    };
    let mut last_match = 0;
    set_error(NO_ERROR);
    if limit == 0 {
        limit = -1;
    }
    if limit == -1 || limit > 1 {
        let mut start = 0;
        let mut result = exec(re, s, start, false, true);
        'outer: loop {
            let mut caps = match result {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(e) => {
                    set_error(e);
                    break;
                }
            };
            loop {
                let (m_start, m_end) = (caps.slots[0], caps.slots[1]);
                if m_end < m_start {
                    set_error(INTERNAL_ERROR);
                    break 'outer;
                }
                if !no_empty || m_start != last_match {
                    piece(&mut out, last_match, m_start);
                    if limit != -1 {
                        limit -= 1;
                    }
                }
                if delim_capture {
                    for i in 1..caps.count() {
                        let range = caps.get(i);
                        let (a, b) = range.unwrap_or((0, 0));
                        if !no_empty || a != b {
                            if offset_capture {
                                push(&mut out, offset_pair(s, range, false));
                            } else {
                                push(&mut out, Value::str(s[a..b].to_vec()));
                            }
                        }
                    }
                }
                start = m_end;
                last_match = m_end;
                if m_end == m_start {
                    if limit != -1 && limit <= 1 {
                        break 'outer;
                    }
                    match exec(re, s, start, true, false) {
                        Ok(Some(c)) => {
                            caps = c;
                            continue;
                        }
                        Ok(None) => {
                            if start < s.len() {
                                start += unit_len(re, s, start);
                            } else {
                                break 'outer;
                            }
                        }
                        Err(e) => {
                            set_error(e);
                            break 'outer;
                        }
                    }
                }
                break;
            }
            if limit != -1 && limit <= 1 {
                break;
            }
            result = exec(re, s, start, false, false);
        }
        if LAST_ERROR.with(Cell::get) != NO_ERROR {
            return Value::Bool(false);
        }
    }
    if !no_empty || last_match < s.len() {
        piece(&mut out, last_match, s.len());
    }
    array_value(out)
}

fn quote(s: &[u8], delim: Option<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        match c {
            b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')' | b'{' | b'}' | b'=' | b'!' | b'>'
            | b'<' | b'|' | b':' | b'-' | b'#' => {
                out.push(b'\\');
                out.push(c);
            }
            0 => out.extend_from_slice(b"\\000"),
            _ => {
                if Some(c) == delim {
                    out.push(b'\\');
                }
                out.push(c);
            }
        }
    }
    out
}
