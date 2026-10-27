//! The filter extension: filter_var() with the validate and sanitize filters,
//! their flags and options

use std::net::{Ipv4Addr, Ipv6Addr};
use std::rc::Rc;

use super::{arg_int, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &["filter_var", "filter_input", "filter_has_var", "filter_list", "filter_id"];

const REQUIRE_SCALAR: i64 = 33554432;
const REQUIRE_ARRAY: i64 = 16777216;
const FORCE_ARRAY: i64 = 67108864;
const NULL_ON_FAILURE: i64 = 134217728;
const VALIDATE_INT: i64 = 257;
const VALIDATE_BOOL: i64 = 258;
const VALIDATE_FLOAT: i64 = 259;
const VALIDATE_REGEXP: i64 = 272;
const VALIDATE_URL: i64 = 273;
const VALIDATE_EMAIL: i64 = 274;
const VALIDATE_IP: i64 = 275;
const VALIDATE_MAC: i64 = 276;
const VALIDATE_DOMAIN: i64 = 277;
const UNSAFE_RAW: i64 = 516;
const SANITIZE_STRING: i64 = 513;
const SANITIZE_ENCODED: i64 = 514;
const SANITIZE_SPECIAL_CHARS: i64 = 515;
const SANITIZE_FULL_SPECIAL_CHARS: i64 = 522;
const SANITIZE_EMAIL: i64 = 517;
const SANITIZE_URL: i64 = 518;
const SANITIZE_NUMBER_INT: i64 = 519;
const SANITIZE_NUMBER_FLOAT: i64 = 520;
const SANITIZE_ADD_SLASHES: i64 = 523;
const CALLBACK: i64 = 1024;
const ALLOW_OCTAL: i64 = 1;
const ALLOW_HEX: i64 = 2;
const STRIP_LOW: i64 = 4;
const STRIP_HIGH: i64 = 8;
const ENCODE_LOW: i64 = 16;
const ENCODE_HIGH: i64 = 32;
const ENCODE_AMP: i64 = 64;
const NO_ENCODE_QUOTES: i64 = 128;
const EMPTY_STRING_NULL: i64 = 256;
const STRIP_BACKTICK: i64 = 512;
const ALLOW_FRACTION: i64 = 4096;
const ALLOW_THOUSAND: i64 = 8192;
const ALLOW_SCIENTIFIC: i64 = 16384;
const PATH_REQUIRED: i64 = 262144;
const QUERY_REQUIRED: i64 = 524288;
const FLAG_IPV4: i64 = 1048576;
const FLAG_IPV6: i64 = 2097152;
const NO_RES_RANGE: i64 = 4194304;
const NO_PRIV_RANGE: i64 = 8388608;
const GLOBAL_RANGE: i64 = 536870912;

const FILTERS: &[(&str, i64)] = &[
    ("int", VALIDATE_INT),
    ("boolean", VALIDATE_BOOL),
    ("bool", VALIDATE_BOOL),
    ("float", VALIDATE_FLOAT),
    ("validate_regexp", VALIDATE_REGEXP),
    ("validate_domain", VALIDATE_DOMAIN),
    ("validate_url", VALIDATE_URL),
    ("validate_email", VALIDATE_EMAIL),
    ("validate_ip", VALIDATE_IP),
    ("validate_mac", VALIDATE_MAC),
    ("string", SANITIZE_STRING),
    ("stripped", SANITIZE_STRING),
    ("encoded", SANITIZE_ENCODED),
    ("special_chars", SANITIZE_SPECIAL_CHARS),
    ("full_special_chars", SANITIZE_FULL_SPECIAL_CHARS),
    ("unsafe_raw", UNSAFE_RAW),
    ("email", SANITIZE_EMAIL),
    ("url", SANITIZE_URL),
    ("number_int", SANITIZE_NUMBER_INT),
    ("number_float", SANITIZE_NUMBER_FLOAT),
    ("add_slashes", SANITIZE_ADD_SLASHES),
    ("callback", CALLBACK),
];

pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "INPUT_POST" => 0,
        "INPUT_GET" => 1,
        "INPUT_COOKIE" => 2,
        "INPUT_ENV" => 4,
        "INPUT_SERVER" => 5,
        "FILTER_FLAG_NONE" => 0,
        "FILTER_REQUIRE_SCALAR" => REQUIRE_SCALAR,
        "FILTER_REQUIRE_ARRAY" => REQUIRE_ARRAY,
        "FILTER_FORCE_ARRAY" => FORCE_ARRAY,
        "FILTER_NULL_ON_FAILURE" => NULL_ON_FAILURE,
        "FILTER_THROW_ON_FAILURE" => 268435456,
        "FILTER_VALIDATE_INT" => VALIDATE_INT,
        "FILTER_VALIDATE_BOOLEAN" | "FILTER_VALIDATE_BOOL" => VALIDATE_BOOL,
        "FILTER_VALIDATE_FLOAT" => VALIDATE_FLOAT,
        "FILTER_VALIDATE_REGEXP" => VALIDATE_REGEXP,
        "FILTER_VALIDATE_DOMAIN" => VALIDATE_DOMAIN,
        "FILTER_VALIDATE_URL" => VALIDATE_URL,
        "FILTER_VALIDATE_EMAIL" => VALIDATE_EMAIL,
        "FILTER_VALIDATE_IP" => VALIDATE_IP,
        "FILTER_VALIDATE_MAC" => VALIDATE_MAC,
        "FILTER_DEFAULT" | "FILTER_UNSAFE_RAW" => UNSAFE_RAW,
        "FILTER_SANITIZE_STRING" | "FILTER_SANITIZE_STRIPPED" => SANITIZE_STRING,
        "FILTER_SANITIZE_ENCODED" => SANITIZE_ENCODED,
        "FILTER_SANITIZE_SPECIAL_CHARS" => SANITIZE_SPECIAL_CHARS,
        "FILTER_SANITIZE_FULL_SPECIAL_CHARS" => SANITIZE_FULL_SPECIAL_CHARS,
        "FILTER_SANITIZE_EMAIL" => SANITIZE_EMAIL,
        "FILTER_SANITIZE_URL" => SANITIZE_URL,
        "FILTER_SANITIZE_NUMBER_INT" => SANITIZE_NUMBER_INT,
        "FILTER_SANITIZE_NUMBER_FLOAT" => SANITIZE_NUMBER_FLOAT,
        "FILTER_SANITIZE_ADD_SLASHES" => SANITIZE_ADD_SLASHES,
        "FILTER_CALLBACK" => CALLBACK,
        "FILTER_FLAG_ALLOW_OCTAL" => ALLOW_OCTAL,
        "FILTER_FLAG_ALLOW_HEX" => ALLOW_HEX,
        "FILTER_FLAG_STRIP_LOW" => STRIP_LOW,
        "FILTER_FLAG_STRIP_HIGH" => STRIP_HIGH,
        "FILTER_FLAG_STRIP_BACKTICK" => STRIP_BACKTICK,
        "FILTER_FLAG_ENCODE_LOW" => ENCODE_LOW,
        "FILTER_FLAG_ENCODE_HIGH" => ENCODE_HIGH,
        "FILTER_FLAG_ENCODE_AMP" => ENCODE_AMP,
        "FILTER_FLAG_NO_ENCODE_QUOTES" => NO_ENCODE_QUOTES,
        "FILTER_FLAG_EMPTY_STRING_NULL" => EMPTY_STRING_NULL,
        "FILTER_FLAG_ALLOW_FRACTION" => ALLOW_FRACTION,
        "FILTER_FLAG_ALLOW_THOUSAND" => ALLOW_THOUSAND,
        "FILTER_FLAG_ALLOW_SCIENTIFIC" => ALLOW_SCIENTIFIC,
        "FILTER_FLAG_PATH_REQUIRED" => PATH_REQUIRED,
        "FILTER_FLAG_QUERY_REQUIRED" => QUERY_REQUIRED,
        "FILTER_FLAG_IPV4" | "FILTER_FLAG_HOSTNAME" | "FILTER_FLAG_EMAIL_UNICODE" => FLAG_IPV4,
        "FILTER_FLAG_IPV6" => FLAG_IPV6,
        "FILTER_FLAG_NO_RES_RANGE" => NO_RES_RANGE,
        "FILTER_FLAG_NO_PRIV_RANGE" => NO_PRIV_RANGE,
        "FILTER_FLAG_GLOBAL_RANGE" => GLOBAL_RANGE,
        _ => return None,
    }))
}

