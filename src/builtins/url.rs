//! URLs and base64: urlencode() and friends, parse_url(), http_build_query(),
//! base64_encode() and base64_decode(), as php-src's url.c, http.c and base64.c

use std::rc::Rc;

use super::{arg_bool, arg_int, arg_str, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::interp::{EResult, Fatal};
use crate::object::{unmangle, Visibility};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "urlencode", "rawurlencode", "urldecode", "rawurldecode", "base64_encode", "base64_decode", "parse_url",
    "http_build_query", "parse_str",
];

const PHP_QUERY_RFC1738: i64 = 1;
const PHP_QUERY_RFC3986: i64 = 2;

pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "PHP_URL_SCHEME" => 0,
        "PHP_URL_HOST" => 1,
        "PHP_URL_PORT" => 2,
        "PHP_URL_USER" => 3,
        "PHP_URL_PASS" => 4,
        "PHP_URL_PATH" => 5,
        "PHP_URL_QUERY" => 6,
        "PHP_URL_FRAGMENT" => 7,
        "PHP_QUERY_RFC1738" => PHP_QUERY_RFC1738,
        "PHP_QUERY_RFC3986" => PHP_QUERY_RFC3986,
        _ => return None,
    }))
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encodes all but alphanumerics and `-_.`; `raw` keeps `~` and
/// encodes spaces as %20 instead of `+`
fn encode_into(out: &mut Vec<u8>, s: &[u8], raw: bool) {
    for &c in s {
        if c == b' ' && !raw {
            out.push(b'+');
        } else if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_') || (raw && c == b'~') {
            out.push(c);
        } else {
            out.extend_from_slice(&[b'%', HEX[(c >> 4) as usize], HEX[(c & 15) as usize]]);
        }
    }
}

pub fn encode(s: &[u8], raw: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    encode_into(&mut out, s, raw);
    out
}

/// Decodes %XX escapes (malformed ones stay as they are) and, unless `raw`, `+`
fn decode(s: &[u8], raw: bool) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == b'+' && !raw {
            out.push(b' ');
        } else if c == b'%'
            && let (Some(h), Some(l)) = (s.get(i + 1).and_then(|&c| hex(c)), s.get(i + 2).and_then(|&c| hex(c)))
        {
            out.push(h << 4 | l);
            i += 2;
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len().div_ceil(3) * 4);
    for chunk in s.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() { B64[(n >> (18 - 6 * i) & 63) as usize] } else { b'=' });
        }
    }
    out
}

/// php_base64_decode_impl: non-strict mode skips characters outside the
/// alphabet; strict mode only skips whitespace and checks the padding
pub fn base64_decode(s: &[u8], strict: bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut n, mut padding, mut acc) = (0usize, 0usize, 0u32);
    for &c in s {
        if c == b'=' {
            padding += 1;
            continue;
        }
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'\t' | b'\n' | b'\r' | b' ' => continue,
            _ if strict => return None,
            _ => continue,
        };
        if strict && padding > 0 {
            return None;
        }
        acc = acc << 6 | v as u32;
        n += 1;
        if n % 4 == 0 {
            out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]);
            acc = 0;
        }
    }
    if strict && (n % 4 == 1 || (padding > 0 && (padding > 2 || (n + padding) % 4 != 0))) {
        return None;
    }
    // The bytes completed by a final partial group
    match n % 4 {
        2 => out.push((acc >> 4) as u8),
        3 => out.extend_from_slice(&[(acc >> 10) as u8, (acc >> 2) as u8]),
        _ => {}
    }
    Some(out)
}

#[derive(Default)]
struct Url {
    scheme: Option<Vec<u8>>,
    host: Option<Vec<u8>>,
    port: Option<u16>,
    user: Option<Vec<u8>>,
    pass: Option<Vec<u8>>,
    path: Option<Vec<u8>>,
    query: Option<Vec<u8>>,
    fragment: Option<Vec<u8>>,
}

/// A component, with control characters replaced by `_`
fn part(s: &[u8]) -> Option<Vec<u8>> {
    Some(s.iter().map(|&c| if c < 0x20 || c == 0x7f { b'_' } else { c }).collect())
}

