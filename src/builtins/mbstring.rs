//! mbstring: the multibyte string functions over UTF-8 and the other
//! encodings php-rs knows (ASCII, ISO-8859-1, Windows-1252, UTF-16, UTF-32,
//! 8bit). Invalid input is split into maximal subparts and replaced by the
//! substitute character, as php-src's mbstring does.

use std::cell::RefCell;
use std::rc::Rc;

use super::{arg_bool, arg_int, arg_str, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "mb_internal_encoding", "mb_strlen", "mb_substr", "mb_strpos", "mb_strrpos", "mb_stripos", "mb_strripos",
    "mb_strtolower", "mb_strtoupper", "mb_convert_case", "mb_check_encoding", "mb_convert_encoding",
    "mb_detect_encoding", "mb_substitute_character", "mb_str_split", "mb_ord", "mb_chr", "mb_strwidth",
    "mb_strimwidth", "mb_substr_count", "mb_strstr", "mb_stristr", "mb_strrchr", "mb_strrichr", "mb_scrub",
    "mb_str_pad", "mb_trim", "mb_ltrim", "mb_rtrim", "mb_ucfirst", "mb_lcfirst", "mb_strcut",
    "mb_preferred_mime_name", "mb_list_encodings", "mb_encoding_aliases", "mb_encode_numericentity",
    "mb_decode_numericentity", "mb_language", "mb_http_output", "mb_http_input", "mb_detect_order", "mb_get_info",
    "mb_regex_encoding", "mb_output_handler",
];

const MB_CASE_UPPER: i64 = 0;
const MB_CASE_LOWER: i64 = 1;
const MB_CASE_TITLE: i64 = 2;
const MB_CASE_FOLD: i64 = 3;

pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "MB_CASE_UPPER" => MB_CASE_UPPER,
        "MB_CASE_LOWER" => MB_CASE_LOWER,
        "MB_CASE_TITLE" => MB_CASE_TITLE,
        "MB_CASE_FOLD" => MB_CASE_FOLD,
        "MB_CASE_UPPER_SIMPLE" => 4,
        "MB_CASE_LOWER_SIMPLE" => 5,
        "MB_CASE_TITLE_SIMPLE" => 6,
        "MB_CASE_FOLD_SIMPLE" => 7,
        _ => return None,
    }))
}

/// php-src's double-width ranges (eaw_table.h)
const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115f), (0x231a, 0x231b), (0x2329, 0x232a), (0x23e9, 0x23ec), (0x23f0, 0x23f0), (0x23f3, 0x23f3),
    (0x25fd, 0x25fe), (0x2614, 0x2615), (0x2630, 0x2637), (0x2648, 0x2653), (0x267f, 0x267f), (0x268a, 0x268f),
    (0x2693, 0x2693), (0x26a1, 0x26a1), (0x26aa, 0x26ab), (0x26bd, 0x26be), (0x26c4, 0x26c5), (0x26ce, 0x26ce),
    (0x26d4, 0x26d4), (0x26ea, 0x26ea), (0x26f2, 0x26f3), (0x26f5, 0x26f5), (0x26fa, 0x26fa), (0x26fd, 0x26fd),
    (0x2705, 0x2705), (0x270a, 0x270b), (0x2728, 0x2728), (0x274c, 0x274c), (0x274e, 0x274e), (0x2753, 0x2755),
    (0x2757, 0x2757), (0x2795, 0x2797), (0x27b0, 0x27b0), (0x27bf, 0x27bf), (0x2b1b, 0x2b1c), (0x2b50, 0x2b50),
    (0x2b55, 0x2b55), (0x2e80, 0x2e99), (0x2e9b, 0x2ef3), (0x2f00, 0x2fd5), (0x2ff0, 0x303e), (0x3041, 0x3096),
    (0x3099, 0x30ff), (0x3105, 0x312f), (0x3131, 0x318e), (0x3190, 0x31e5), (0x31ef, 0x321e), (0x3220, 0x3247),
    (0x3250, 0xa48c), (0xa490, 0xa4c6), (0xa960, 0xa97c), (0xac00, 0xd7a3), (0xf900, 0xfaff), (0xfe10, 0xfe19),
    (0xfe30, 0xfe52), (0xfe54, 0xfe66), (0xfe68, 0xfe6b), (0xff01, 0xff60), (0xffe0, 0xffe6), (0x16fe0, 0x16fe4),
    (0x16ff0, 0x16ff6), (0x17000, 0x18cd5), (0x18cff, 0x18d1e), (0x18d80, 0x18df2), (0x1aff0, 0x1aff3), (0x1aff5,
    0x1affb), (0x1affd, 0x1affe), (0x1b000, 0x1b122), (0x1b132, 0x1b132), (0x1b150, 0x1b152), (0x1b155, 0x1b155),
    (0x1b164, 0x1b167), (0x1b170, 0x1b2fb), (0x1d300, 0x1d356), (0x1d360, 0x1d376), (0x1f004, 0x1f004), (0x1f0cf,
    0x1f0cf), (0x1f18e, 0x1f18e), (0x1f191, 0x1f19a), (0x1f200, 0x1f202), (0x1f210, 0x1f23b), (0x1f240, 0x1f248),
    (0x1f250, 0x1f251), (0x1f260, 0x1f265), (0x1f300, 0x1f320), (0x1f32d, 0x1f335), (0x1f337, 0x1f37c), (0x1f37e,
    0x1f393), (0x1f3a0, 0x1f3ca), (0x1f3cf, 0x1f3d3), (0x1f3e0, 0x1f3f0), (0x1f3f4, 0x1f3f4), (0x1f3f8, 0x1f43e),
    (0x1f440, 0x1f440), (0x1f442, 0x1f4fc), (0x1f4ff, 0x1f53d), (0x1f54b, 0x1f54e), (0x1f550, 0x1f567), (0x1f57a,
    0x1f57a), (0x1f595, 0x1f596), (0x1f5a4, 0x1f5a4), (0x1f5fb, 0x1f64f), (0x1f680, 0x1f6c5), (0x1f6cc, 0x1f6cc),
    (0x1f6d0, 0x1f6d2), (0x1f6d5, 0x1f6d8), (0x1f6dc, 0x1f6df), (0x1f6eb, 0x1f6ec), (0x1f6f4, 0x1f6fc), (0x1f7e0,
    0x1f7eb), (0x1f7f0, 0x1f7f0), (0x1f90c, 0x1f93a), (0x1f93c, 0x1f945), (0x1f947, 0x1f9ff), (0x1fa70, 0x1fa7c),
    (0x1fa80, 0x1fa8a), (0x1fa8e, 0x1fac6), (0x1fac8, 0x1fac8), (0x1facd, 0x1fadc), (0x1fadf, 0x1faea), (0x1faef,
    0x1faf8), (0x20000, 0x2fffd), (0x30000, 0x3fffd),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Enc {
    Utf8,
    Ascii,
    Latin1,
    Cp1252,
    Utf16Be,
    Utf16Le,
    /// UTF-16 with an optional byte order mark, big-endian without one
    Utf16,
    Utf32Be,
    Utf32Le,
    Utf32,
    Bit8,
}