/// What a filter needs: its flags and the 'options' array (or callback)
struct Spec {
    filter: i64,
    flags: i64,
    options: Option<Value>,
}

impl Spec {
    fn option(&self, name: &str) -> Option<Value> {
        match &self.options {
            Some(Value::Array(a)) => a.get(&Key::Str(name.as_bytes().into())).map(Value::copy_out),
            _ => None,
        }
    }
}

fn trim(s: &[u8]) -> &[u8] {
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | 0);
    let start = s.iter().position(|c| !ws(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|c| !ws(c)).map_or(start, |e| e + 1);
    &s[start..end]
}

fn validate_int(s: &[u8], spec: &Spec) -> Option<Value> {
    let s = trim(s);
    let (neg, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let lower = digits.to_ascii_lowercase();
    let n: i64 = if spec.flags & ALLOW_HEX != 0 && lower.starts_with(b"0x") {
        let h = &lower[2..];
        if h.is_empty() || !h.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        i64::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?
    } else if spec.flags & ALLOW_OCTAL != 0 && digits.len() > 1 && digits[0] == b'0' {
        let o = digits.strip_prefix(b"0o").or(digits.strip_prefix(b"0O")).unwrap_or(&digits[1..]);
        if o.is_empty() || !o.iter().all(|c| (b'0'..=b'7').contains(c)) {
            return None;
        }
        i64::from_str_radix(std::str::from_utf8(o).ok()?, 8).ok()?
    } else {
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) || (digits.len() > 1 && digits[0] == b'0') {
            return None;
        }
        let text = std::str::from_utf8(s).ok()?;
        return text.trim_start_matches('+').parse::<i64>().ok().and_then(|n| in_range(n, spec));
    };
    let n = if neg { n.checked_neg()? } else { n };
    in_range(n, spec)
}

