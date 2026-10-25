//! String functions.

use std::rc::Rc;

use super::{arg_bool, arg_int, arg_str, args, list, Runtime};
use crate::array::PhpArray;
use crate::diag;
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "str_pad", "ucwords", "wordwrap", "str_replace", "str_ireplace", "substr_replace", "strtr", "nl2br",
    "htmlspecialchars", "htmlspecialchars_decode", "addslashes", "stripslashes", "strstr", "stristr", "strchr",
    "strrchr", "stripos", "strrpos", "strripos", "substr_count", "strpbrk", "strspn", "strcspn", "strcmp",
    "strcasecmp", "strncmp", "strncasecmp", "strnatcmp", "strnatcasecmp", "str_split", "chunk_split",
    "str_word_count", "similar_text", "levenshtein", "chr", "ord", "bin2hex", "hex2bin", "crc32", "ctype_digit",
    "ctype_alpha", "ctype_alnum", "ctype_upper", "ctype_lower", "ctype_space", "ctype_xdigit", "ctype_punct",
];

pub const STR_PAD_LEFT: i64 = 0;
pub const STR_PAD_RIGHT: i64 = 1;
pub const STR_PAD_BOTH: i64 = 2;
pub const ENT_COMPAT: i64 = 2;
pub const ENT_QUOTES: i64 = 3;
pub const ENT_NOQUOTES: i64 = 0;
pub const ENT_IGNORE: i64 = 4;
pub const ENT_SUBSTITUTE: i64 = 8;
pub const ENT_HTML401: i64 = 0;
pub const ENT_XML1: i64 = 16;
pub const ENT_XHTML: i64 = 32;
pub const ENT_HTML5: i64 = 48;

/// Constants this module defines
pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "STR_PAD_LEFT" => STR_PAD_LEFT,
        "STR_PAD_RIGHT" => STR_PAD_RIGHT,
        "STR_PAD_BOTH" => STR_PAD_BOTH,
        "ENT_COMPAT" => ENT_COMPAT,
        "ENT_QUOTES" => ENT_QUOTES,
        "ENT_NOQUOTES" => ENT_NOQUOTES,
        "ENT_IGNORE" => ENT_IGNORE,
        "ENT_SUBSTITUTE" => ENT_SUBSTITUTE,
        "ENT_HTML401" => ENT_HTML401,
        "ENT_XML1" => ENT_XML1,
        "ENT_XHTML" => ENT_XHTML,
        "ENT_HTML5" => ENT_HTML5,
        _ => return None,
    }))
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(0);
    }
    h.windows(n.len()).position(|w| w == n)
}

fn rfind(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(h.len());
    }
    h.windows(n.len()).rposition(|w| w == n)
}

fn lower(s: &[u8]) -> Vec<u8> {
    s.to_ascii_lowercase()
}

fn sign(o: std::cmp::Ordering) -> Value {
    Value::Int(o as i64)
}

