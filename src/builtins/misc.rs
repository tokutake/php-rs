//! Assorted standard functions: version_compare(), strtok(), substr_compare(),
//! strip_tags(), the newer array_* helpers and a few others

use std::cell::RefCell;
use std::rc::Rc;

use super::{arg_bool, arg_int, arg_str, args, expect_array, Runtime};
use crate::array::{Key, PhpArray};
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "version_compare", "substr_compare", "strtok", "array_change_key_case", "array_replace_recursive", "array_first",
    "array_last", "array_any", "array_all", "array_find", "array_find_key", "utf8_encode", "utf8_decode", "gethostname",
    "sleep", "usleep", "strip_tags", "addcslashes", "stripcslashes", "quotemeta", "escapeshellarg", "escapeshellcmd",
    "setlocale", "timezone_identifiers_list", "iconv", "iconv_strlen", "iconv_substr", "iconv_strpos", "gethostbyname",
    "gethostbynamel", "sscanf",
];

/// The zones of timezone_identifiers_list(): zone.tab's, with their country
/// codes, sorted, then UTC
fn zone_list() -> Vec<(String, String)> {
    let tab = std::fs::read_to_string("/usr/share/zoneinfo/zone.tab").unwrap_or_default();
    let mut zones: Vec<(String, String)> = tab
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut f = l.split('\t');
            let cc = f.next()?;
            f.next()?;
            Some((f.next()?.to_string(), cc.to_string()))
        })
        .collect();
    zones.sort();
    zones.push(("UTC".into(), String::new()));
    zones
}

/// Converts between UTF-8, ISO-8859-1 and ASCII; None for an invalid input
/// sequence or a character the target cannot hold (unless //IGNORE or
/// //TRANSLIT says otherwise)
fn iconv(from: &str, to: &str, s: &[u8]) -> Option<Vec<u8>> {
    let norm = |e: &str| e.to_ascii_uppercase().replace(['-', '_'], "");
    let to_upper = to.to_ascii_uppercase();
    let translit = to_upper.contains("//TRANSLIT");
    let ignore = to_upper.contains("//IGNORE");
    let to = norm(to.split("//").next().unwrap_or(""));
    let from = norm(from.split("//").next().unwrap_or(""));
    let chars: Vec<char> = match from.as_str() {
        "UTF8" => match std::str::from_utf8(s) {
            Ok(t) => t.chars().collect(),
            Err(_) if ignore => String::from_utf8_lossy(s).chars().filter(|&c| c != '\u{fffd}').collect(),
            Err(_) => return None,
        },
        "ISO88591" | "LATIN1" => s.iter().map(|&b| b as char).collect(),
        "ASCII" | "USASCII" => {
            if !ignore && s.iter().any(|&b| b >= 128) {
                return None;
            }
            s.iter().filter(|&&b| b < 128).map(|&b| b as char).collect()
        }
        _ => return None,
    };
    let limit: u32 = match to.as_str() {
        "UTF8" => return Some(chars.iter().collect::<String>().into_bytes()),
        "ISO88591" | "LATIN1" => 0xff,
        "ASCII" | "USASCII" => 0x7f,
        _ => return None,
    };
    let mut out = Vec::with_capacity(chars.len());
    for c in chars {
        if (c as u32) <= limit {
            out.push(c as u8);
        } else if translit {
            // libiconv spells accented letters as the letter after an accent mark
            let base = match c {
                'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a", 'è' | 'é' | 'ê' | 'ë' => "e", 'ì' | 'í' | 'î' | 'ï' => "i",
                'ò' | 'ó' | 'ô' | 'õ' | 'ö' => "o", 'ù' | 'ú' | 'û' | 'ü' => "u", 'ç' => "c", 'ñ' => "n",
                _ => "?",
            };
            let mark = match c {
                'à' | 'è' | 'ì' | 'ò' | 'ù' => "`", 'á' | 'é' | 'í' | 'ó' | 'ú' => "'", 'â' | 'ê' | 'î' | 'ô' | 'û' => "^",
                'ã' | 'õ' | 'ñ' => "~", 'ä' | 'ë' | 'ï' | 'ö' | 'ü' => "\"", 'ç' => ",", _ => "",
            };
            out.extend_from_slice(base.as_bytes());
            let _ = mark;
            if base != "?" {
                let pos = out.len() - base.len();
                out.splice(pos..pos, mark.bytes());
            }
        } else if !ignore {
            return None;
        }
    }
    Some(out)
}