fn in_range(n: i64, spec: &Spec) -> Option<Value> {
    if spec.option("min_range").is_some_and(|m| n < m.to_int()) || spec.option("max_range").is_some_and(|m| n > m.to_int()) {
        return None;
    }
    Some(Value::Int(n))
}

fn validate_float(s: &[u8], spec: &Spec) -> Option<Value> {
    let s = trim(s);
    let dec = spec.option("decimal").map_or(b'.', |d| d.to_bytes().first().copied().unwrap_or(b'.'));
    let thousand = spec.option("thousand").map(|t| t.to_bytes()).unwrap_or_else(|| b"',.".to_vec());
    let mut text = String::new();
    let mut i = 0;
    if matches!(s.first(), Some(b'-' | b'+')) {
        text.push(s[0] as char);
        i = 1;
    }
    let mut seen_digit = false;
    let mut seen_dec = false;
    let mut seen_exp = false;
    while i < s.len() {
        let c = s[i];
        if c.is_ascii_digit() {
            text.push(c as char);
            seen_digit = true;
        } else if c == dec && !seen_dec && !seen_exp {
            text.push('.');
            seen_dec = true;
        } else if (c == b'e' || c == b'E') && seen_digit && !seen_exp {
            text.push('e');
            seen_exp = true;
            if matches!(s.get(i + 1), Some(b'-' | b'+')) {
                i += 1;
                text.push(s[i] as char);
            }
        } else if spec.flags & ALLOW_THOUSAND != 0 && thousand.contains(&c) && seen_digit && !seen_dec && !seen_exp {
            // A thousands separator needs three digits after it
            if !s.get(i + 1..i + 4).is_some_and(|d| d.iter().all(u8::is_ascii_digit)) {
                return None;
            }
        } else {
            return None;
        }
        i += 1;
    }
    let f: f64 = text.parse().ok()?;
    if !seen_digit || !f.is_finite() {
        return None;
    }
    if spec.option("min_range").is_some_and(|m| f < m.to_float()) || spec.option("max_range").is_some_and(|m| f > m.to_float()) {
        return None;
    }
    Some(Value::Float(f))
}