/// A character set as trim() and friends take it: "a..z" is a range
pub fn char_mask(fname: &str, chars: &[u8]) -> [bool; 256] {
    let mut mask = [false; 256];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if i + 3 < chars.len() && chars[i + 1] == b'.' && chars[i + 2] == b'.' && chars[i + 3] >= c {
            for b in c..=chars[i + 3] {
                mask[b as usize] = true;
            }
            i += 4;
            continue;
        }
        if c == b'.' && i + 1 < chars.len() && chars[i + 1] == b'.' {
            let msg = if i == 0 {
                "Invalid '..'-range, no character to the left of '..'"
            } else if i + 2 >= chars.len() {
                "Invalid '..'-range, no character to the right of '..'"
            } else {
                "Invalid '..'-range, '..'-range needs to be incrementing"
            };
            diag::warning(format!("{fname}(): {msg}"));
        }
        mask[c as usize] = true;
        i += 1;
    }
    mask
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "str_pad" => {
            let [s, len, pad, ty] = args::<2, 4>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let len = arg_int(lname, 2, "length", &len.unwrap())?;
            let pad = match pad {
                Some(p) => arg_str(rt, lname, 3, "pad_string", &p)?,
                None => b" ".to_vec(),
            };
            let ty = match ty {
                Some(t) => arg_int(lname, 4, "pad_type", &t)?,
                None => STR_PAD_RIGHT,
            };
            if len <= s.len() as i64 {
                return Ok(Value::str(s));
            }
            if pad.is_empty() {
                return Err(Fatal::value_error("str_pad(): Argument #3 ($pad_string) must be a non-empty string"));
            }
            if !(STR_PAD_LEFT..=STR_PAD_BOTH).contains(&ty) {
                return Err(Fatal::value_error(
                    "str_pad(): Argument #4 ($pad_type) must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH",
                ));
            }
            let total = (len as usize).saturating_sub(s.len());
            if total > isize::MAX as usize / 2 {
                return Err(Fatal::error("Possible integer overflow in memory allocation"));
            }
            let (left, right) = match ty {
                STR_PAD_LEFT => (total, 0),
                STR_PAD_RIGHT => (0, total),
                _ => (total / 2, total - total / 2),
            };
            let fill = |n: usize| pad.iter().cycle().take(n).copied().collect::<Vec<u8>>();
            let mut out = fill(left);
            out.extend_from_slice(&s);
            out.extend(fill(right));
            Ok(Value::str(out))
        }
        "ucwords" => {
            let [s, delims] = args::<1, 2>(name, vals)?;
            let mut s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let delims = match delims {
                Some(d) => arg_str(rt, lname, 2, "separators", &d)?,
                None => b" \t\r\n\x0c\x0b".to_vec(),
            };
            let mask = char_mask(lname, &delims);
            let mut at_start = true;
            for c in s.iter_mut() {
                if at_start {
                    c.make_ascii_uppercase();
                }
                at_start = mask[*c as usize];
            }
            Ok(Value::str(s))
        }
        "wordwrap" => {
            let [s, width, brk, cut] = args::<1, 4>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let width = match width {
                Some(w) => arg_int(lname, 2, "width", &w)?,
                None => 75,
            };
            let brk = match brk {
                Some(b) => arg_str(rt, lname, 3, "break", &b)?,
                None => b"\n".to_vec(),
            };
            let cut = match cut {
                Some(c) => arg_bool(lname, 4, "cut_long_words", &c)?,
                None => false,
            };
            if s.is_empty() {
                return Ok(Value::str(""));
            }
            if brk.is_empty() {
                return Err(Fatal::value_error("wordwrap(): Argument #3 ($break) cannot be empty"));
            }
            if width == 0 && cut {
                return Err(Fatal::value_error("wordwrap(): Argument #4 ($cut_long_words) cannot be true when argument #2 ($width) is 0"));
            }
            Ok(Value::str(wordwrap(&s, width, &brk, cut)))
        }
        "str_replace" | "str_ireplace" => {
            let [search, replace, subject, _count] = args::<3, 4>(name, vals)?;
            let (v, _) = str_replace(rt, lname, search.unwrap(), replace.unwrap(), subject.unwrap())?;
            Ok(v)
        }
        "substr_replace" => {
            let [s, rep, off, len] = args::<3, 4>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let rep = arg_str(rt, lname, 2, "replace", &rep.unwrap())?;
            let n = s.len() as i64;
            let off = arg_int(lname, 3, "offset", &off.unwrap())?;
            let start = if off < 0 { n.saturating_add(off).max(0) } else { off.min(n) };
            let end = match len {
                None | Some(Value::Null) => n,
                Some(l) => {
                    let l = arg_int(lname, 4, "length", &l)?;
                    if l < 0 { n.saturating_add(l).max(start) } else { start.saturating_add(l).min(n) }
                }
            };
            let mut out = s[..start as usize].to_vec();
            out.extend_from_slice(&rep);
            out.extend_from_slice(&s[end.max(start) as usize..]);
            Ok(Value::str(out))
        }
        "strtr" => {
            let [s, from, to] = args::<2, 3>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            match (from.unwrap(), to) {
                (Value::Array(pairs), None) => {
                    let mut map: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
                    for (k, v) in pairs.iter() {
                        let k = k.to_value().to_bytes();
                        if k.is_empty() {
                            continue;
                        }
                        map.push((k, rt.stringify(&v.copy_out())?));
                    }
                    Ok(Value::str(strtr_pairs(&s, map)))
                }
                (from, Some(to)) => {
                    let from = arg_str(rt, lname, 2, "from", &from)?;
                    let to = arg_str(rt, lname, 3, "to", &to)?;
                    let mut table: [u8; 256] = std::array::from_fn(|i| i as u8);
                    for (f, t) in from.iter().zip(&to) {
                        table[*f as usize] = *t;
                    }
                    Ok(Value::str(s.iter().map(|c| table[*c as usize]).collect::<Vec<u8>>()))
                }
                (other, None) => Err(Fatal::type_error(format!(
                    "strtr(): Argument #2 ($from) must be of type array, {} given",
                    other.value_name()
                ))),
            }
        }
        "nl2br" => {
            let [s, xhtml] = args::<1, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let xhtml = match xhtml {
                Some(x) => arg_bool(lname, 2, "use_xhtml", &x)?,
                None => true,
            };
            let tag: &[u8] = if xhtml { b"<br />" } else { b"<br>" };
            let mut out = Vec::with_capacity(s.len());
            let mut i = 0;
            while i < s.len() {
                let c = s[i];
                if c == b'\r' || c == b'\n' {
                    out.extend_from_slice(tag);
                    out.push(c);
                    // \r\n and \n\r count as one line break
                    if let Some(&next) = s.get(i + 1)
                        && (next == b'\r' || next == b'\n')
                        && next != c
                    {
                        out.push(next);
                        i += 1;
                    }
                } else {
                    out.push(c);
                }
                i += 1;
            }
            Ok(Value::str(out))
        }
        "htmlspecialchars" => {
            let [s, flags, _encoding, double] = args::<1, 4>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let flags = match flags {
                Some(f) => arg_int(lname, 2, "flags", &f)?,
                None => ENT_QUOTES | ENT_SUBSTITUTE | ENT_HTML401,
            };
            let double = match double {
                Some(d) => arg_bool(lname, 4, "double_encode", &d)?,
                None => true,
            };
            let doc = flags & (ENT_XML1 | ENT_XHTML | ENT_HTML5);
            let mut out = Vec::with_capacity(s.len());
            for (i, &c) in s.iter().enumerate() {
                match c {
                    b'&' if !double && is_entity(&s[i..]) => out.push(b'&'),
                    b'&' => out.extend_from_slice(b"&amp;"),
                    b'"' if flags & ENT_COMPAT != 0 => out.extend_from_slice(b"&quot;"),
                    b'\'' if flags & 1 != 0 => {
                        out.extend_from_slice(if doc == ENT_XML1 || doc == ENT_HTML5 { b"&apos;" } else { b"&#039;" })
                    }
                    b'<' => out.extend_from_slice(b"&lt;"),
                    b'>' => out.extend_from_slice(b"&gt;"),
                    c => out.push(c),
                }
            }
            Ok(Value::str(out))
        }
        "htmlspecialchars_decode" => {
            let [s, flags] = args::<1, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let flags = match flags {
                Some(f) => arg_int(lname, 2, "flags", &f)?,
                None => ENT_QUOTES | ENT_SUBSTITUTE | ENT_HTML401,
            };
            let mut table: Vec<(&[u8], u8)> = vec![(b"&amp;", b'&'), (b"&lt;", b'<'), (b"&gt;", b'>')];
            if flags & ENT_COMPAT != 0 {
                table.push((b"&quot;", b'"'));
            }
            if flags & 1 != 0 {
                table.extend([(&b"&#039;"[..], b'\''), (b"&#39;", b'\''), (b"&apos;", b'\''), (b"&#x27;", b'\'')]);
            }
            let mut out = Vec::with_capacity(s.len());
            let mut i = 0;
            'outer: while i < s.len() {
                if s[i] == b'&' {
                    for (ent, c) in &table {
                        if s[i..].starts_with(ent) {
                            out.push(*c);
                            i += ent.len();
                            continue 'outer;
                        }
                    }
                }
                out.push(s[i]);
                i += 1;
            }
            Ok(Value::str(out))
        }
        "addslashes" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let mut out = Vec::with_capacity(s.len());
            for c in s {
                match c {
                    b'\'' | b'"' | b'\\' => {
                        out.push(b'\\');
                        out.push(c);
                    }
                    0 => out.extend_from_slice(b"\\0"),
                    c => out.push(c),
                }
            }
            Ok(Value::str(out))
        }
        "stripslashes" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let mut out = Vec::with_capacity(s.len());
            let mut it = s.iter();
            while let Some(&c) = it.next() {
                if c == b'\\' {
                    match it.next() {
                        Some(b'0') => out.push(0),
                        Some(&n) => out.push(n),
                        None => {}
                    }
                } else {
                    out.push(c);
                }
            }
            Ok(Value::str(out))
        }
        "strstr" | "stristr" | "strchr" => {
            let [h, n, before] = args::<2, 3>(name, vals)?;
            let h = arg_str(rt, lname, 1, "haystack", &h.unwrap())?;
            let n = arg_str(rt, lname, 2, "needle", &n.unwrap())?;
            let before = match before {
                Some(b) => arg_bool(lname, 3, "before_needle", &b)?,
                None => false,
            };
            let pos = if lname == "stristr" { find(&lower(&h), &lower(&n)) } else { find(&h, &n) };
            Ok(match pos {
                Some(p) if before => Value::str(h[..p].to_vec()),
                Some(p) => Value::str(h[p..].to_vec()),
                None => Value::Bool(false),
            })
        }
        "strrchr" => {
            let [h, n, before] = args::<2, 3>(name, vals)?;
            let h = arg_str(rt, lname, 1, "haystack", &h.unwrap())?;
            let n = arg_str(rt, lname, 2, "needle", &n.unwrap())?;
            let before = match before {
                Some(b) => arg_bool(lname, 3, "before_needle", &b)?,
                None => false,
            };
            let c = n.first().copied().unwrap_or(0);
            Ok(match h.iter().rposition(|&b| b == c) {
                Some(p) if before => Value::str(h[..p].to_vec()),
                Some(p) => Value::str(h[p..].to_vec()),
                None => Value::Bool(false),
            })
        }
        "stripos" | "strrpos" | "strripos" => {
            let [h, n, off] = args::<2, 3>(name, vals)?;
            let mut h = arg_str(rt, lname, 1, "haystack", &h.unwrap())?;
            let mut n = arg_str(rt, lname, 2, "needle", &n.unwrap())?;
            let off = match off {
                Some(o) => arg_int(lname, 3, "offset", &o)?,
                None => 0,
            };
            if lname != "strrpos" {
                h = lower(&h);
                n = lower(&n);
            }
            let len = h.len() as i64;
            if off > len || off < -len {
                return Err(Fatal::value_error(format!(
                    "{lname}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"
                )));
            }
            let found = if lname == "stripos" {
                let start = if off < 0 { len + off } else { off } as usize;
                find(&h[start..], &n).map(|p| p + start)
            } else if off >= 0 {
                rfind(&h[off as usize..], &n).map(|p| p + off as usize)
            } else {
                // A negative offset: the match must start at or before len + off
                let last_start = len + off;
                let end = (last_start as usize + n.len()).min(h.len());
                rfind(&h[..end], &n)
            };
            Ok(found.map_or(Value::Bool(false), |p| Value::Int(p as i64)))
        }
        "substr_count" => {
            let [h, n, off, len] = args::<2, 4>(name, vals)?;
            let h = arg_str(rt, lname, 1, "haystack", &h.unwrap())?;
            let n = arg_str(rt, lname, 2, "needle", &n.unwrap())?;
            if n.is_empty() {
                return Err(Fatal::value_error("substr_count(): Argument #2 ($needle) cannot be empty"));
            }
            let size = h.len() as i64;
            let mut off = match off {
                Some(o) => arg_int(lname, 3, "offset", &o)?,
                None => 0,
            };
            if off < 0 {
                off += size;
            }
            if off < 0 || off > size {
                return Err(Fatal::value_error("substr_count(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"));
            }
            let end = match len {
                None | Some(Value::Null) => size,
                Some(l) => {
                    let mut l = arg_int(lname, 4, "length", &l)?;
                    if l < 0 {
                        l += size - off;
                    }
                    if l < 0 || l > size - off {
                        return Err(Fatal::value_error("substr_count(): Argument #4 ($length) must be contained in argument #1 ($haystack)"));
                    }
                    off + l
                }
            };
            let hay = &h[off as usize..end as usize];
            let mut count = 0;
            let mut i = 0;
            while i + n.len() <= hay.len() {
                if &hay[i..i + n.len()] == n.as_slice() {
                    count += 1;
                    i += n.len();
                } else {
                    i += 1;
                }
            }
            Ok(Value::Int(count))
        }
        "strpbrk" => {
            let [s, chars] = args::<2, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let chars = arg_str(rt, lname, 2, "characters", &chars.unwrap())?;
            if chars.is_empty() {
                return Err(Fatal::value_error("strpbrk(): Argument #2 ($characters) must be a non-empty string"));
            }
            Ok(match s.iter().position(|c| chars.contains(c)) {
                Some(p) => Value::str(s[p..].to_vec()),
                None => Value::Bool(false),
            })
        }
        "strspn" | "strcspn" => {
            let [s, chars, off, len] = args::<2, 4>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let chars = arg_str(rt, lname, 2, "characters", &chars.unwrap())?;
            let n = s.len() as i64;
            let start = match off {
                Some(o) => {
                    let o = arg_int(lname, 3, "offset", &o)?;
                    if o < 0 { (n + o).max(0) } else { o.min(n) }
                }
                None => 0,
            };
            let end = match len {
                None | Some(Value::Null) => n,
                Some(l) => {
                    let l = arg_int(lname, 4, "length", &l)?;
                    if l < 0 { (n + l).max(start) } else { (start + l).min(n) }
                }
            };
            let part = &s[start as usize..end.max(start) as usize];
            let want = lname == "strspn";
            Ok(Value::Int(part.iter().take_while(|c| chars.contains(c) == want).count() as i64))
        }
        "strcmp" | "strcasecmp" => {
            let [a, b] = args::<2, 2>(name, vals)?;
            let mut a = arg_str(rt, lname, 1, "string1", &a.unwrap())?;
            let mut b = arg_str(rt, lname, 2, "string2", &b.unwrap())?;
            if lname == "strcasecmp" {
                a = lower(&a);
                b = lower(&b);
            }
            Ok(sign(a.cmp(&b)))
        }
        "strncmp" | "strncasecmp" => {
            let [a, b, n] = args::<3, 3>(name, vals)?;
            let mut a = arg_str(rt, lname, 1, "string1", &a.unwrap())?;
            let mut b = arg_str(rt, lname, 2, "string2", &b.unwrap())?;
            let n = arg_int(lname, 3, "length", &n.unwrap())?;
            if n < 0 {
                return Err(Fatal::value_error(format!("{lname}(): Argument #3 ($length) must be greater than or equal to 0")));
            }
            if lname == "strncasecmp" {
                a = lower(&a);
                b = lower(&b);
            }
            let n = n as usize;
            Ok(sign(a[..n.min(a.len())].cmp(&b[..n.min(b.len())])))
        }
        "strnatcmp" | "strnatcasecmp" => {
            let [a, b] = args::<2, 2>(name, vals)?;
            let a = arg_str(rt, lname, 1, "string1", &a.unwrap())?;
            let b = arg_str(rt, lname, 2, "string2", &b.unwrap())?;
            Ok(Value::Int(strnatcmp(&a, &b, lname == "strnatcasecmp") as i64))
        }
        "str_split" => {
            let [s, len] = args::<1, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let len = match len {
                Some(l) => arg_int(lname, 2, "length", &l)?,
                None => 1,
            };
            if len < 1 {
                return Err(Fatal::value_error("str_split(): Argument #2 ($length) must be greater than 0"));
            }
            Ok(list(s.chunks(len.min(isize::MAX as i64) as usize).map(|c| Value::str(c.to_vec()))))
        }
        "chunk_split" => {
            let [s, len, end] = args::<1, 3>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let len = match len {
                Some(l) => arg_int(lname, 2, "length", &l)?,
                None => 76,
            };
            let end = match end {
                Some(e) => arg_str(rt, lname, 3, "separator", &e)?,
                None => b"\r\n".to_vec(),
            };
            if len < 1 {
                return Err(Fatal::value_error("chunk_split(): Argument #2 ($length) must be greater than 0"));
            }
            let mut out = Vec::new();
            for c in s.chunks(len.min(isize::MAX as i64) as usize) {
                out.extend_from_slice(c);
                out.extend_from_slice(&end);
            }
            Ok(Value::str(out))
        }
        "str_word_count" => {
            let [s, format, chars] = args::<1, 3>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let format = match format {
                Some(f) => arg_int(lname, 2, "format", &f)?,
                None => 0,
            };
            let extra = match chars {
                Some(Value::Null) | None => [false; 256],
                Some(c) => char_mask(lname, &arg_str(rt, lname, 3, "characters", &c)?),
            };
            let words = str_words(&s, &extra);
            Ok(match format {
                0 => Value::Int(words.len() as i64),
                1 => list(words.into_iter().map(|(_, w)| Value::str(w.to_vec()))),
                2 => {
                    let mut arr = PhpArray::new();
                    for (pos, w) in words {
                        arr.insert(crate::array::Key::Int(pos as i64), Value::str(w.to_vec()));
                    }
                    Value::Array(Rc::new(arr))
                }
                _ => {
                    return Err(Fatal::value_error("str_word_count(): Argument #2 ($format) must be a valid format value"))
                }
            })
        }
        "similar_text" => {
            let [a, b, _percent] = args::<2, 3>(name, vals)?;
            let a = arg_str(rt, lname, 1, "string1", &a.unwrap())?;
            let b = arg_str(rt, lname, 2, "string2", &b.unwrap())?;
            Ok(Value::Int(similar_text(&a, &b) as i64))
        }
        "levenshtein" => {
            let [a, b, ins, rep, del] = args::<2, 5>(name, vals)?;
            let a = arg_str(rt, lname, 1, "string1", &a.unwrap())?;
            let b = arg_str(rt, lname, 2, "string2", &b.unwrap())?;
            let cost = |v: Option<Value>, n, p| v.map_or(Ok(1), |v| arg_int(lname, n, p, &v));
            let (ins, rep, del) = (cost(ins, 3, "insertion_cost")?, cost(rep, 4, "replacement_cost")?, cost(del, 5, "deletion_cost")?);
            Ok(Value::Int(levenshtein(&a, &b, ins, rep, del)))
        }
        "chr" => {
            let [c] = args::<1, 1>(name, vals)?;
            let c = arg_int(lname, 1, "codepoint", &c.unwrap())?;
            Ok(Value::str(vec![c.rem_euclid(256) as u8]))
        }
        "ord" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "character", &s.unwrap())?;
            Ok(Value::Int(s.first().copied().unwrap_or(0) as i64))
        }
        "bin2hex" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            Ok(Value::str(s.iter().map(|b| format!("{b:02x}")).collect::<String>()))
        }
        "hex2bin" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            if s.len() % 2 != 0 {
                diag::warning("hex2bin(): Hexadecimal input string must have an even length");
                return Ok(Value::Bool(false));
            }
            let digit = |c: u8| (c as char).to_digit(16);
            let mut out = Vec::with_capacity(s.len() / 2);
            for pair in s.chunks(2) {
                match (digit(pair[0]), digit(pair[1])) {
                    (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
                    _ => {
                        diag::warning("hex2bin(): Input string must be hexadecimal string");
                        return Ok(Value::Bool(false));
                    }
                }
            }
            Ok(Value::str(out))
        }
        "crc32" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            Ok(Value::Int(crc32(&s) as i64))
        }
        "ctype_digit" | "ctype_alpha" | "ctype_alnum" | "ctype_upper" | "ctype_lower" | "ctype_space"
        | "ctype_xdigit" | "ctype_punct" => {
            let [v] = args::<1, 1>(name, vals)?;
            let test: fn(&u8) -> bool = match lname {
                "ctype_digit" => u8::is_ascii_digit,
                "ctype_alpha" => u8::is_ascii_alphabetic,
                "ctype_alnum" => u8::is_ascii_alphanumeric,
                "ctype_upper" => u8::is_ascii_uppercase,
                "ctype_lower" => u8::is_ascii_lowercase,
                "ctype_space" => |c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c),
                "ctype_xdigit" => u8::is_ascii_hexdigit,
                _ => u8::is_ascii_punctuation,
            };
            Ok(Value::Bool(match v.unwrap() {
                Value::Str(s) => !s.is_empty() && s.iter().all(test),
                Value::Int(i) => {
                    diag::deprecated(format!("{lname}(): Argument of type int will be interpreted as string in the future"));
                    // Legacy behavior: -128..255 is a character code, anything else a number string
                    match i {
                        -128..=-1 => test(&((i + 256) as u8)),
                        0..=255 => test(&(i as u8)),
                        _ => i.to_string().as_bytes().iter().all(test),
                    }
                }
                other => {
                    diag::deprecated(format!(
                        "{lname}(): Argument of type {} will be interpreted as string in the future",
                        other.type_name()
                    ));
                    false
                }
            }))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}