/// sscanf(): the values parsed by the format, null for those the input ran out before
fn sscanf(s: &[u8], f: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut failed = false;
    while j < f.len() {
        let c = f[j];
        if c.is_ascii_whitespace() {
            while i < s.len() && s[i].is_ascii_whitespace() {
                i += 1;
            }
            j += 1;
            continue;
        }
        if c != b'%' {
            if !failed && s.get(i) == Some(&c) {
                i += 1;
            } else {
                failed = true;
            }
            j += 1;
            continue;
        }
        j += 1;
        if f.get(j) == Some(&b'%') {
            if !failed && s.get(i) == Some(&b'%') {
                i += 1;
            } else {
                failed = true;
            }
            j += 1;
            continue;
        }
        let suppress = f.get(j) == Some(&b'*');
        if suppress {
            j += 1;
        }
        let wstart = j;
        while f.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        let width: Option<usize> = std::str::from_utf8(&f[wstart..j]).ok().and_then(|w| w.parse().ok());
        while matches!(f.get(j), Some(b'h' | b'l' | b'L')) {
            j += 1;
        }
        let Some(&conv) = f.get(j) else { break };
        j += 1;
        let mut set = None;
        if conv == b'[' {
            let start = j;
            if f.get(j) == Some(&b'^') {
                j += 1;
            }
            if f.get(j) == Some(&b']') {
                j += 1;
            }
            while j < f.len() && f[j] != b']' {
                j += 1;
            }
            set = Some(f[start..j].to_vec());
            j += 1;
        }
        let max = width.unwrap_or(usize::MAX);
        if failed {
            if !suppress && conv != b'n' {
                out.push(Value::Null);
            }
            continue;
        }
        if !matches!(conv, b'c' | b'[' | b'n') {
            while i < s.len() && s[i].is_ascii_whitespace() {
                i += 1;
            }
        }
        let start = i;
        let value = match conv {
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' => {
                let radix = match conv {
                    b'x' | b'X' => 16,
                    b'o' => 8,
                    _ => 10,
                };
                if matches!(s.get(i), Some(b'-' | b'+')) && i - start < max {
                    i += 1;
                }
                if radix == 16 && s[i..].len() >= 2 && s[i] == b'0' && matches!(s[i + 1], b'x' | b'X') {
                    i += 2;
                }
                let digits_start = i;
                while i < s.len() && i - start < max && (s[i] as char).is_digit(radix) {
                    i += 1;
                }
                if i == digits_start {
                    None
                } else {
                    let text = std::str::from_utf8(&s[digits_start..i]).unwrap_or("0");
                    let n = i64::from_str_radix(text, radix).unwrap_or(i64::MAX);
                    Some(Value::Int(if s[start] == b'-' { -n } else { n }))
                }
            }
            b'f' | b'e' | b'E' | b'g' => {
                while i < s.len() && i - start < max && (s[i].is_ascii_digit() || matches!(s[i], b'.' | b'-' | b'+' | b'e' | b'E')) {
                    i += 1;
                }
                std::str::from_utf8(&s[start..i]).ok().and_then(|t| t.parse::<f64>().ok()).map(Value::Float)
            }
            b's' => {
                while i < s.len() && i - start < max && !s[i].is_ascii_whitespace() {
                    i += 1;
                }
                (i > start).then(|| Value::str(s[start..i].to_vec()))
            }
            b'c' => {
                let n = width.unwrap_or(1);
                if i + n <= s.len() {
                    i += n;
                    Some(Value::str(s[start..i].to_vec()))
                } else {
                    None
                }
            }
            b'[' => {
                let set = set.unwrap_or_default();
                let (neg, set) = match set.split_first() {
                    Some((b'^', rest)) => (true, rest.to_vec()),
                    _ => (false, set),
                };
                let member = |c: u8| {
                    let mut k = 0;
                    while k < set.len() {
                        if k + 2 < set.len() && set[k + 1] == b'-' {
                            if (set[k]..=set[k + 2]).contains(&c) {
                                return true;
                            }
                            k += 3;
                        } else {
                            if set[k] == c {
                                return true;
                            }
                            k += 1;
                        }
                    }
                    false
                };
                while i < s.len() && i - start < max && member(s[i]) != neg {
                    i += 1;
                }
                (i > start).then(|| Value::str(s[start..i].to_vec()))
            }
            b'n' => {
                if !suppress {
                    out.push(Value::Int(i as i64));
                }
                continue;
            }
            _ => None,
        };
        match value {
            Some(v) if !suppress => out.push(v),
            Some(_) => {}
            None => {
                failed = true;
                if !suppress {
                    out.push(Value::Null);
                }
            }
        }
    }
    out
}