fn validate_bool(s: &[u8], spec: &Spec) -> Option<Value> {
    match trim(s).to_ascii_lowercase().as_slice() {
        b"1" | b"true" | b"on" | b"yes" => Some(Value::Bool(true)),
        b"0" | b"false" | b"off" | b"no" | b"" => Some(Value::Bool(false)),
        _ if spec.flags & NULL_ON_FAILURE != 0 => None,
        _ => Some(Value::Bool(false)),
    }
}

fn valid_hostname(host: &[u8], dns_only: bool) -> bool {
    let host = host.strip_suffix(b".").unwrap_or(host);
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    host.split(|&c| c == b'.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && (!dns_only || (label.first() != Some(&b'-') && label.last() != Some(&b'-')))
            && label.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'-' || (!dns_only && c == b'_'))
    })
}

fn validate_email(s: &[u8]) -> bool {
    if s.len() > 320 {
        return false;
    }
    let Some(at) = s.iter().rposition(|&c| c == b'@') else { return false };
    let (local, domain) = (&s[..at], &s[at + 1..]);
    if local.is_empty() || local.len() > 64 {
        return false;
    }
    let local_ok = if local.first() == Some(&b'"') && local.last() == Some(&b'"') && local.len() >= 2 {
        local[1..local.len() - 1].iter().all(|&c| (32..127).contains(&c) && c != b'"' && c != b'\\')
    } else {
        let atext = |c: u8| c.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&c);
        local.split(|&c| c == b'.').all(|part| !part.is_empty() && part.iter().all(|&c| atext(c)))
    };
    if !local_ok {
        return false;
    }
    if domain.first() == Some(&b'[') && domain.last() == Some(&b']') {
        let inner = &domain[1..domain.len() - 1];
        let text = String::from_utf8_lossy(inner);
        return text.parse::<Ipv4Addr>().is_ok() || text.strip_prefix("IPv6:").is_some_and(|v6| v6.parse::<Ipv6Addr>().is_ok());
    }
    // php-src's pattern wants a dotted domain whose labels are letters, digits and inner hyphens
    domain.contains(&b'.') && valid_hostname(domain, true)
}

/// The characters FILTER_SANITIZE_URL keeps
fn url_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"$-_.+!*'(),{}|\\^~[]`<>#%\";/?:@&=".contains(&c)
}

fn validate_url(s: &[u8], spec: &Spec) -> bool {
    if !s.iter().all(|&c| url_char(c)) {
        return false;
    }
    let text = String::from_utf8_lossy(s);
    let Some(colon) = text.find(':') else { return false };
    let scheme = &text[..colon];
    if scheme.is_empty() || !scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
        return false;
    }
    let rest = &text[colon + 1..];
    let (host, after) = match rest.strip_prefix("//") {
        Some(r) => {
            let end = r.find(['/', '?', '#']).unwrap_or(r.len());
            (Some(&r[..end]), &r[end..])
        }
        None => (None, rest),
    };
    let lscheme = scheme.to_ascii_lowercase();
    if let Some(authority) = host {
        let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = if hostport.starts_with('[') {
            hostport
        } else {
            hostport.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map_or(hostport, |(h, _)| h)
        };
        if host.is_empty() && matches!(lscheme.as_str(), "http" | "https") {
            return false;
        }
        if !host.is_empty() && !host.starts_with('[') && !valid_hostname(host.as_bytes(), false) {
            return false;
        }
    } else if !matches!(lscheme.as_str(), "mailto" | "news" | "file") {
        // http and https need a host; only these schemes do without one
        return false;
    }
    let path = after.split(['?', '#']).next().unwrap_or("");
    if spec.flags & PATH_REQUIRED != 0 && path.is_empty() {
        return false;
    }
    if spec.flags & QUERY_REQUIRED != 0 && !after.contains('?') {
        return false;
    }
    true
}