/// The first position at or after `from` holding one of `set`, or the end
fn cspn(s: &[u8], from: usize, set: &[u8]) -> usize {
    s[from..].iter().position(|c| set.contains(c)).map_or(s.len(), |i| from + i)
}

/// A port of up to five digits, as strtol reads it
fn port_number(digits: &[u8]) -> Option<u16> {
    let n = digits.iter().take_while(|c| c.is_ascii_digit()).fold(0u32, |n, &c| n * 10 + (c - b'0') as u32);
    if digits.first().is_some_and(u8::is_ascii_digit) && n <= 65535 { Some(n as u16) } else { None }
}

enum Next {
    Host,
    Path,
}

/// php_url_parse_ex2, step for step
fn parse_url(str: &[u8]) -> Option<Url> {
    let mut ret = Url::default();
    let ue = str.len();
    let mut s = 0usize;
    let starts_slashes = |s: usize| s + 1 < ue && str[s] == b'/' && str[s + 1] == b'/';

    let colon = str.iter().position(|&c| c == b':');
    let mut port_scan = None;
    let next = 'scheme: {
        if let Some(e) = colon
            && e != 0
        {
            let valid = str[..e].iter().all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'));
            if !valid {
                if e + 1 < ue && e < cspn(str, 0, b"?#") {
                    port_scan = Some(e);
                    break 'scheme Next::Host;
                } else if starts_slashes(s) {
                    s += 2;
                    break 'scheme Next::Host;
                }
                break 'scheme Next::Path;
            }
            if e + 1 == ue {
                ret.scheme = part(&str[..e]);
                return Some(ret);
            }
            if str[e + 1] != b'/' {
                // `a.com:80` is a host and port, `mailto:x` a scheme and path
                let digits = str[e + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
                let p = e + 1 + digits;
                if (p == ue || str[p] == b'/') && p - e < 7 {
                    port_scan = Some(e);
                    break 'scheme Next::Host;
                }
                ret.scheme = part(&str[..e]);
                s = e + 1;
                break 'scheme Next::Path;
            }
            ret.scheme = part(&str[..e]);
            if e + 2 < ue && str[e + 2] == b'/' {
                s = e + 3;
                if str[..e].eq_ignore_ascii_case(b"file") && e + 3 < ue && str[e + 3] == b'/' {
                    // file:///c:/dir keeps the drive letter in the path
                    if e + 5 < ue && str[e + 5] == b':' {
                        s = e + 4;
                    }
                    break 'scheme Next::Path;
                }
                break 'scheme Next::Host;
            }
            s = e + 1;
            break 'scheme Next::Path;
        } else if let Some(e) = colon {
            port_scan = Some(e);
            break 'scheme Next::Host;
        } else if starts_slashes(s) {
            s += 2;
            break 'scheme Next::Host;
        }
        Next::Path
    };

    let mut next = next;
    if let Some(e) = port_scan {
        // A colon not ending a scheme: maybe `host:port`
        let p = e + 1;
        let digits = str[p..].iter().take(6).take_while(|c| c.is_ascii_digit()).count();
        let pp = p + digits;
        if digits > 0 && digits < 6 && (pp == ue || str[pp] == b'/') {
            ret.port = Some(port_number(&str[p..pp])?);
            if starts_slashes(s) {
                s += 2;
            }
        } else if digits == 0 && pp == ue {
            return None;
        } else if starts_slashes(s) {
            s += 2;
        } else {
            next = Next::Path;
        }
    }

    if let Next::Host = next {
        let e = cspn(str, s, b"/?#");
        // user and password
        if let Some(at) = str[s..e].iter().rposition(|&c| c == b'@').map(|i| s + i) {
            match str[s..at].iter().position(|&c| c == b':').map(|i| s + i) {
                Some(c) => {
                    ret.user = part(&str[s..c]);
                    ret.pass = part(&str[c + 1..at]);
                }
                None => ret.user = part(&str[s..at]),
            }
            s = at + 1;
        }
        // the port, unless the host is an IPv6 literal
        let ipv6 = s < ue && str[s] == b'[' && e > 0 && str[e - 1] == b']';
        let colon = if ipv6 { None } else { str[s.min(e)..e].iter().rposition(|&c| c == b':').map(|i| s + i) };
        let host_end = match colon {
            Some(c) => {
                if ret.port.is_none() {
                    let digits = &str[c + 1..e];
                    if digits.len() > 5 {
                        return None;
                    } else if !digits.is_empty() {
                        ret.port = Some(port_number(digits)?);
                    }
                }
                c
            }
            None => e,
        };
        if host_end <= s {
            return None;
        }
        ret.host = part(&str[s..host_end]);
        if e == ue {
            return Some(ret);
        }
        s = e;
    }

    // just_path
    let mut e = ue;
    if let Some(h) = str[s..e].iter().position(|&c| c == b'#').map(|i| s + i) {
        ret.fragment = part(&str[h + 1..e]);
        e = h;
    }
    if let Some(q) = str[s..e].iter().position(|&c| c == b'?').map(|i| s + i) {
        ret.query = part(&str[q + 1..e]);
        e = q;
    }
    if s < e || s == ue {
        ret.path = part(&str[s..e]);
    }
    Some(ret)
}