impl Enc {
    fn name(self) -> &'static str {
        match self {
            Enc::Utf8 => "UTF-8",
            Enc::Ascii => "ASCII",
            Enc::Latin1 => "ISO-8859-1",
            Enc::Cp1252 => "Windows-1252",
            Enc::Utf16Be => "UTF-16BE",
            Enc::Utf16Le => "UTF-16LE",
            Enc::Utf16 => "UTF-16",
            Enc::Utf32Be => "UTF-32BE",
            Enc::Utf32Le => "UTF-32LE",
            Enc::Utf32 => "UTF-32",
            Enc::Bit8 => "8bit",
        }
    }

    fn parse(name: &[u8]) -> Option<Enc> {
        let n: String = String::from_utf8_lossy(name).to_ascii_lowercase().chars().filter(|c| *c != '-' && *c != '_').collect();
        Some(match n.as_str() {
            "utf8" => Enc::Utf8,
            "ascii" | "usascii" | "us" | "iso646us" => Enc::Ascii,
            "iso88591" | "latin1" | "l1" => Enc::Latin1,
            "windows1252" | "cp1252" => Enc::Cp1252,
            "utf16be" => Enc::Utf16Be,
            "utf16le" => Enc::Utf16Le,
            "utf16" => Enc::Utf16,
            "utf32be" => Enc::Utf32Be,
            "utf32le" => Enc::Utf32Le,
            "utf32" => Enc::Utf32,
            "8bit" | "binary" => Enc::Bit8,
            _ => return None,
        })
    }
}

/// Windows-1252's 0x80..0x9f (0: undefined)
const CP1252: [u32; 32] = [
    0x20ac, 0, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017d, 0,
    0, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0, 0x017e, 0x0178,
];

/// A decoded unit: a code point, or an invalid sequence
type Unit = Result<char, ()>;

/// UTF-8 split into characters, an invalid sequence's maximal subpart being one unit
fn utf8_units(s: &[u8]) -> Vec<(Unit, usize, usize)> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        let (len, lo, hi) = match b {
            0x00..=0x7f => {
                out.push((Ok(b as char), i, i + 1));
                i += 1;
                continue;
            }
            0xc2..=0xdf => (2, 0x80, 0xbf),
            0xe0 => (3, 0xa0, 0xbf),
            0xe1..=0xec | 0xee..=0xef => (3, 0x80, 0xbf),
            0xed => (3, 0x80, 0x9f),
            0xf0 => (4, 0x90, 0xbf),
            0xf1..=0xf3 => (4, 0x80, 0xbf),
            0xf4 => (4, 0x80, 0x8f),
            _ => {
                out.push((Err(()), i, i + 1));
                i += 1;
                continue;
            }
        };
        // The second byte has a narrower range for some leads; the rest are 80..bf
        let mut j = i + 1;
        let mut ok = true;
        for k in 1..len {
            let range = if k == 1 { lo..=hi } else { 0x80..=0xbf };
            match s.get(j) {
                Some(c) if range.contains(c) => j += 1,
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let text = std::str::from_utf8(&s[i..j]).ok().and_then(|t| t.chars().next());
            out.push((text.ok_or(()), i, j));
        } else {
            out.push((Err(()), i, j));
        }
        i = j;
    }
    out
}

