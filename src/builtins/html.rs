//! htmlentities() and html_entity_decode() with php-src's entity tables and
//! rules for each document type

use super::entities::{HTML4, HTML5, HTML5_ENCODE};
use super::{arg_bool, arg_int, arg_str, args, Runtime};
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &["html_entity_decode", "htmlentities"];

const QUOTE_SINGLE: i64 = 1;
const QUOTE_DOUBLE: i64 = 2;
const IGNORE: i64 = 4;
const SUBSTITUTE: i64 = 8;
const DOC_MASK: i64 = 16 | 32;
const DOC_HTML401: i64 = 0;
const DOC_XML1: i64 = 16;
const DOC_XHTML: i64 = 32;
const DOC_HTML5: i64 = 16 | 32;
const DISALLOWED: i64 = 128;
/// The longest entity name php-src looks for
const LONGEST_ENTITY: usize = 32;

fn cp_allowed(cp: u32, doc: i64) -> bool {
    match doc {
        DOC_HTML401 => (0x20..=0x7e).contains(&cp) || matches!(cp, 0x09 | 0x0a | 0x0d) || (0xa0..=0xd7ff).contains(&cp) || (0xe000..=0x10ffff).contains(&cp),
        DOC_HTML5 => {
            (0x20..=0x7e).contains(&cp)
                || ((0x09..=0x0d).contains(&cp) && cp != 0x0b)
                || (0xa0..=0xd7ff).contains(&cp)
                || ((0xe000..=0x10ffff).contains(&cp) && (cp & 0xffff) < 0xfffe && !(0xfdd0..=0xfdef).contains(&cp))
        }
        _ => (0x20..=0xd7ff).contains(&cp) || matches!(cp, 0x09 | 0x0a | 0x0d) || ((0xe000..=0x10ffff).contains(&cp) && cp != 0xfffe && cp != 0xffff),
    }
}

fn push_utf8(out: &mut Vec<u8>, cp: u32) {
    let c = char::from_u32(cp).unwrap_or('\u{fffd}');
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

/// The code points of a named entity in the document type's table
fn named(name: &[u8], doc: i64) -> Option<(u32, u32)> {
    let name = std::str::from_utf8(name).ok()?;
    match doc {
        DOC_HTML5 => HTML5.binary_search_by(|(n, _, _)| n.cmp(&name)).ok().map(|i| (HTML5[i].1, HTML5[i].2)),
        DOC_XML1 => match name {
            "amp" => Some((38, 0)),
            "lt" => Some((60, 0)),
            "gt" => Some((62, 0)),
            "quot" => Some((34, 0)),
            "apos" => Some((39, 0)),
            _ => None,
        },
        _ => match HTML4.binary_search_by(|(n, _)| n.cmp(&name)) {
            Ok(i) => Some((HTML4[i].1, 0)),
            // XHTML uses the HTML 4 names, plus &apos;
            Err(_) if doc == DOC_XHTML && name == "apos" => Some((39, 0)),
            Err(_) => None,
        },
    }
}

/// traverse_for_entities with all entities
pub fn decode(s: &[u8], flags: i64) -> Vec<u8> {
    let doc = flags & DOC_MASK;
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'&' || s.len() - i < 4 {
            out.push(s[i]);
            i += 1;
            continue;
        }
        // The entity's code points and the position of its `;`
        let parsed = if s[i + 1] == b'#' {
            let hex = matches!(s.get(i + 2), Some(b'x' | b'X'));
            let start = i + 2 + hex as usize;
            let digits = s[start..].iter().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
            let end = start + digits;
            let cp = std::str::from_utf8(&s[start..end]).ok().and_then(|t| u64::from_str_radix(t, if hex { 16 } else { 10 }).ok());
            match cp {
                Some(cp) if digits > 0 && s.get(end) == Some(&b';') && cp <= 0x10ffff => {
                    let cp = cp as u32;
                    if !cp_allowed(cp, doc) || (doc == DOC_HTML5 && cp == 0x0d) { None } else { Some(((cp, 0), end)) }
                }
                _ => None,
            }
        } else {
            let rest = &s[i + 1..s.len().min(i + 2 + LONGEST_ENTITY)];
            rest.iter().position(|&c| c == b';').filter(|&n| n > 0).and_then(|n| named(&rest[..n], doc).map(|cps| (cps, i + 1 + n)))
        };
        let Some(((cp, cp2), end)) = parsed else {
            out.push(b'&');
            i += 1;
            continue;
        };
        // Quotes only decode when their quote style is on
        if (cp == 39 && flags & QUOTE_SINGLE == 0) || (cp == 34 && flags & QUOTE_DOUBLE == 0) {
            out.extend_from_slice(&s[i..end]);
            i = end;
            continue;
        }
        push_utf8(&mut out, cp);
        if cp2 != 0 {
            push_utf8(&mut out, cp2);
        }
        i = end + 1;
    }
    out
}