fn is_entity(s: &[u8]) -> bool {
    // `&name;`, `&#123;` or `&#x1F;`
    let body = &s[1..];
    let end = body.iter().position(|&c| c == b';');
    let Some(end) = end else { return false };
    let name = &body[..end];
    if let Some(num) = name.strip_prefix(b"#") {
        return match num.strip_prefix(b"x").or_else(|| num.strip_prefix(b"X")) {
            Some(hex) => !hex.is_empty() && hex.iter().all(u8::is_ascii_hexdigit),
            None => !num.is_empty() && num.iter().all(u8::is_ascii_digit),
        };
    }
    !name.is_empty() && name.iter().all(u8::is_ascii_alphanumeric)
}

fn wordwrap(s: &[u8], width: i64, brk: &[u8], cut: bool) -> Vec<u8> {
    let n = s.len() as i64;
    if brk.len() == 1 && !cut {
        let mut out = s.to_vec();
        let (mut laststart, mut lastspace) = (0i64, 0i64);
        for current in 0..n {
            let c = s[current as usize];
            if c == brk[0] {
                laststart = current + 1;
                lastspace = current + 1;
            } else if c == b' ' {
                if current - laststart >= width {
                    out[current as usize] = brk[0];
                    laststart = current + 1;
                }
                lastspace = current;
            } else if current - laststart >= width && laststart != lastspace {
                out[lastspace as usize] = brk[0];
                laststart = lastspace + 1;
            }
        }
        return out;
    }
    let mut out = Vec::with_capacity(s.len());
    let (mut laststart, mut lastspace) = (0i64, 0i64);
    let mut current = 0i64;
    while current < n {
        let c = s[current as usize];
        let cu = current as usize;
        if c == brk[0] && cu + brk.len() <= s.len() && &s[cu..cu + brk.len()] == brk {
            out.extend_from_slice(&s[laststart as usize..cu + brk.len()]);
            current += brk.len() as i64 - 1;
            laststart = current + 1;
            lastspace = current + 1;
        } else if c == b' ' {
            if current - laststart >= width {
                out.extend_from_slice(&s[laststart as usize..cu]);
                out.extend_from_slice(brk);
                laststart = current + 1;
            }
            lastspace = current;
        } else if current - laststart >= width && cut && laststart >= lastspace {
            out.extend_from_slice(&s[laststart as usize..cu]);
            out.extend_from_slice(brk);
            laststart = current;
            lastspace = current;
        } else if current - laststart >= width && laststart < lastspace {
            out.extend_from_slice(&s[laststart as usize..lastspace as usize]);
            out.extend_from_slice(brk);
            lastspace += 1;
            laststart = lastspace;
        }
        current += 1;
    }
    if laststart != current {
        out.extend_from_slice(&s[laststart as usize..]);
    }
    out
}