fn validate_ip(s: &[u8], spec: &Spec) -> bool {
    let text = String::from_utf8_lossy(s);
    let want4 = spec.flags & FLAG_IPV4 != 0;
    let want6 = spec.flags & FLAG_IPV6 != 0;
    let (any4, any6) = (want4 || !want6, want6 || !want4);
    let no_priv = spec.flags & (NO_PRIV_RANGE | GLOBAL_RANGE) != 0;
    let no_res = spec.flags & (NO_RES_RANGE | GLOBAL_RANGE) != 0;
    if let Ok(v4) = text.parse::<Ipv4Addr>() {
        // Leading zeros are not accepted
        if !any4 || text.split('.').any(|p| p.len() > 1 && p.starts_with('0')) {
            return false;
        }
        let o = v4.octets();
        if no_priv && (o[0] == 10 || (o[0] == 172 && (16..32).contains(&o[1])) || (o[0] == 192 && o[1] == 168)) {
            return false;
        }
        if no_res && (o[0] == 0 || o[0] == 127 || (o[0] == 169 && o[1] == 254) || o[0] >= 240) {
            return false;
        }
        return true;
    }
    if let Ok(v6) = text.parse::<Ipv6Addr>() {
        if !any6 {
            return false;
        }
        let seg = v6.segments();
        if no_priv && (seg[0] & 0xfe00) == 0xfc00 {
            return false;
        }
        if no_res && (v6.is_loopback() || v6.is_unspecified() || (seg[0] & 0xffc0) == 0xfe80) {
            return false;
        }
        return true;
    }
    false
}

fn validate_mac(s: &[u8]) -> bool {
    let text = String::from_utf8_lossy(s);
    let hex = |p: &str, n: usize| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit());
    match () {
        _ if text.matches('.').count() == 2 => text.split('.').all(|p| hex(p, 4)),
        _ if text.matches(':').count() == 5 => text.split(':').all(|p| hex(p, 2)),
        _ if text.matches('-').count() == 5 => text.split('-').all(|p| hex(p, 2)),
        _ => false,
    }
}

/// The STRIP/ENCODE flags of the string filters
fn strip_encode(s: &[u8], flags: i64, encode: impl Fn(u8) -> bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        if (flags & STRIP_LOW != 0 && c < 32) || (flags & STRIP_HIGH != 0 && c >= 128) || (flags & STRIP_BACKTICK != 0 && c == b'`') {
            continue;
        }
        if encode(c) || (flags & ENCODE_LOW != 0 && c < 32) || (flags & ENCODE_HIGH != 0 && c >= 128) || (flags & ENCODE_AMP != 0 && c == b'&') {
            out.extend_from_slice(format!("&#{c};").as_bytes());
        } else {
            out.push(c);
        }
    }
    out
}