fn decode(s: &[u8], enc: Enc) -> Vec<Unit> {
    match enc {
        Enc::Utf8 => utf8_units(s).into_iter().map(|(u, _, _)| u).collect(),
        Enc::Ascii => s.iter().map(|&b| if b < 128 { Ok(b as char) } else { Err(()) }).collect(),
        Enc::Latin1 | Enc::Bit8 => s.iter().map(|&b| Ok(b as char)).collect(),
        Enc::Cp1252 => s
            .iter()
            .map(|&b| match b {
                0x80..=0x9f => char::from_u32(CP1252[(b - 0x80) as usize]).filter(|c| *c != '\0').ok_or(()),
                b => Ok(b as char),
            })
            .collect(),
        Enc::Utf16 | Enc::Utf16Be | Enc::Utf16Le => {
            let (mut be, mut s) = (enc != Enc::Utf16Le, s);
            if enc == Enc::Utf16 {
                if s.starts_with(&[0xff, 0xfe]) {
                    be = false;
                    s = &s[2..];
                } else if s.starts_with(&[0xfe, 0xff]) {
                    s = &s[2..];
                }
            }
            let units: Vec<u16> = s.chunks(2).map(|c| if c.len() == 2 { if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) } } else { 0xdc00 }).collect();
            char::decode_utf16(units).map(|r| r.map_err(|_| ())).collect()
        }
        Enc::Utf32 | Enc::Utf32Be | Enc::Utf32Le => {
            let be = enc != Enc::Utf32Le;
            s.chunks(4)
                .map(|c| {
                    if c.len() < 4 {
                        return Err(());
                    }
                    let n = if be { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) } else { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) };
                    char::from_u32(n).ok_or(())
                })
                .collect()
        }
    }
}

#[derive(Clone, Copy)]
enum Subst {
    Char(u32),
    None,
    Long,
    Entity,
}

thread_local! {
    static SUBST: RefCell<Subst> = const { RefCell::new(Subst::Char(0x3f)) };
    static INTERNAL: RefCell<Enc> = const { RefCell::new(Enc::Utf8) };
}

fn subst() -> Subst {
    SUBST.with(|s| *s.borrow())
}

fn internal() -> Enc {
    INTERNAL.with(|e| *e.borrow())
}

/// Encodes one character, None when the encoding cannot hold it
fn encode_char(out: &mut Vec<u8>, c: char, enc: Enc) -> bool {
    let cp = c as u32;
    match enc {
        Enc::Utf8 => {
            let mut b = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
        }
        Enc::Ascii if cp < 128 => out.push(cp as u8),
        Enc::Latin1 | Enc::Bit8 if cp < 256 => out.push(cp as u8),
        Enc::Cp1252 if cp < 128 || (0xa0..256).contains(&cp) => out.push(cp as u8),
        Enc::Cp1252 => match CP1252.iter().position(|&x| x == cp && x != 0) {
            Some(i) => out.push(0x80 + i as u8),
            None => return false,
        },
        Enc::Utf16 | Enc::Utf16Be | Enc::Utf16Le => {
            let mut b = [0u16; 2];
            for u in c.encode_utf16(&mut b) {
                out.extend_from_slice(&if enc == Enc::Utf16Le { u.to_le_bytes() } else { u.to_be_bytes() });
            }
        }
        Enc::Utf32 | Enc::Utf32Be => out.extend_from_slice(&cp.to_be_bytes()),
        Enc::Utf32Le => out.extend_from_slice(&cp.to_le_bytes()),
        _ => return false,
    }
    true
}

/// The substitute for an invalid sequence or an unencodable character
fn substitute(out: &mut Vec<u8>, original: Option<char>, enc: Enc) {
    match subst() {
        Subst::None => {}
        Subst::Char(cp) => {
            let c = char::from_u32(cp).unwrap_or('?');
            if !encode_char(out, c, enc) {
                out.push(b'?');
            }
        }
        Subst::Long => match original {
            Some(c) => out.extend_from_slice(format!("U+{:X}", c as u32).as_bytes()),
            None => out.push(b'?'),
        },
        Subst::Entity => match original {
            Some(c) => out.extend_from_slice(format!("&#x{:X};", c as u32).as_bytes()),
            None => out.push(b'?'),
        },
    }
}

fn encode(units: &[Unit], enc: Enc) -> Vec<u8> {
    let mut out = Vec::with_capacity(units.len());
    for u in units {
        match u {
            Ok(c) => {
                if !encode_char(&mut out, *c, enc) {
                    substitute(&mut out, Some(*c), enc);
                }
            }
            Err(()) => substitute(&mut out, None, enc),
        }
    }
    out
}

fn encoding_arg(rt: &mut dyn Runtime, fname: &str, n: usize, v: Option<Value>) -> EResult<Enc> {
    match v {
        None | Some(Value::Null) => Ok(internal()),
        Some(v) => {
            let name = arg_str(rt, fname, n, "encoding", &v)?;
            Enc::parse(&name).ok_or_else(|| {
                Fatal::value_error(format!(
                    "{fname}(): Argument #{n} ($encoding) must be a valid encoding, \"{}\" given",
                    String::from_utf8_lossy(&name)
                ))
            })
        }
    }
}

fn units_of(rt: &mut dyn Runtime, fname: &str, v: Value, enc: Enc) -> EResult<Vec<Unit>> {
    Ok(decode(&arg_str(rt, fname, 1, "string", &v)?, enc))
}