pub fn constant(name: &str) -> Option<Value> {
    match name {
        "CASE_LOWER" => Some(Value::Int(0)),
        "CASE_UPPER" => Some(Value::Int(1)),
        // The C library's numbering, which differs between systems
        "LC_ALL" => Some(Value::Int(if cfg!(target_os = "macos") { 0 } else { 6 })),
        "LC_COLLATE" => Some(Value::Int(if cfg!(target_os = "macos") { 1 } else { 3 })),
        "LC_CTYPE" => Some(Value::Int(if cfg!(target_os = "macos") { 2 } else { 0 })),
        "LC_MONETARY" => Some(Value::Int(if cfg!(target_os = "macos") { 3 } else { 4 })),
        "LC_NUMERIC" => Some(Value::Int(if cfg!(target_os = "macos") { 4 } else { 1 })),
        "LC_TIME" => Some(Value::Int(if cfg!(target_os = "macos") { 5 } else { 2 })),
        "LC_MESSAGES" => Some(Value::Int(if cfg!(target_os = "macos") { 6 } else { 5 })),
        _ => None,
    }
}

thread_local! {
    /// strtok()'s string and position between calls
    static STRTOK: RefCell<(Vec<u8>, usize)> = const { RefCell::new((Vec::new(), 0)) };
}

/// php_canonicalize_version: separators become dots, and dots go between
/// digits and other characters
fn canonicalize(v: &[u8]) -> Vec<u8> {
    let Some((&first, rest)) = v.split_first() else { return Vec::new() };
    let isdig = |c: u8| c.is_ascii_digit();
    let isndig = |c: u8| !c.is_ascii_digit() && c != b'.';
    let mut out = vec![first];
    let mut lp = first;
    for &c in rest {
        let lq = *out.last().unwrap();
        if matches!(c, b'-' | b'_' | b'+') {
            if lq != b'.' {
                out.push(b'.');
            }
        } else if (isndig(lp) && isdig(c)) || (isdig(lp) && isndig(c)) {
            if lq != b'.' {
                out.push(b'.');
            }
            out.push(c);
        } else if !c.is_ascii_alphanumeric() {
            if lq != b'.' {
                out.push(b'.');
            }
        } else {
            out.push(c);
        }
        lp = c;
    }
    if out.last() == Some(&b'.') {
        out.pop();
    }
    out
}

fn special_form(form: &[u8]) -> i32 {
    const FORMS: &[(&[u8], i32)] =
        &[(b"dev", 0), (b"alpha", 1), (b"a", 1), (b"beta", 2), (b"b", 2), (b"RC", 3), (b"rc", 3), (b"#", 4), (b"pl", 5), (b"p", 5)];
    FORMS.iter().find(|(name, _)| form.starts_with(name)).map_or(-1, |(_, o)| *o)
}

fn compare_forms(a: &[u8], b: &[u8]) -> i32 {
    (special_form(a) - special_form(b)).signum()
}

/// strtol on a component: its leading digits
fn component_number(s: &[u8]) -> i64 {
    s.iter().take_while(|c| c.is_ascii_digit()).fold(0i64, |n, &c| n.saturating_mul(10).saturating_add((c - b'0') as i64))
}