fn apply_scalar(rt: &mut dyn Runtime, v: &Value, spec: &Spec) -> EResult<Option<Value>> {
    if spec.filter == CALLBACK {
        let Some(cb) = &spec.options else {
            crate::diag::warning("filter_var(): First argument is expected to be a valid callback");
            return Ok(None);
        };
        return rt.call_callback(cb, vec![v.clone()]).map(Some);
    }
    let s = match v {
        Value::Bool(false) if spec.filter == VALIDATE_BOOL => b"".to_vec(),
        Value::Object(o) if o.class.find_method("__tostring").is_none() => return Ok(None),
        Value::Array(_) => return Ok(None),
        other => rt.stringify(other)?,
    };
    let f = spec.flags;
    Ok(match spec.filter {
        VALIDATE_INT => validate_int(&s, spec),
        VALIDATE_FLOAT => validate_float(&s, spec),
        VALIDATE_BOOL => return Ok(validate_bool(&s, spec)),
        VALIDATE_EMAIL => validate_email(&s).then(|| Value::str(s.clone())),
        VALIDATE_URL => validate_url(&s, spec).then(|| Value::str(s.clone())),
        VALIDATE_IP => validate_ip(&s, spec).then(|| Value::str(s.clone())),
        VALIDATE_MAC => validate_mac(&s).then(|| Value::str(s.clone())),
        VALIDATE_DOMAIN => valid_hostname(&s, f & FLAG_IPV4 != 0).then(|| Value::str(s.clone())),
        VALIDATE_REGEXP => {
            let Some(re) = spec.option("regexp") else {
                return Err(Fatal::value_error("filter_var(): \"regexp\" option missing"));
            };
            let m = rt.call_callback(&Value::str("preg_match"), vec![re, Value::str(s.clone())])?;
            (m.to_int() > 0).then(|| Value::str(s.clone()))
        }
        SANITIZE_SPECIAL_CHARS => Some(Value::str(strip_encode(&s, f, |c| matches!(c, b'"' | b'\'' | b'<' | b'>' | b'&') || c < 32))),
        SANITIZE_FULL_SPECIAL_CHARS => {
            let quotes = if f & NO_ENCODE_QUOTES != 0 { 0 } else { 3 };
            let mut out = Vec::with_capacity(s.len());
            for &c in &s {
                match c {
                    b'&' => out.extend_from_slice(b"&amp;"),
                    b'<' => out.extend_from_slice(b"&lt;"),
                    b'>' => out.extend_from_slice(b"&gt;"),
                    b'"' if quotes != 0 => out.extend_from_slice(b"&quot;"),
                    b'\'' if quotes != 0 => out.extend_from_slice(b"&#039;"),
                    c => out.push(c),
                }
            }
            Some(Value::str(out))
        }
        SANITIZE_STRING => {
            crate::diag::deprecated("Constant FILTER_SANITIZE_STRING is deprecated");
            let stripped = super::misc::strip_tags(&s, &[]);
            Some(Value::str(strip_encode(&stripped, f, |c| f & NO_ENCODE_QUOTES == 0 && matches!(c, b'"' | b'\''))))
        }
        SANITIZE_ENCODED => {
            let mut out = Vec::new();
            for c in strip_encode(&s, f & !(ENCODE_LOW | ENCODE_HIGH | ENCODE_AMP), |_| false) {
                if c.is_ascii_alphanumeric() || b"-._".contains(&c) {
                    out.push(c);
                } else {
                    out.extend_from_slice(format!("%{c:02X}").as_bytes());
                }
            }
            Some(Value::str(out))
        }
        SANITIZE_EMAIL => Some(Value::str(s.iter().copied().filter(|&c| c.is_ascii_alphanumeric() || b"!#$%&'*+-=?^_`{|}~@.[]".contains(&c)).collect::<Vec<u8>>())),
        SANITIZE_URL => Some(Value::str(s.iter().copied().filter(|&c| url_char(c)).collect::<Vec<u8>>())),
        SANITIZE_NUMBER_INT => Some(Value::str(s.iter().copied().filter(|&c| c.is_ascii_digit() || c == b'+' || c == b'-').collect::<Vec<u8>>())),
        SANITIZE_NUMBER_FLOAT => Some(Value::str(
            s.iter()
                .copied()
                .filter(|&c| {
                    c.is_ascii_digit()
                        || c == b'+'
                        || c == b'-'
                        || (f & ALLOW_FRACTION != 0 && c == b'.')
                        || (f & ALLOW_THOUSAND != 0 && c == b',')
                        || (f & ALLOW_SCIENTIFIC != 0 && (c == b'e' || c == b'E'))
                })
                .collect::<Vec<u8>>(),
        )),
        SANITIZE_ADD_SLASHES => {
            let mut out = Vec::with_capacity(s.len());
            for &c in &s {
                match c {
                    b'\'' | b'"' | b'\\' => {
                        out.push(b'\\');
                        out.push(c);
                    }
                    0 => out.extend_from_slice(b"\\0"),
                    c => out.push(c),
                }
            }
            Some(Value::str(out))
        }
        _ => {
            if f & EMPTY_STRING_NULL != 0 && s.is_empty() {
                return Ok(Some(Value::Null));
            }
            Some(Value::str(strip_encode(&s, f, |_| false)))
        }
    })
}