/// Stores a variable of a query string as php_register_variable_ex does:
/// `a[b][]=v` makes nested arrays, and `.` or ` ` in the base name become `_`
fn register(out: &mut PhpArray, name: &[u8], value: Vec<u8>) {
    let name = &name[name.iter().take_while(|&&c| c == b' ').count()..];
    let bracket = name.iter().position(|&c| c == b'[');
    let fix = |s: &[u8]| -> Vec<u8> { s.iter().map(|&c| if c == b' ' || c == b'.' { b'_' } else { c }).collect() };
    let mut base = fix(&name[..bracket.unwrap_or(name.len())]);
    let mut indexes: Vec<Option<Vec<u8>>> = Vec::new();
    if let Some(b) = bracket {
        let mut pos = b;
        loop {
            let start = pos + 1;
            match name[start..].iter().position(|&c| c == b']') {
                None => {
                    if indexes.is_empty() {
                        // Not an index after all: the `[` is part of the name
                        base.push(b'_');
                        base.extend(name[start..].iter().map(|&c| if matches!(c, b' ' | b'.' | b'[') { b'_' } else { c }));
                    }
                    break;
                }
                Some(n) => {
                    let idx = &name[start..start + n];
                    indexes.push((!idx.is_empty()).then(|| idx.to_vec()));
                    pos = start + n + 1;
                    if name.get(pos) != Some(&b'[') {
                        break;
                    }
                }
            }
        }
    }
    if base.is_empty() {
        return;
    }
    let mut slot = out.entry(Key::from_bytes(&base));
    if indexes.is_empty() {
        *slot = Value::str(value);
        return;
    }
    // Each level is an array, made fresh when something else is there
    let depth = indexes.len();
    for (i, idx) in indexes.into_iter().enumerate() {
        if !matches!(slot, Value::Array(_)) {
            *slot = Value::Array(Rc::new(PhpArray::new()));
        }
        let Value::Array(a) = slot else { unreachable!() };
        let a = Rc::make_mut(a);
        let next = match idx {
            Some(k) => a.entry(Key::from_bytes(&k)),
            None => match a.push(Value::Null) {
                Ok(v) => v,
                Err(_) => return,
            },
        };
        if i + 1 == depth {
            *next = Value::str(value);
            return;
        }
        slot = next;
    }
}

/// parse_str(): a query string as nested arrays
pub fn parse_query(s: &[u8]) -> PhpArray {
    let mut out = PhpArray::new();
    for pair in s.split(|&c| c == b'&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.iter().position(|&c| c == b'=') {
            Some(i) => (&pair[..i], &pair[i + 1..]),
            None => (pair, &b""[..]),
        };
        register(&mut out, &decode(k, false), decode(v, false));
    }
    out
}

struct QueryBuilder<'a> {
    rt: &'a mut dyn Runtime,
    out: Vec<u8>,
    sep: Vec<u8>,
    raw: bool,
    /// The containers being encoded, to skip recursion as php-src does
    visiting: Vec<usize>,
}