fn width(c: char) -> usize {
    let cp = c as u32;
    if cp >= 0x1100 && WIDE.iter().any(|&(a, b)| (a..=b).contains(&cp)) { 2 } else { 1 }
}

fn chars_eq_ci(a: &[Unit], b: &[Unit]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| match (x, y) {
        (Ok(x), Ok(y)) => x == y || fold(*x) == fold(*y),
        _ => false,
    })
}

fn fold(c: char) -> String {
    c.to_lowercase().collect()
}

/// The unit index of `needle` in `hay` from `from`, searching backwards with `rev`
fn find(hay: &[Unit], needle: &[Unit], from: usize, rev: bool, ci: bool) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    let matches = |i: usize| {
        let w = &hay[i..i + needle.len()];
        if ci { chars_eq_ci(w, needle) } else { w == needle }
    };
    let last = hay.len() - needle.len();
    if rev {
        (from..=last).rev().find(|&i| matches(i))
    } else {
        (from..=last).find(|&i| matches(i))
    }
}

fn case_map(units: &[Unit], mode: i64) -> Vec<Unit> {
    let text: String = units.iter().map(|u| u.unwrap_or('\u{fffd}')).collect();
    let mapped = match mode {
        MB_CASE_UPPER | 4 => text.to_uppercase(),
        MB_CASE_LOWER | 5 => text.to_lowercase(),
        MB_CASE_FOLD | 7 => {
            // Full case folding lowercases and expands ß
            text.to_lowercase().replace('ß', "ss")
        }
        _ => {
            let mut out = String::with_capacity(text.len());
            let mut in_word = false;
            for c in text.chars() {
                // Apostrophes and the like do not break a word
                let ignorable = matches!(c, '\'' | '\u{2019}' | '.' | ':' | '\u{b7}' | '\u{2018}') || c.is_ascii_digit() && in_word;
                if c.is_alphabetic() || c.is_numeric() {
                    if in_word {
                        out.extend(c.to_lowercase());
                    } else {
                        let title: String = match c {
                            'ǆ' | 'ǅ' | 'Ǆ' => "ǅ".into(),
                            'ǉ' | 'ǈ' | 'Ǉ' => "ǈ".into(),
                            'ǌ' | 'ǋ' | 'Ǌ' => "ǋ".into(),
                            'ǳ' | 'ǲ' | 'Ǳ' => "ǲ".into(),
                            'ß' => "Ss".into(),
                            c => c.to_uppercase().collect(),
                        };
                        out.push_str(&title);
                        in_word = true;
                    }
                } else {
                    out.push(c);
                    if !ignorable {
                        in_word = false;
                    }
                }
            }
            out
        }
    };
    // Invalid sequences stay invalid
    let mut result: Vec<Unit> = mapped.chars().map(Ok).collect();
    if units.iter().any(|u| u.is_err()) {
        let mut it = units.iter();
        result = mapped.chars().map(|c| if c == '\u{fffd}' && it.any(|u| u.is_err()) { Err(()) } else { Ok(c) }).collect();
    }
    result
}