/// str_replace() and str_ireplace(), with the number of replacements made
pub fn str_replace(
    rt: &mut dyn Runtime,
    lname: &str,
    search: Value,
    replace: Value,
    subject: Value,
) -> EResult<(Value, i64)> {
    let ci = lname == "str_ireplace";
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = match (&search, &replace) {
        (Value::Array(s), Value::Array(r)) => {
            let mut reps = r.values();
            let mut out = Vec::new();
            for v in s.values() {
                let rep = match reps.next() {
                    Some(r) => rt.stringify(&r.copy_out())?,
                    None => Vec::new(),
                };
                out.push((rt.stringify(&v.copy_out())?, rep));
            }
            out
        }
        (Value::Array(s), r) => {
            let rep = arg_str(rt, lname, 2, "replace", r)?;
            let mut out = Vec::new();
            for v in s.values() {
                out.push((rt.stringify(&v.copy_out())?, rep.clone()));
            }
            out
        }
        (s, Value::Array(_)) => {
            arg_str(rt, lname, 1, "search", s)?;
            return Err(Fatal::type_error(format!(
                "{lname}(): Argument #2 ($replace) must be of type string when argument #1 ($search) is a string"
            )));
        }
        (s, r) => vec![(arg_str(rt, lname, 1, "search", s)?, arg_str(rt, lname, 2, "replace", r)?)],
    };
    let mut count = 0;
    let mut one = |rt: &mut dyn Runtime, v: &Value| -> EResult<Value> {
        let mut s = match v {
            Value::Array(_) | Value::Object(_) => return Ok(v.clone()),
            v => rt.stringify(v)?,
        };
        for (from, to) in &pairs {
            if from.is_empty() {
                continue;
            }
            s = replace_all(&s, from, to, ci, &mut count);
        }
        Ok(Value::str(s))
    };
    let result = match &subject {
        Value::Array(arr) => {
            let mut out = PhpArray::new();
            for (k, v) in arr.iter() {
                out.insert(k.clone(), one(rt, &v.copy_out())?);
            }
            Value::Array(Rc::new(out))
        }
        other => {
            let s = arg_str(rt, lname, 3, "subject", other)?;
            one(rt, &Value::str(s))?
        }
    };
    Ok((result, count))
}