/// php-src checks the native stack here; this bounds the nesting instead
const MAX_NESTING: usize = 10_000;

impl QueryBuilder<'_> {
    /// php_url_encode_hash_ex: `num_prefix` only applies at the top level
    fn hash(&mut self, entries: Vec<(Key, Value)>, num_prefix: Option<&[u8]>, key_prefix: Option<&[u8]>) -> EResult<()> {
        if self.visiting.len() >= MAX_NESTING {
            return Err(Fatal::error("Maximum call stack size reached."));
        }
        for (key, v) in entries {
            let v = v.copy_out();
            let nested = match &v {
                Value::Array(a) => Some((Rc::as_ptr(a) as usize, a.iter().map(|(k, v)| (k.clone(), v.clone())).collect())),
                Value::Object(o) if !o.class.is_enum => Some((Rc::as_ptr(o) as usize, public_props(o))),
                _ => None,
            };
            if let Some((addr, entries)) = nested {
                let mut prefix = Vec::new();
                match &key {
                    Key::Str(k) => {
                        if let Some(kp) = key_prefix {
                            prefix.extend_from_slice(kp);
                        }
                        encode_into(&mut prefix, k, self.raw);
                        prefix.extend_from_slice(if key_prefix.is_some() { b"%5D%5B" } else { b"%5B" });
                    }
                    Key::Int(i) => {
                        if let Some(kp) = key_prefix {
                            prefix.extend_from_slice(kp);
                        }
                        if let Some(np) = num_prefix {
                            prefix.extend_from_slice(np);
                        }
                        prefix.extend_from_slice(i.to_string().as_bytes());
                        prefix.extend_from_slice(if key_prefix.is_some() { b"%5D%5B" } else { b"%5B" });
                    }
                }
                if self.visiting.contains(&addr) {
                    continue;
                }
                self.visiting.push(addr);
                let r = self.hash(entries, None, Some(&prefix));
                self.visiting.pop();
                r?;
                continue;
            }
            if matches!(v, Value::Null | Value::Resource(_)) {
                continue;
            }
            self.scalar(&key, &v, num_prefix, key_prefix)?;
        }
        Ok(())
    }

    fn scalar(&mut self, key: &Key, v: &Value, num_prefix: Option<&[u8]>, key_prefix: Option<&[u8]>) -> EResult<()> {
        let value = match v {
            Value::Str(s) => encode(s, self.raw),
            Value::Int(i) => i.to_string().into_bytes(),
            Value::Float(_) => encode(&v.to_bytes(), self.raw),
            Value::Bool(b) => vec![if *b { b'1' } else { b'0' }],
            Value::Object(o) => {
                if o.class.backing.is_none() {
                    return Err(Fatal::value_error(format!(
                        "Unbacked enum {} cannot be converted to a string",
                        o.class.name
                    )));
                }
                return self.scalar(key, &o.get("value"), num_prefix, key_prefix);
            }
            _ => self.rt.stringify(v)?,
        };
        if !self.out.is_empty() {
            self.out.extend_from_slice(&self.sep);
        }
        if let Some(kp) = key_prefix {
            self.out.extend_from_slice(kp);
        }
        match key {
            Key::Str(k) => encode_into(&mut self.out, k, self.raw),
            Key::Int(i) => {
                if let Some(np) = num_prefix {
                    self.out.extend_from_slice(np);
                }
                self.out.extend_from_slice(i.to_string().as_bytes());
            }
        }
        if key_prefix.is_some() {
            self.out.extend_from_slice(b"%5D");
        }
        self.out.push(b'=');
        self.out.extend_from_slice(&value);
        Ok(())
    }
}