/// php_version_compare
pub fn version_compare(v1: &[u8], v2: &[u8]) -> i32 {
    if v1.is_empty() || v2.is_empty() {
        return if v1.is_empty() && v2.is_empty() { 0 } else if v1.is_empty() { -1 } else { 1 };
    }
    let a = if v1[0] == b'#' { v1.to_vec() } else { canonicalize(v1) };
    let b = if v2[0] == b'#' { v2.to_vec() } else { canonicalize(v2) };
    let (mut p1, mut p2) = (0usize, 0usize);
    let (mut n1, mut n2): (Option<usize>, Option<usize>) = (Some(0), Some(0));
    let mut compare = 0;
    let digit = |s: &[u8], p: usize| s.get(p).is_some_and(u8::is_ascii_digit);
    while p1 < a.len() && p2 < b.len() && n1.is_some() && n2.is_some() {
        n1 = a[p1..].iter().position(|&c| c == b'.').map(|i| p1 + i);
        n2 = b[p2..].iter().position(|&c| c == b'.').map(|i| p2 + i);
        let c1 = &a[p1..n1.unwrap_or(a.len())];
        let c2 = &b[p2..n2.unwrap_or(b.len())];
        compare = match (digit(&a, p1), digit(&b, p2)) {
            (true, true) => (component_number(c1) - component_number(c2)).signum() as i32,
            (false, false) => compare_forms(c1, c2),
            (true, false) => compare_forms(b"#N#", c2),
            (false, true) => compare_forms(c1, b"#N#"),
        };
        if compare != 0 {
            break;
        }
        if let Some(n) = n1 {
            p1 = n + 1;
        }
        if let Some(n) = n2 {
            p2 = n + 1;
        }
    }
    if compare == 0 {
        if n1.is_some() {
            compare = if digit(&a, p1) { 1 } else { version_compare(&a[p1..], b"#N#") };
        } else if n2.is_some() {
            compare = if digit(&b, p2) { -1 } else { version_compare(b"#N#", &b[p2..]) };
        }
    }
    compare
}

/// strip_tags: removes HTML and PHP tags and comments, keeping the allowed tags
pub fn strip_tags(s: &[u8], allowed: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c != b'<' || s.get(i + 1).is_none_or(|n| n.is_ascii_whitespace()) {
            out.push(c);
            i += 1;
            continue;
        }
        if s[i..].starts_with(b"<!--") {
            i = s[i + 4..].windows(3).position(|w| w == b"-->").map_or(s.len(), |p| i + 4 + p + 3);
            continue;
        }
        // A tag runs to the `>` outside quotes
        let start = i;
        let mut quote = None;
        let mut depth = 0;
        let mut j = i + 1;
        while j < s.len() {
            let d = s[j];
            match quote {
                Some(q) if d == q => quote = None,
                Some(_) => {}
                None if d == b'"' || d == b'\'' => quote = Some(d),
                None if d == b'<' => depth += 1,
                None if d == b'>' && depth > 0 => depth -= 1,
                None if d == b'>' => break,
                None => {}
            }
            j += 1;
        }
        let end = (j + 1).min(s.len());
        let tag = &s[start..end];
        if !allowed.is_empty() {
            let name: Vec<u8> = tag[1..]
                .iter()
                .skip_while(|&&c| c == b'/')
                .take_while(|c| c.is_ascii_alphanumeric())
                .map(u8::to_ascii_lowercase)
                .collect();
            if !name.is_empty() && allowed.contains(&name) {
                out.extend_from_slice(tag);
            }
        }
        i = end;
    }
    out
}

/// The tag names of strip_tags()'s allowed list: `"<a><b>"` or an array
fn allowed_tags(rt: &mut dyn Runtime, v: Option<Value>) -> EResult<Vec<Vec<u8>>> {
    let names = match v.map(|v| v.copy_out()) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => {
            let mut out = Vec::new();
            for v in a.values() {
                out.push(arg_str(rt, "strip_tags", 2, "allowed_tags", v)?.to_ascii_lowercase());
            }
            return Ok(out);
        }
        Some(v) => arg_str(rt, "strip_tags", 2, "allowed_tags", &v)?,
    };
    Ok(names
        .split(|&c| c == b'<')
        .filter_map(|t| {
            let name: Vec<u8> = t.iter().take_while(|&&c| c != b'>').map(u8::to_ascii_lowercase).collect();
            (!name.is_empty()).then_some(name)
        })
        .collect())
}

/// The characters addcslashes() takes, with `a..z` ranges
fn char_list(chars: &[u8]) -> [bool; 256] {
    let mut mask = [false; 256];
    let mut i = 0;
    while i < chars.len() {
        if i + 3 < chars.len() && &chars[i + 1..i + 3] == b".." && chars[i + 3] >= chars[i] {
            for c in chars[i]..=chars[i + 3] {
                mask[c as usize] = true;
            }
            i += 4;
        } else {
            mask[chars[i] as usize] = true;
            i += 1;
        }
    }
    mask
}