fn replace_all(s: &[u8], from: &[u8], to: &[u8], ci: bool, count: &mut i64) -> Vec<u8> {
    let hay = if ci { lower(s) } else { s.to_vec() };
    let needle = if ci { lower(from) } else { from.to_vec() };
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if hay[i..].starts_with(&needle) {
            out.extend_from_slice(to);
            i += needle.len();
            *count += 1;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// strtr() with an array: longest keys first, and replaced text is not looked at again
fn strtr_pairs(s: &[u8], mut map: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<u8> {
    if map.is_empty() {
        return s.to_vec();
    }
    map.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    'outer: while i < s.len() {
        for (from, to) in &map {
            if s[i..].starts_with(from) {
                out.extend_from_slice(to);
                i += from.len();
                continue 'outer;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// Natural order comparison, ported from php-src's strnatcmp_ex
pub fn strnatcmp(a: &[u8], b: &[u8], ci: bool) -> i32 {
    use std::cmp::Ordering;
    if a.is_empty() || b.is_empty() {
        return match a.len().cmp(&b.len()) {
            Ordering::Equal => 0,
            Ordering::Greater => 1,
            Ordering::Less => -1,
        };
    }
    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
    let digit = |s: &[u8], i: usize| i < s.len() && s[i].is_ascii_digit();
    let (mut ai, mut bi) = (0, 0);
    while at(a, ai) == b'0' && ai + 1 < a.len() && a[ai + 1].is_ascii_digit() {
        ai += 1;
    }
    while at(b, bi) == b'0' && bi + 1 < b.len() && b[bi + 1].is_ascii_digit() {
        bi += 1;
    }
    loop {
        while at(a, ai).is_ascii_whitespace() && ai < a.len() {
            ai += 1;
        }
        while at(b, bi).is_ascii_whitespace() && bi < b.len() {
            bi += 1;
        }
        let (ca, cb) = (at(a, ai), at(b, bi));
        if digit(a, ai) && digit(b, bi) {
            let fractional = ca == b'0' || cb == b'0';
            let result = if fractional {
                // Left-aligned: the first difference wins
                loop {
                    match (digit(a, ai), digit(b, bi)) {
                        (false, false) => break 0,
                        (false, true) => break -1,
                        (true, false) => break 1,
                        _ => {}
                    }
                    match a[ai].cmp(&b[bi]) {
                        Ordering::Less => break -1,
                        Ordering::Greater => break 1,
                        Ordering::Equal => {}
                    }
                    ai += 1;
                    bi += 1;
                }
            } else {
                // Right-aligned: the longest run wins, then the first difference
                let mut bias = 0;
                loop {
                    match (digit(a, ai), digit(b, bi)) {
                        (false, false) => break bias,
                        (false, true) => break -1,
                        (true, false) => break 1,
                        _ => {}
                    }
                    if bias == 0 {
                        bias = match a[ai].cmp(&b[bi]) {
                            Ordering::Less => -1,
                            Ordering::Greater => 1,
                            Ordering::Equal => 0,
                        };
                    }
                    ai += 1;
                    bi += 1;
                }
            };
            if result != 0 {
                return result;
            }
            match (ai >= a.len(), bi >= b.len()) {
                (true, true) => return 0,
                (true, false) => return -1,
                (false, true) => return 1,
                _ => continue,
            }
        }
        let (ca, cb) = if ci { (ca.to_ascii_uppercase(), cb.to_ascii_uppercase()) } else { (ca, cb) };
        match ca.cmp(&cb) {
            Ordering::Less => return -1,
            Ordering::Greater => return 1,
            Ordering::Equal => {}
        }
        ai += 1;
        bi += 1;
        match (ai >= a.len(), bi >= b.len()) {
            (true, true) => return 0,
            (true, false) => return -1,
            (false, true) => return 1,
            _ => {}
        }
    }
}

/// Words as str_word_count() sees them, with their byte offsets
fn str_words<'a>(s: &'a [u8], extra: &[bool; 256]) -> Vec<(usize, &'a [u8])> {
    let word = |c: u8| c.is_ascii_alphabetic() || extra[c as usize] || c == b'\'' || c == b'-';
    let mut out = Vec::new();
    let mut p = 0;
    let mut e = s.len();
    // The first character cannot be ' or -, the last cannot be -, unless allowed explicitly
    while p < e && (s[p] == b'\'' && !extra[b'\'' as usize] || s[p] == b'-' && !extra[b'-' as usize]) {
        p += 1;
    }
    if e > p && s[e - 1] == b'-' && !extra[b'-' as usize] {
        e -= 1;
    }
    while p < e {
        let start = p;
        while p < e && word(s[p]) {
            p += 1;
        }
        if p > start {
            out.push((start, &s[start..p]));
        } else {
            p += 1;
        }
    }
    out
}

pub fn similar_text(a: &[u8], b: &[u8]) -> usize {
    // Iterative version of php_similar_char: the recursion depth would follow the input
    let mut sum = 0;
    let mut work = vec![(a, b)];
    while let Some((a, b)) = work.pop() {
        let (mut max, mut count, mut pos1, mut pos2) = (0, 0, 0, 0);
        for p in 0..a.len() {
            for q in 0..b.len() {
                let l = a[p..].iter().zip(&b[q..]).take_while(|(x, y)| x == y).count();
                if l > max {
                    max = l;
                    count += 1;
                    pos1 = p;
                    pos2 = q;
                }
            }
        }
        if max == 0 {
            continue;
        }
        sum += max;
        if pos1 > 0 && pos2 > 0 && count > 1 {
            work.push((&a[..pos1], &b[..pos2]));
        }
        if pos1 + max < a.len() && pos2 + max < b.len() {
            work.push((&a[pos1 + max..], &b[pos2 + max..]));
        }
    }
    sum
}

fn levenshtein(a: &[u8], b: &[u8], ins: i64, rep: i64, del: i64) -> i64 {
    if a.is_empty() {
        return b.len() as i64 * ins;
    }
    if b.is_empty() {
        return a.len() as i64 * del;
    }
    let mut p1: Vec<i64> = (0..=b.len() as i64).map(|i| i * ins).collect();
    let mut p2 = vec![0; b.len() + 1];
    for &ca in a {
        p2[0] = p1[0] + del;
        for (j, &cb) in b.iter().enumerate() {
            let c0 = p1[j] + if ca == cb { 0 } else { rep };
            let c1 = p1[j + 1] + del;
            let c2 = p2[j] + ins;
            p2[j + 1] = c0.min(c1).min(c2);
        }
        std::mem::swap(&mut p1, &mut p2);
    }
    p1[b.len()]
}

pub fn crc32(s: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in s {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