fn failure(spec: &Spec) -> Value {
    if let Some(d) = spec.option("default") {
        return d;
    }
    if spec.flags & NULL_ON_FAILURE != 0 { Value::Null } else { Value::Bool(false) }
}

fn apply(rt: &mut dyn Runtime, v: &Value, spec: &Spec, depth: usize) -> EResult<Value> {
    match v.copy_out() {
        Value::Array(a) if spec.flags & (REQUIRE_ARRAY | FORCE_ARRAY) != 0 && depth < 128 => {
            let mut out = PhpArray::new();
            for (k, item) in a.iter() {
                out.insert(k.clone(), apply(rt, item, spec, depth + 1)?);
            }
            Ok(Value::Array(Rc::new(out)))
        }
        Value::Array(_) => Ok(failure(spec)),
        scalar if spec.flags & REQUIRE_ARRAY != 0 && depth == 0 => {
            let _ = scalar;
            Ok(failure(spec))
        }
        scalar => {
            let r = apply_scalar(rt, &scalar, spec)?;
            let r = r.map_or_else(|| failure(spec), |v| v);
            if spec.flags & FORCE_ARRAY != 0 && depth == 0 {
                return Ok(super::list([r]));
            }
            Ok(r)
        }
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "filter_var" => {
            let [v, filter, options] = args::<1, 3>(name, vals)?;
            let filter = match filter {
                Some(f) => arg_int(lname, 2, "filter", &f)?,
                None => UNSAFE_RAW,
            };
            if !FILTERS.iter().any(|(_, id)| *id == filter) {
                crate::diag::warning("filter_var(): Unknown filter with ID ".to_string() + &filter.to_string());
                return Ok(Value::Bool(false));
            }
            let (flags, options) = match options.map(|o| o.copy_out()) {
                Some(Value::Array(a)) => {
                    let flags = a.get(&Key::Str((*b"flags").into())).map_or(0, Value::to_int);
                    (flags, a.get(&Key::Str((*b"options").into())).map(Value::copy_out))
                }
                Some(other) if filter == CALLBACK => (0, Some(other)),
                Some(other) => (other.to_int(), None),
                None => (0, None),
            };
            let spec = Spec { filter, flags, options };
            apply(rt, &v.unwrap(), &spec, 0)
        }
        "filter_input" => {
            // The CLI has no request input
            let [_, _, _, options] = args::<2, 4>(name, vals)?;
            let null_on_failure = options.is_some_and(|o| o.to_int() & NULL_ON_FAILURE != 0);
            Ok(if null_on_failure { Value::Bool(false) } else { Value::Null })
        }
        "filter_has_var" => {
            args::<2, 2>(name, vals)?;
            Ok(Value::Bool(false))
        }
        "filter_list" => {
            args::<0, 0>(name, vals)?;
            Ok(super::list(FILTERS.iter().map(|(n, _)| Value::str(*n))))
        }
        _ => {
            let [n] = args::<1, 1>(name, vals)?;
            let n = rt.stringify(&n.unwrap())?;
            Ok(FILTERS.iter().find(|(f, _)| f.as_bytes() == n.as_slice()).map_or(Value::Bool(false), |(_, id)| Value::Int(*id)))
        }
    }
}