/// Whether `s` (at a `&`) starts an entity, which double_encode=false keeps
fn starts_entity(s: &[u8], doc: i64) -> Option<usize> {
    let rest = &s[1..];
    if rest.first() == Some(&b'#') {
        let hex = matches!(rest.get(1), Some(b'x' | b'X'));
        let start = 1 + hex as usize;
        let n = rest[start..].iter().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
        return (n > 0 && rest.get(start + n) == Some(&b';')).then_some(1 + start + n + 1);
    }
    let n = rest.iter().take(LONGEST_ENTITY + 1).position(|&c| c == b';')?;
    let name = &rest[..n];
    let valid = !name.is_empty() && name.iter().all(u8::is_ascii_alphanumeric);
    (valid && named(name, doc).is_some()).then_some(1 + n + 1)
}

/// The entity name htmlentities() uses for a code point (and the next one)
fn encode_name(cp: u32, next: Option<u32>, doc: i64) -> Option<(&'static str, bool)> {
    match doc {
        DOC_HTML5 => {
            let start = HTML5_ENCODE.partition_point(|(c, _, _)| *c < cp);
            let entries = HTML5_ENCODE[start..].iter().take_while(|(c, _, _)| *c == cp);
            let mut single = None;
            for (_, second, name) in entries {
                if *second == 0 {
                    single = Some((*name, false));
                } else if Some(*second) == next {
                    return Some((name, true));
                }
            }
            single
        }
        DOC_XML1 => None,
        _ => HTML4.iter().find(|(_, c)| *c == cp).map(|(n, _)| (*n, false)),
    }
}

fn encode(s: &[u8], flags: i64, double: bool) -> Vec<u8> {
    let doc = flags & DOC_MASK;
    let text = String::from_utf8_lossy(s);
    let valid = matches!(text, std::borrow::Cow::Borrowed(_));
    if !valid && flags & (IGNORE | SUBSTITUTE) == 0 {
        return Vec::new();
    }
    let chars: Vec<char> = if valid || flags & SUBSTITUTE != 0 {
        text.chars().collect()
    } else {
        // ENT_IGNORE drops the invalid sequences
        text.chars().filter(|&c| c != '\u{fffd}').collect()
    };
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    // Byte offsets of the characters, for `&` lookahead
    let raw: Vec<u8> = chars.iter().collect::<String>().into_bytes();
    let mut byte = 0;
    while i < chars.len() {
        let c = chars[i];
        let cp = c as u32;
        let len = c.len_utf8();
        match c {
            '&' if !double => match starts_entity(&raw[byte..], doc) {
                Some(n) => {
                    out.extend_from_slice(&raw[byte..byte + n]);
                    let consumed = std::str::from_utf8(&raw[byte..byte + n]).map_or(1, |t| t.chars().count());
                    i += consumed;
                    byte += n;
                    continue;
                }
                None => out.extend_from_slice(b"&amp;"),
            },
            '&' => out.extend_from_slice(b"&amp;"),
            '"' if flags & QUOTE_DOUBLE != 0 => out.extend_from_slice(b"&quot;"),
            '"' => out.push(b'"'),
            '\'' if flags & QUOTE_SINGLE != 0 => {
                out.extend_from_slice(if matches!(doc, DOC_XML1 | DOC_XHTML | DOC_HTML5) { b"&apos;" } else { b"&#039;" })
            }
            '\'' => out.push(b'\''),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            _ if flags & DISALLOWED != 0 && !cp_allowed(cp, doc) => out.extend_from_slice(b"&#xFFFD;"),
            _ => match encode_name(cp, chars.get(i + 1).map(|&n| n as u32), doc) {
                Some((name, pair)) if cp >= 0x80 || doc == DOC_HTML5 => {
                    out.push(b'&');
                    out.extend_from_slice(name.as_bytes());
                    out.push(b';');
                    if pair {
                        i += 1;
                        byte += chars[i].len_utf8();
                    }
                }
                _ => push_utf8(&mut out, cp),
            },
        }
        i += 1;
        byte += len;
    }
    out
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "html_entity_decode" => {
            let [s, flags, _encoding] = args::<1, 3>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let flags = match flags {
                Some(f) => arg_int(lname, 2, "flags", &f)?,
                None => QUOTE_SINGLE | QUOTE_DOUBLE | SUBSTITUTE,
            };
            Ok(Value::str(decode(&s, flags)))
        }
        "htmlentities" => {
            let [s, flags, _encoding, double] = args::<1, 4>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let flags = match flags {
                Some(f) => arg_int(lname, 2, "flags", &f)?,
                None => QUOTE_SINGLE | QUOTE_DOUBLE | SUBSTITUTE,
            };
            let double = match double {
                Some(d) => arg_bool(lname, 4, "double_encode", &d)?,
                None => true,
            };
            Ok(Value::str(encode(&s, flags, double)))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