fn replace_recursive(base: &PhpArray, with: &PhpArray, depth: usize) -> PhpArray {
    let mut out = base.clone();
    for (k, v) in with.iter() {
        let merged = match (out.get(k).map(Value::copy_out), v.copy_out()) {
            (Some(Value::Array(a)), Value::Array(b)) if depth < 256 => Value::Array(Rc::new(replace_recursive(&a, &b, depth + 1))),
            (_, v) => v,
        };
        out.insert(k.clone(), merged);
    }
    out
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "version_compare" => {
            let [a, b, op] = args::<2, 3>(name, vals)?;
            let a = arg_str(rt, lname, 1, "version1", &a.unwrap())?;
            let b = arg_str(rt, lname, 2, "version2", &b.unwrap())?;
            // C strings: a NUL ends them
            let cut = |s: &[u8]| s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())].to_vec();
            let c = version_compare(&cut(&a), &cut(&b));
            let Some(op) = op.filter(|o| !o.is_null()) else { return Ok(Value::Int(c as i64)) };
            let op = arg_str(rt, lname, 3, "operator", &op)?;
            Ok(Value::Bool(match op.as_slice() {
                b"<" | b"lt" => c == -1,
                b"<=" | b"le" => c != 1,
                b">" | b"gt" => c == 1,
                b">=" | b"ge" => c != -1,
                b"==" | b"=" | b"eq" => c == 0,
                b"!=" | b"<>" | b"ne" => c != 0,
                _ => return Err(Fatal::value_error("version_compare(): Argument #3 ($operator) must be a valid comparison operator")),
            }))
        }
        "substr_compare" => {
            let [h, n, off, len, ci] = args::<3, 5>(name, vals)?;
            let h = arg_str(rt, lname, 1, "haystack", &h.unwrap())?;
            let n = arg_str(rt, lname, 2, "needle", &n.unwrap())?;
            let mut off = arg_int(lname, 3, "offset", &off.unwrap())?;
            let len = match len.filter(|l| !l.is_null()) {
                Some(l) => {
                    let l = arg_int(lname, 4, "length", &l)?;
                    if l < 0 {
                        return Err(Fatal::value_error("substr_compare(): Argument #4 ($length) must be greater than or equal to 0"));
                    }
                    Some(l as usize)
                }
                None => None,
            };
            let ci = match ci {
                Some(c) => arg_bool(lname, 5, "case_insensitive", &c)?,
                None => false,
            };
            if off < 0 {
                off = (h.len() as i64 + off).max(0);
            }
            if off as usize > h.len() {
                return Err(Fatal::value_error("substr_compare(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"));
            }
            let a = &h[off as usize..];
            let cmp_len = len.unwrap_or_else(|| a.len().max(n.len()));
            let a = &a[..cmp_len.min(a.len())];
            let b = &n[..cmp_len.min(n.len())];
            let (a, b) = if ci { (a.to_ascii_lowercase(), b.to_ascii_lowercase()) } else { (a.to_vec(), b.to_vec()) };
            Ok(Value::Int(match a.cmp(&b) {
                std::cmp::Ordering::Equal => 0,
                // php-src returns the byte difference or the length difference
                _ => match a.iter().zip(&b).find(|(x, y)| x != y) {
                    Some((x, y)) => *x as i64 - *y as i64,
                    None => a.len() as i64 - b.len() as i64,
                },
            }))
        }
        "strtok" => {
            let [first, second] = args::<1, 2>(name, vals)?;
            let (token, reset) = match second {
                Some(t) => (arg_str(rt, lname, 2, "token", &t)?, Some(arg_str(rt, lname, 1, "string", &first.unwrap())?)),
                None => (arg_str(rt, lname, 1, "string", &first.unwrap())?, None),
            };
            STRTOK.with(|st| {
                let mut st = st.borrow_mut();
                if let Some(s) = reset {
                    *st = (s, 0);
                }
                let (s, pos) = &mut *st;
                let delim = |c: &u8| token.contains(c);
                while *pos < s.len() && delim(&s[*pos]) {
                    *pos += 1;
                }
                if *pos >= s.len() {
                    return Ok(Value::Bool(false));
                }
                let start = *pos;
                while *pos < s.len() && !delim(&s[*pos]) {
                    *pos += 1;
                }
                let tok = s[start..*pos].to_vec();
                // The delimiter after the token is consumed
                if *pos < s.len() {
                    *pos += 1;
                }
                Ok(Value::str(tok))
            })
        }
        "array_change_key_case" => {
            let [a, case] = args::<1, 2>(name, vals)?;
            let a = expect_array(lname, 1, "array", &a.unwrap())?;
            let upper = match case {
                Some(c) => arg_int(lname, 2, "case", &c)? != 0,
                None => false,
            };
            let mut out = PhpArray::new();
            for (k, v) in a.iter() {
                let k = match k {
                    Key::Str(s) => Key::Str(if upper { s.to_ascii_uppercase() } else { s.to_ascii_lowercase() }.into()),
                    k => k.clone(),
                };
                out.insert(k, v.clone());
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_replace_recursive" => {
            let mut it = vals.into_iter();
            let Some(first) = it.next() else { return Err(super::arg_count(lname, 1, 0)) };
            let mut out = (*expect_array(lname, 1, "array", &first)?).clone();
            for (i, v) in it.enumerate() {
                let b = expect_array(lname, i + 2, "replacements", &v)?;
                out = replace_recursive(&out, &b, 0);
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "array_first" | "array_last" => {
            let [a] = args::<1, 1>(name, vals)?;
            let a = expect_array(lname, 1, "array", &a.unwrap())?;
            let v = if lname == "array_first" { a.values().next() } else { a.iter().next_back().map(|(_, v)| v) };
            Ok(v.map_or(Value::Null, Value::copy_out))
        }
        "array_any" | "array_all" | "array_find" | "array_find_key" => {
            let [a, f] = args::<2, 2>(name, vals)?;
            let a = expect_array(lname, 1, "array", &a.unwrap())?;
            let f = f.unwrap();
            rt.check_callback(lname, 2, &f)?;
            for (k, v) in a.iter() {
                let hit = rt.call_callback(&f, vec![v.copy_out(), k.to_value()])?.to_bool();
                match (lname, hit) {
                    ("array_any", true) => return Ok(Value::Bool(true)),
                    ("array_all", false) => return Ok(Value::Bool(false)),
                    ("array_find", true) => return Ok(v.copy_out()),
                    ("array_find_key", true) => return Ok(k.to_value()),
                    _ => {}
                }
            }
            Ok(match lname {
                "array_any" => Value::Bool(false),
                "array_all" => Value::Bool(true),
                _ => Value::Null,
            })
        }
        "utf8_encode" | "utf8_decode" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            crate::diag::deprecated(format!("Function {lname}() is deprecated since 8.2, visit the php.net documentation for various alternatives"));
            if lname == "utf8_encode" {
                return Ok(Value::str(s.iter().map(|&b| b as char).collect::<String>()));
            }
            // Characters beyond Latin-1, and malformed bytes, become `?`
            let text = String::from_utf8_lossy(&s);
            Ok(Value::str(text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect::<Vec<u8>>()))
        }
        "escapeshellarg" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "arg", &s.unwrap())?;
            let mut out = vec![b'\''];
            for &c in &s {
                if c == b'\'' {
                    out.extend_from_slice(b"'\\''");
                } else {
                    out.push(c);
                }
            }
            out.push(b'\'');
            Ok(Value::str(out))
        }
        "escapeshellcmd" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "command", &s.unwrap())?;
            // Quotes are escaped only when unpaired
            let paired = |q: u8| s.iter().filter(|&&c| c == q).count() % 2 == 0;
            let mut out = Vec::with_capacity(s.len());
            for &c in &s {
                let quote_ok = (c == b'\'' || c == b'"') && paired(c);
                if b"#&;`|*?~<>^()[]{}$\\,\x0A\xFF\"'".contains(&c) && !quote_ok {
                    out.push(b'\\');
                }
                out.push(c);
            }
            Ok(Value::str(out))
        }
        "setlocale" => {
            let mut it = vals.into_iter();
            let Some(_category) = it.next() else { return Err(super::arg_count(name, 2, 0)) };
            let mut wanted = Vec::new();
            for v in it {
                match v.copy_out() {
                    Value::Array(a) => wanted.extend(a.values().map(Value::to_bytes)),
                    v => wanted.push(rt.stringify(&v)?),
                }
            }
            // Only the C locale exists here; "0" asks for the current one
            for w in wanted {
                match w.as_slice() {
                    b"0" | b"" | b"C" | b"POSIX" => return Ok(Value::str("C")),
                    _ => {}
                }
            }
            Ok(Value::Bool(false))
        }
        "timezone_identifiers_list" => {
            let [group, country] = args::<0, 2>(name, vals)?;
            let group = match group {
                Some(g) => arg_int(lname, 1, "timezoneGroup", &g)?,
                None => 2047,
            };
            let country = match country {
                Some(c) if !c.is_null() => Some(String::from_utf8_lossy(&arg_str(rt, lname, 2, "countryCode", &c)?).to_ascii_uppercase()),
                _ => None,
            };
            const GROUPS: &[(&str, i64)] = &[
                ("Africa/", 1), ("America/", 2), ("Antarctica/", 4), ("Arctic/", 8), ("Asia/", 16), ("Atlantic/", 32),
                ("Australia/", 64), ("Europe/", 128), ("Indian/", 256), ("Pacific/", 512),
            ];
            let zones = zone_list().into_iter().filter(|(z, cc)| {
                if group == 4096 {
                    return country.as_deref() == Some(cc.as_str());
                }
                if z == "UTC" {
                    return group & 1024 != 0;
                }
                GROUPS.iter().any(|(p, bit)| group & bit != 0 && z.starts_with(p))
            });
            Ok(super::list(zones.map(|(z, _)| Value::str(z))))
        }
        "iconv" => {
            let [from, to, s] = args::<3, 3>(name, vals)?;
            let from = String::from_utf8_lossy(&arg_str(rt, lname, 1, "from_encoding", &from.unwrap())?).into_owned();
            let to = String::from_utf8_lossy(&arg_str(rt, lname, 2, "to_encoding", &to.unwrap())?).into_owned();
            let s = arg_str(rt, lname, 3, "string", &s.unwrap())?;
            match iconv(&from, &to, &s) {
                Some(out) => Ok(Value::str(out)),
                None => {
                    crate::diag::notice("iconv(): Detected an illegal character in input string");
                    Ok(Value::Bool(false))
                }
            }
        }
        "iconv_strlen" | "iconv_substr" | "iconv_strpos" => {
            let mut it = vals.into_iter();
            let s = arg_str(rt, lname, 1, "string", &it.next().unwrap_or(Value::Null))?;
            let Ok(text) = std::str::from_utf8(&s) else {
                crate::diag::notice(format!("{lname}(): Detected an illegal character in input string"));
                return Ok(Value::Bool(false));
            };
            let chars: Vec<char> = text.chars().collect();
            match lname {
                "iconv_strlen" => Ok(Value::Int(chars.len() as i64)),
                "iconv_substr" => {
                    let off = it.next().map_or(Ok(0), |v| arg_int(lname, 2, "offset", &v))?;
                    let len = match it.next() {
                        Some(v) if !v.is_null() => Some(arg_int(lname, 3, "length", &v)?),
                        _ => None,
                    };
                    let n = chars.len() as i64;
                    let start = if off < 0 { (n + off).max(0) } else { off.min(n) };
                    let end = match len {
                        Some(l) if l < 0 => (n + l).max(start),
                        Some(l) => (start + l).min(n),
                        None => n,
                    };
                    Ok(Value::str(chars[start as usize..end as usize].iter().collect::<String>()))
                }
                _ => {
                    let needle = arg_str(rt, lname, 2, "needle", &it.next().unwrap_or(Value::Null))?;
                    let needle = String::from_utf8_lossy(&needle).into_owned();
                    let off = it.next().map_or(Ok(0), |v| arg_int(lname, 3, "offset", &v))?.max(0) as usize;
                    let rest: String = chars.iter().skip(off).collect();
                    Ok(rest.find(&needle).map_or(Value::Bool(false), |b| Value::Int((off + rest[..b].chars().count()) as i64)))
                }
            }
        }
        "gethostbyname" | "gethostbynamel" => {
            let [host] = args::<1, 1>(name, vals)?;
            let host = String::from_utf8_lossy(&arg_str(rt, lname, 1, "hostname", &host.unwrap())?).into_owned();
            use std::net::ToSocketAddrs;
            let addrs: Vec<String> = (host.as_str(), 0)
                .to_socket_addrs()
                .map(|a| a.filter(|a| a.is_ipv4()).map(|a| a.ip().to_string()).collect())
                .unwrap_or_default();
            Ok(match lname {
                "gethostbyname" => Value::str(addrs.first().cloned().unwrap_or(host)),
                _ if addrs.is_empty() => Value::Bool(false),
                _ => super::list(addrs.into_iter().map(Value::str)),
            })
        }
        "sscanf" => {
            let mut it = vals.into_iter();
            let s = arg_str(rt, lname, 1, "string", &it.next().unwrap_or(Value::Null))?;
            let f = arg_str(rt, lname, 2, "format", &it.next().unwrap_or(Value::Null))?;
            let values = sscanf(&s, &f);
            // Without the input, every conversion is -1 in php-src
            if s.is_empty() {
                return Ok(Value::Int(-1));
            }
            Ok(super::list(values))
        }
        "gethostname" => {
            args::<0, 0>(name, vals)?;
            let host = std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_string())
                .or_else(|| std::env::var("HOSTNAME").ok())
                .or_else(|| {
                    std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                })
                .unwrap_or_else(|| "localhost".into());
            Ok(Value::str(host))
        }
        "sleep" | "usleep" => {
            let [n] = args::<1, 1>(name, vals)?;
            let n = arg_int(lname, 1, if lname == "sleep" { "seconds" } else { "microseconds" }, &n.unwrap())?;
            if n < 0 {
                return Err(Fatal::value_error(format!(
                    "{lname}(): Argument #1 (${}) must be greater than or equal to 0",
                    if lname == "sleep" { "seconds" } else { "microseconds" }
                )));
            }
            let d = if lname == "sleep" { std::time::Duration::from_secs(n as u64) } else { std::time::Duration::from_micros(n as u64) };
            std::thread::sleep(d);
            Ok(if lname == "sleep" { Value::Int(0) } else { Value::Null })
        }
        "strip_tags" => {
            let [s, allowed] = args::<1, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let allowed = allowed_tags(rt, allowed)?;
            Ok(Value::str(strip_tags(&s, &allowed)))
        }
        "addcslashes" => {
            let [s, chars] = args::<2, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let mask = char_list(&arg_str(rt, lname, 2, "characters", &chars.unwrap())?);
            let mut out = Vec::with_capacity(s.len());
            for &c in &s {
                if !mask[c as usize] {
                    out.push(c);
                    continue;
                }
                out.push(b'\\');
                match c {
                    b'\n' => out.push(b'n'),
                    b'\t' => out.push(b't'),
                    b'\r' => out.push(b'r'),
                    0x07 => out.push(b'a'),
                    0x0b => out.push(b'v'),
                    0x08 => out.push(b'b'),
                    0x0c => out.push(b'f'),
                    c if !(32..127).contains(&c) => out.extend_from_slice(format!("{c:03o}").as_bytes()),
                    c => out.push(c),
                }
            }
            Ok(Value::str(out))
        }
        "stripcslashes" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let mut out = Vec::with_capacity(s.len());
            let mut i = 0;
            while i < s.len() {
                if s[i] != b'\\' || i + 1 >= s.len() {
                    out.push(s[i]);
                    i += 1;
                    continue;
                }
                i += 1;
                let c = s[i];
                i += 1;
                match c {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'a' => out.push(0x07),
                    b'v' => out.push(0x0b),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'x' if s.get(i).is_some_and(u8::is_ascii_hexdigit) => {
                        let n = s[i..].iter().take(2).take_while(|c| c.is_ascii_hexdigit()).count();
                        out.push(u8::from_str_radix(std::str::from_utf8(&s[i..i + n]).unwrap_or("0"), 16).unwrap_or(0));
                        i += n;
                    }
                    b'0'..=b'7' => {
                        let n = s[i - 1..].iter().take(3).take_while(|c| (b'0'..=b'7').contains(c)).count();
                        let v = s[i - 1..i - 1 + n].iter().fold(0u32, |v, &c| v * 8 + (c - b'0') as u32);
                        out.push(v as u8);
                        i += n - 1;
                    }
                    c => out.push(c),
                }
            }
            Ok(Value::str(out))
        }
        "quotemeta" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let mut out = Vec::with_capacity(s.len());
            for &c in &s {
                if b".\\+*?[^]$()".contains(&c) {
                    out.push(b'\\');
                }
                out.push(c);
            }
            Ok(Value::str(out))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