fn is_mb_space(c: char) -> bool {
    c.is_whitespace() || c == '\0'
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn str_value(units: &[Unit], enc: Enc) -> Value {
    Value::str(encode(units, enc))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "mb_internal_encoding" | "mb_regex_encoding" | "mb_http_output" => {
            let [enc] = args::<0, 1>(name, vals)?;
            match enc {
                None | Some(Value::Null) => Ok(Value::str(if lname == "mb_regex_encoding" { "UTF-8" } else { internal().name() })),
                Some(e) => {
                    let e = encoding_arg(rt, lname, 1, Some(e))?;
                    if lname == "mb_internal_encoding" {
                        INTERNAL.with(|i| *i.borrow_mut() = e);
                    }
                    Ok(Value::Bool(true))
                }
            }
        }
        "mb_language" => {
            let [l] = args::<0, 1>(name, vals)?;
            Ok(if l.is_some_and(|l| !l.is_null()) { Value::Bool(true) } else { Value::str("neutral") })
        }
        "mb_http_input" => Ok(Value::Bool(false)),
        "mb_detect_order" => {
            let [o] = args::<0, 1>(name, vals)?;
            Ok(if o.is_some_and(|o| !o.is_null()) { Value::Bool(true) } else { super::list([Value::str("ASCII"), Value::str("UTF-8")]) })
        }
        "mb_get_info" => {
            let [t] = args::<0, 1>(name, vals)?;
            let t = match t {
                Some(t) => arg_str(rt, lname, 1, "type", &t)?,
                None => b"all".to_vec(),
            };
            Ok(match t.as_slice() {
                b"internal_encoding" | b"http_output" => Value::str(internal().name()),
                b"language" => Value::str("neutral"),
                _ => Value::Bool(false),
            })
        }
        "mb_output_handler" => {
            let [s, _] = args::<2, 2>(name, vals)?;
            Ok(s.unwrap())
        }
        "mb_strlen" => {
            let [s, enc] = args::<1, 2>(name, vals)?;
            let enc = encoding_arg(rt, lname, 2, enc)?;
            Ok(Value::Int(units_of(rt, lname, s.unwrap(), enc)?.len() as i64))
        }
        "mb_substr" => {
            let [s, start, len, enc] = args::<2, 4>(name, vals)?;
            let enc = encoding_arg(rt, lname, 4, enc)?;
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            let n = u.len() as i64;
            let start = arg_int(lname, 2, "start", &start.unwrap())?;
            let len = match len {
                Some(l) if !l.is_null() => Some(arg_int(lname, 3, "length", &l)?),
                _ => None,
            };
            let from = if start < 0 { (n + start).max(0) } else { start.min(n) };
            let to = match len {
                Some(l) if l < 0 => (n + l).max(from),
                Some(l) => from.saturating_add(l).min(n),
                None => n,
            };
            Ok(str_value(&u[from as usize..to.max(from) as usize], enc))
        }
        "mb_strpos" | "mb_strrpos" | "mb_stripos" | "mb_strripos" => {
            let [h, n, off, enc] = args::<2, 4>(name, vals)?;
            let enc = encoding_arg(rt, lname, 4, enc)?;
            let h = units_of(rt, lname, h.unwrap(), enc)?;
            let n = decode(&arg_str(rt, lname, 2, "needle", &n.unwrap())?, enc);
            let off = match off {
                Some(o) => arg_int(lname, 3, "offset", &o)?,
                None => 0,
            };
            let len = h.len() as i64;
            if off > len || off < -len {
                return Err(Fatal::value_error(format!("{lname}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)")));
            }
            let rev = lname.contains("rpos");
            let ci = lname.contains("ipos");
            let found = if rev {
                // A negative offset limits where the needle may start
                let (lo, hi) = if off >= 0 { (off as usize, h.len()) } else { (0, (len + off) as usize + n.len()) };
                let hi = hi.min(h.len());
                find(&h[..hi], &n, lo.min(hi), true, ci)
            } else {
                let from = if off < 0 { (len + off) as usize } else { off as usize };
                find(&h, &n, from, false, ci)
            };
            Ok(found.map_or(Value::Bool(false), |i| Value::Int(i as i64)))
        }
        "mb_substr_count" => {
            let [h, n, enc] = args::<2, 3>(name, vals)?;
            let enc = encoding_arg(rt, lname, 3, enc)?;
            let h = units_of(rt, lname, h.unwrap(), enc)?;
            let n = decode(&arg_str(rt, lname, 2, "needle", &n.unwrap())?, enc);
            if n.is_empty() {
                return Err(Fatal::value_error("mb_substr_count(): Argument #2 ($needle) must not be empty"));
            }
            let (mut count, mut i) = (0, 0);
            while let Some(p) = find(&h, &n, i, false, false) {
                count += 1;
                i = p + n.len();
            }
            Ok(Value::Int(count))
        }
        "mb_strstr" | "mb_stristr" | "mb_strrchr" | "mb_strrichr" => {
            let [h, n, before, enc] = args::<2, 4>(name, vals)?;
            let enc = encoding_arg(rt, lname, 4, enc)?;
            let h = units_of(rt, lname, h.unwrap(), enc)?;
            let n = decode(&arg_str(rt, lname, 2, "needle", &n.unwrap())?, enc);
            let before = match before {
                Some(b) => arg_bool(lname, 3, "before_needle", &b)?,
                None => false,
            };
            let rev = lname.contains("rchr") || lname.contains("richr");
            let ci = lname == "mb_stristr" || lname == "mb_strrichr";
            let found = if rev { find(&h, &n, 0, true, ci) } else { find(&h, &n, 0, false, ci) };
            Ok(match found {
                Some(i) if before => str_value(&h[..i], enc),
                Some(i) => str_value(&h[i..], enc),
                None => Value::Bool(false),
            })
        }
        "mb_strtolower" | "mb_strtoupper" | "mb_convert_case" | "mb_ucfirst" | "mb_lcfirst" => {
            let mut it = vals.into_iter();
            let s = it.next().unwrap_or(Value::Null);
            let (mode, enc) = match lname {
                "mb_convert_case" => {
                    let mode = arg_int(lname, 2, "mode", &it.next().unwrap_or(Value::Null))?;
                    if !(0..=7).contains(&mode) {
                        return Err(Fatal::value_error("mb_convert_case(): Argument #2 ($mode) must be one of the MB_CASE_* constants"));
                    }
                    (mode, encoding_arg(rt, lname, 3, it.next())?)
                }
                "mb_strtolower" | "mb_lcfirst" => (MB_CASE_LOWER, encoding_arg(rt, lname, 2, it.next())?),
                _ => (MB_CASE_UPPER, encoding_arg(rt, lname, 2, it.next())?),
            };
            let u = units_of(rt, lname, s, enc)?;
            if lname == "mb_ucfirst" || lname == "mb_lcfirst" {
                let Some((first, rest)) = u.split_first() else { return Ok(Value::str("")) };
                let mut head = if lname == "mb_ucfirst" { case_map(std::slice::from_ref(first), MB_CASE_TITLE) } else { case_map(std::slice::from_ref(first), mode) };
                head.extend_from_slice(rest);
                return Ok(str_value(&head, enc));
            }
            Ok(str_value(&case_map(&u, mode), enc))
        }
        "mb_check_encoding" => {
            let [v, enc] = args::<0, 2>(name, vals)?;
            let enc = encoding_arg(rt, lname, 2, enc)?;
            let check = |s: &[u8]| decode(s, enc).iter().all(Result::is_ok) && (enc != Enc::Utf16 || s.len() % 2 == 0);
            Ok(Value::Bool(match v.map(|v| v.copy_out()) {
                None | Some(Value::Null) => true,
                Some(Value::Array(a)) => a.iter().all(|(k, v)| {
                    let key_ok = match k {
                        Key::Str(s) => check(s),
                        Key::Int(_) => true,
                    };
                    key_ok && check(&v.to_bytes())
                }),
                Some(v) => check(&rt.stringify(&v)?),
            }))
        }
        "mb_scrub" => {
            let [s, enc] = args::<1, 2>(name, vals)?;
            let enc = encoding_arg(rt, lname, 2, enc)?;
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            Ok(str_value(&u, enc))
        }
        "mb_convert_encoding" => {
            let [s, to, from] = args::<2, 3>(name, vals)?;
            let to_enc = encoding_arg(rt, lname, 2, to)?;
            let from_enc = match from.map(|f| f.copy_out()) {
                None | Some(Value::Null) => internal(),
                Some(Value::Array(a)) => a.values().filter_map(|v| Enc::parse(&v.to_bytes())).next().unwrap_or(internal()),
                Some(f) => {
                    let f = rt.stringify(&f)?;
                    // A list of candidates picks the first that fits
                    let s = s.clone().map(|v| v.to_bytes()).unwrap_or_default();
                    f.split(|&c| c == b',')
                        .filter_map(|n| Enc::parse(n.trim_ascii()))
                        .find(|e| decode(&s, *e).iter().all(Result::is_ok))
                        .or_else(|| f.split(|&c| c == b',').filter_map(|n| Enc::parse(n.trim_ascii())).next())
                        .ok_or_else(|| {
                            Fatal::value_error(format!(
                                "mb_convert_encoding(): Argument #3 ($from_encoding) contains invalid encoding \"{}\"",
                                String::from_utf8_lossy(&f)
                            ))
                        })?
                }
            };
            match s.map(|v| v.copy_out()) {
                Some(Value::Array(a)) => {
                    let mut out = PhpArray::new();
                    for (k, v) in a.iter() {
                        out.insert(k.clone(), Value::str(encode(&decode(&v.to_bytes(), from_enc), to_enc)));
                    }
                    Ok(Value::Array(Rc::new(out)))
                }
                Some(v) => {
                    let s = rt.stringify(&v)?;
                    Ok(Value::str(encode(&decode(&s, from_enc), to_enc)))
                }
                None => Err(super::arg_count(name, 2, 0)),
            }
        }
        "mb_detect_encoding" => {
            let [s, list, strict] = args::<1, 3>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let list: Vec<Enc> = match list.map(|l| l.copy_out()) {
                None | Some(Value::Null) => vec![Enc::Ascii, Enc::Utf8],
                Some(Value::Array(a)) => a.values().filter_map(|v| Enc::parse(&v.to_bytes())).collect(),
                Some(l) => rt.stringify(&l)?.split(|&c| c == b',').filter_map(|n| Enc::parse(n.trim_ascii())).collect(),
            };
            let strict = match strict {
                Some(b) => arg_bool(lname, 3, "strict", &b)?,
                None => false,
            };
            let ok = |e: &Enc| decode(&s, *e).iter().all(Result::is_ok);
            Ok(match list.iter().find(|e| ok(e)) {
                Some(e) => Value::str(e.name()),
                None if !strict => list.first().map_or(Value::Bool(false), |e| Value::str(e.name())),
                None => Value::Bool(false),
            })
        }
        "mb_substitute_character" => {
            let [c] = args::<0, 1>(name, vals)?;
            match c.map(|c| c.copy_out()) {
                None | Some(Value::Null) => Ok(match subst() {
                    Subst::Char(cp) => Value::Int(cp as i64),
                    Subst::None => Value::str("none"),
                    Subst::Long => Value::str("long"),
                    Subst::Entity => Value::str("entity"),
                }),
                Some(Value::Str(s)) if matches!(s.to_ascii_lowercase().as_slice(), b"none" | b"long" | b"entity") => {
                    let v = match s.to_ascii_lowercase().as_slice() {
                        b"none" => Subst::None,
                        b"long" => Subst::Long,
                        _ => Subst::Entity,
                    };
                    SUBST.with(|x| *x.borrow_mut() = v);
                    Ok(Value::Bool(true))
                }
                Some(v) => {
                    let cp = arg_int(lname, 1, "substitute_character", &v)?;
                    if char::from_u32(cp as u32).is_none() || cp < 0 {
                        return Err(Fatal::value_error("mb_substitute_character(): Argument #1 ($substitute_character) is not a valid codepoint"));
                    }
                    SUBST.with(|x| *x.borrow_mut() = Subst::Char(cp as u32));
                    Ok(Value::Bool(true))
                }
            }
        }
        "mb_str_split" => {
            let [s, len, enc] = args::<1, 3>(name, vals)?;
            let len = match len {
                Some(l) => arg_int(lname, 2, "length", &l)?,
                None => 1,
            };
            if len < 1 {
                return Err(Fatal::value_error("mb_str_split(): Argument #2 ($length) must be greater than 0"));
            }
            let enc = encoding_arg(rt, lname, 3, enc)?;
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            Ok(super::list(u.chunks(len as usize).map(|c| str_value(c, enc))))
        }
        "mb_ord" => {
            let [s, enc] = args::<1, 2>(name, vals)?;
            let enc = encoding_arg(rt, lname, 2, enc)?;
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            match u.first() {
                None => Err(Fatal::value_error("mb_ord(): Argument #1 ($string) must not be empty")),
                Some(Ok(c)) => Ok(Value::Int(*c as i64)),
                Some(Err(())) => Ok(Value::Bool(false)),
            }
        }
        "mb_chr" => {
            let [cp, enc] = args::<1, 2>(name, vals)?;
            let enc = encoding_arg(rt, lname, 2, enc)?;
            let cp = arg_int(lname, 1, "codepoint", &cp.unwrap())?;
            let Some(c) = u32::try_from(cp).ok().and_then(char::from_u32) else { return Ok(Value::Bool(false)) };
            let mut out = Vec::new();
            Ok(if encode_char(&mut out, c, enc) { Value::str(out) } else { Value::Bool(false) })
        }
        "mb_strwidth" => {
            let [s, enc] = args::<1, 2>(name, vals)?;
            let enc = encoding_arg(rt, lname, 2, enc)?;
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            Ok(Value::Int(u.iter().map(|c| c.map_or(1, width)).sum::<usize>() as i64))
        }
        "mb_strimwidth" => {
            let [s, start, w, marker, enc] = args::<3, 5>(name, vals)?;
            let enc = encoding_arg(rt, lname, 5, enc)?;
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            let start = arg_int(lname, 2, "start", &start.unwrap())?;
            let w = arg_int(lname, 3, "width", &w.unwrap())?;
            let marker = match marker {
                Some(m) => decode(&arg_str(rt, lname, 4, "trim_marker", &m)?, enc),
                None => Vec::new(),
            };
            let n = u.len() as i64;
            let start = if start < 0 { (n + start).max(0) } else { start.min(n) } as usize;
            let rest = &u[start..];
            let wid = |s: &[Unit]| s.iter().map(|c| c.map_or(1, width)).sum::<usize>() as i64;
            let total = wid(rest);
            let w = if w < 0 { total + w } else { w };
            if total <= w {
                return Ok(str_value(rest, enc));
            }
            let avail = w - wid(&marker);
            let mut used = 0;
            let mut end = 0;
            for c in rest {
                let cw = c.map_or(1, width) as i64;
                if used + cw > avail {
                    break;
                }
                used += cw;
                end += 1;
            }
            let mut out = rest[..end].to_vec();
            out.extend_from_slice(&marker);
            Ok(str_value(&out, enc))
        }
        "mb_str_pad" => {
            let [s, len, pad, ty, enc] = args::<2, 5>(name, vals)?;
            let enc = encoding_arg(rt, lname, 5, enc)?;
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            let len = arg_int(lname, 2, "length", &len.unwrap())?;
            let pad = match pad {
                Some(p) => decode(&arg_str(rt, lname, 3, "pad_string", &p)?, enc),
                None => vec![Ok(' ')],
            };
            if pad.is_empty() {
                return Err(Fatal::value_error("mb_str_pad(): Argument #3 ($pad_string) must be a non-empty string"));
            }
            let ty = match ty {
                Some(t) => arg_int(lname, 4, "pad_type", &t)?,
                None => 1,
            };
            let missing = (len - u.len() as i64).max(0) as usize;
            let (left, right) = match ty {
                0 => (missing, 0),
                2 => (missing / 2, missing - missing / 2),
                _ => (0, missing),
            };
            let fill = |n: usize| pad.iter().cycle().take(n).cloned().collect::<Vec<Unit>>();
            let mut out = fill(left);
            out.extend_from_slice(&u);
            out.extend(fill(right));
            Ok(str_value(&out, enc))
        }
        "mb_trim" | "mb_ltrim" | "mb_rtrim" => {
            let [s, chars, enc] = args::<1, 3>(name, vals)?;
            let enc = encoding_arg(rt, lname, 3, enc)?;
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            let set: Option<Vec<Unit>> = match chars {
                Some(c) if !c.is_null() => Some(decode(&arg_str(rt, lname, 2, "characters", &c)?, enc)),
                _ => None,
            };
            let strip = |c: &Unit| match &set {
                Some(set) => set.contains(c),
                None => c.is_ok_and(is_mb_space),
            };
            let start = if lname == "mb_rtrim" { 0 } else { u.iter().position(|c| !strip(c)).unwrap_or(u.len()) };
            let end = if lname == "mb_ltrim" { u.len() } else { u.iter().rposition(|c| !strip(c)).map_or(start, |e| e + 1) };
            Ok(str_value(&u[start..end.max(start)], enc))
        }
        "mb_strcut" => {
            let [s, start, len, enc] = args::<2, 4>(name, vals)?;
            let enc = encoding_arg(rt, lname, 4, enc)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let n = s.len() as i64;
            let start = arg_int(lname, 2, "start", &start.unwrap())?;
            let start = if start < 0 { (n + start).max(0) } else { start.min(n) } as usize;
            let len = match len {
                Some(l) if !l.is_null() => arg_int(lname, 3, "length", &l)?,
                _ => n,
            };
            let end = if len < 0 { (n + len).max(start as i64) } else { (start as i64 + len).min(n) } as usize;
            if enc != Enc::Utf8 {
                return Ok(Value::str(s[start..end].to_vec()));
            }
            // Cuts move back to character boundaries
            let boundary = |mut i: usize| {
                while i > 0 && i < s.len() && (s[i] & 0xc0) == 0x80 {
                    i -= 1;
                }
                i
            };
            let (a, b) = (boundary(start), boundary(end));
            Ok(Value::str(s[a..b.max(a)].to_vec()))
        }
        "mb_preferred_mime_name" => {
            let [e] = args::<1, 1>(name, vals)?;
            let e = arg_str(rt, lname, 1, "encoding", &e.unwrap())?;
            let lower = String::from_utf8_lossy(&e).to_ascii_lowercase();
            Ok(match lower.as_str() {
                "sjis" | "shift_jis" => Value::str("Shift_JIS"),
                "eucjp" | "euc-jp" => Value::str("EUC-JP"),
                _ => match Enc::parse(&e) {
                    Some(Enc::Ascii) => Value::str("US-ASCII"),
                    Some(enc) => Value::str(enc.name()),
                    None => {
                        return Err(Fatal::value_error(format!(
                            "mb_preferred_mime_name(): Argument #1 ($encoding) must be a valid encoding, \"{}\" given",
                            String::from_utf8_lossy(&e)
                        )));
                    }
                },
            })
        }
        "mb_list_encodings" => {
            args::<0, 0>(name, vals)?;
            let all = [Enc::Bit8, Enc::Utf32, Enc::Utf32Be, Enc::Utf32Le, Enc::Utf16, Enc::Utf16Be, Enc::Utf16Le, Enc::Utf8, Enc::Ascii, Enc::Latin1, Enc::Cp1252];
            Ok(super::list(all.iter().map(|e| Value::str(e.name()))))
        }
        "mb_encoding_aliases" => {
            let [e] = args::<1, 1>(name, vals)?;
            let e = encoding_arg(rt, lname, 1, e)?;
            let aliases: &[&str] = match e {
                Enc::Utf8 => &["utf8"],
                Enc::Ascii => &["ANSI_X3.4-1968", "iso-ir-6", "ANSI_X3.4-1986", "ISO_646.irv:1991", "US-ASCII", "ISO646-US", "us", "IBM367", "IBM-367", "cp367", "csASCII"],
                Enc::Latin1 => &["ISO8859-1", "latin1"],
                Enc::Cp1252 => &["cp1252"],
                Enc::Bit8 => &["binary"],
                _ => &[],
            };
            Ok(super::list(aliases.iter().map(|a| Value::str(*a))))
        }
        "mb_encode_numericentity" | "mb_decode_numericentity" => {
            let [s, map, enc, hex] = args::<2, 4>(name, vals)?;
            let enc = encoding_arg(rt, lname, 3, enc)?;
            let map: Vec<i64> = match map.map(|m| m.copy_out()) {
                Some(Value::Array(a)) => a.values().map(Value::to_int).collect(),
                _ => return Err(Fatal::type_error(format!("{lname}(): Argument #2 ($map) must be of type array"))),
            };
            if map.len() % 4 != 0 {
                return Err(Fatal::value_error(format!("{lname}(): Argument #2 ($map) must have a multiple of 4 elements")));
            }
            let hex = hex.is_some_and(|h| h.to_bool());
            let u = units_of(rt, lname, s.unwrap(), enc)?;
            if lname == "mb_encode_numericentity" {
                let mut out: Vec<Unit> = Vec::new();
                for c in u {
                    let cp = c.map_or(-1, |c| c as i64);
                    let hit = map.chunks(4).find(|m| cp >= m[0] && cp <= m[1]);
                    match hit {
                        Some(m) => {
                            let v = ((cp + m[2]) & m[3]) as u32;
                            let text = if hex { format!("&#x{v:X};") } else { format!("&#{v};") };
                            out.extend(text.chars().map(Ok));
                        }
                        None => out.push(c),
                    }
                }
                return Ok(str_value(&out, enc));
            }
            let text: String = u.iter().map(|c| c.unwrap_or('\u{fffd}')).collect();
            let mut out = String::new();
            let mut rest = text.as_str();
            while let Some(p) = rest.find("&#") {
                out.push_str(&rest[..p]);
                let after = &rest[p + 2..];
                let (is_hex, digits) = match after.strip_prefix(['x', 'X']) {
                    Some(h) => (true, h),
                    None => (false, after),
                };
                let n = digits.chars().take_while(|c| if is_hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
                let parsed = (n > 0).then(|| i64::from_str_radix(&digits[..n], if is_hex { 16 } else { 10 }).ok()).flatten();
                let decoded = parsed.and_then(|v| {
                    map.chunks(4).find_map(|m| {
                        let cp = v - m[2];
                        (cp >= m[0] && cp <= m[1]).then(|| char::from_u32(cp as u32)).flatten()
                    })
                });
                match decoded {
                    Some(c) => {
                        out.push(c);
                        let consumed = 2 + usize::from(is_hex) + n + usize::from(digits[n..].starts_with(';'));
                        rest = &rest[p + consumed..];
                    }
                    None => {
                        out.push_str("&#");
                        rest = after;
                    }
                }
            }
            out.push_str(rest);
            Ok(Value::str(encode(&out.chars().map(Ok).collect::<Vec<_>>(), enc)))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