/// An object's public properties, unmangled
fn public_props(o: &crate::object::Object) -> Vec<(Key, Value)> {
    o.export_props()
        .into_iter()
        .filter_map(|(k, v)| {
            let (name, vis, _) = unmangle(&k);
            (vis == Visibility::Public).then(|| (Key::Str(name.into_bytes().into()), v))
        })
        .collect()
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "urlencode" | "rawurlencode" | "urldecode" | "rawurldecode" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let raw = lname.starts_with("raw");
            Ok(Value::str(if lname.ends_with("encode") { encode(&s, raw) } else { decode(&s, raw) }))
        }
        "base64_encode" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            Ok(Value::str(base64_encode(&s)))
        }
        "base64_decode" => {
            let [s, strict] = args::<1, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let strict = match strict {
                Some(b) => arg_bool(lname, 2, "strict", &b)?,
                None => false,
            };
            Ok(base64_decode(&s, strict).map_or(Value::Bool(false), Value::str))
        }
        "parse_url" => {
            let [s, component] = args::<1, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "url", &s.unwrap())?;
            let component = match component {
                Some(c) => arg_int(lname, 2, "component", &c)?,
                None => -1,
            };
            let Some(url) = parse_url(&s) else { return Ok(Value::Bool(false)) };
            let str_or_null = |p: Option<Vec<u8>>| p.map_or(Value::Null, Value::str);
            if component > -1 {
                return Ok(match component {
                    0 => str_or_null(url.scheme),
                    1 => str_or_null(url.host),
                    2 => url.port.map_or(Value::Null, |p| Value::Int(p as i64)),
                    3 => str_or_null(url.user),
                    4 => str_or_null(url.pass),
                    5 => str_or_null(url.path),
                    6 => str_or_null(url.query),
                    7 => str_or_null(url.fragment),
                    _ => {
                        return Err(Fatal::value_error(format!(
                            "parse_url(): Argument #2 ($component) must be a valid URL component identifier, {component} given"
                        )));
                    }
                });
            }
            let mut out = PhpArray::new();
            let mut add = |k: &str, v: Option<Value>| {
                if let Some(v) = v {
                    out.insert(Key::Str(k.as_bytes().into()), v);
                }
            };
            add("scheme", url.scheme.map(Value::str));
            add("host", url.host.map(Value::str));
            add("port", url.port.map(|p| Value::Int(p as i64)));
            add("user", url.user.map(Value::str));
            add("pass", url.pass.map(Value::str));
            add("path", url.path.map(Value::str));
            add("query", url.query.map(Value::str));
            add("fragment", url.fragment.map(Value::str));
            Ok(Value::Array(Rc::new(out)))
        }
        "parse_str" => {
            // With its result argument this runs through the by-reference path
            args::<2, 2>(name, vals)?;
            Ok(Value::Null)
        }
        "http_build_query" => {
            let [data, prefix, sep, enc] = args::<1, 4>(name, vals)?;
            let data = data.unwrap().copy_out();
            let entries = match &data {
                Value::Array(a) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                Value::Object(o) if o.class.is_enum => {
                    return Err(Fatal::type_error(format!(
                        "http_build_query(): Argument #1 ($data) must not be an enum, {} given",
                        data.value_name()
                    )));
                }
                Value::Object(o) => public_props(o),
                other => {
                    return Err(Fatal::type_error(format!(
                        "http_build_query(): Argument #1 ($data) must be of type array, {} given",
                        other.value_name()
                    )));
                }
            };
            let prefix = match prefix {
                Some(p) => arg_str(rt, lname, 2, "numeric_prefix", &p)?,
                None => Vec::new(),
            };
            let sep = match sep {
                None | Some(Value::Null) => b"&".to_vec(),
                Some(s) => arg_str(rt, lname, 3, "arg_separator", &s)?,
            };
            let enc = match enc {
                Some(e) => arg_int(lname, 4, "encoding_type", &e)?,
                None => PHP_QUERY_RFC1738,
            };
            let addr = match &data {
                Value::Array(a) => Rc::as_ptr(a) as usize,
                Value::Object(o) => Rc::as_ptr(o) as usize,
                _ => 0,
            };
            let mut b = QueryBuilder { rt, out: Vec::new(), sep, raw: enc == PHP_QUERY_RFC3986, visiting: vec![addr] };
            let num_prefix = (!prefix.is_empty()).then_some(prefix.as_slice());
            b.hash(entries, num_prefix, None)?;
            Ok(Value::str(b.out))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
