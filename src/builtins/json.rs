//! json_encode(), json_decode() and friends, after php-src's ext/json.
//!
//! Both directions avoid native recursion proportional to the input's nesting:
//! the decoder keeps its own stack, and the encoder stops at a fixed depth.

use std::cell::Cell;
use std::rc::Rc;

use super::{arg_bool, arg_int, arg_str, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::interp::{EResult, Fatal};
use crate::object::{unmangle, Internal, Object, Visibility};
use crate::value::{parse_numeric, with_deref, Num, Numeric, Value};

pub const NAMES: &[&str] = &["json_encode", "json_decode", "json_validate", "json_last_error", "json_last_error_msg"];

const HEX_TAG: i64 = 1 << 0;
const HEX_AMP: i64 = 1 << 1;
const HEX_APOS: i64 = 1 << 2;
const HEX_QUOT: i64 = 1 << 3;
const FORCE_OBJECT: i64 = 1 << 4;
const NUMERIC_CHECK: i64 = 1 << 5;
const UNESCAPED_SLASHES: i64 = 1 << 6;
const PRETTY_PRINT: i64 = 1 << 7;
const UNESCAPED_UNICODE: i64 = 1 << 8;
const PARTIAL_OUTPUT_ON_ERROR: i64 = 1 << 9;
const PRESERVE_ZERO_FRACTION: i64 = 1 << 10;
const UNESCAPED_LINE_TERMINATORS: i64 = 1 << 11;
const OBJECT_AS_ARRAY: i64 = 1 << 0;
const BIGINT_AS_STRING: i64 = 1 << 1;
const INVALID_UTF8_IGNORE: i64 = 1 << 20;
const INVALID_UTF8_SUBSTITUTE: i64 = 1 << 21;
const THROW_ON_ERROR: i64 = 1 << 22;

const DEFAULT_DEPTH: i64 = 512;
/// Nesting beyond this is a depth error whatever the depth argument, as
/// php-src's stack limit check makes it
const NATIVE_DEPTH: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Error {
    None = 0,
    Depth,
    StateMismatch,
    CtrlChar,
    Syntax,
    Utf8,
    Recursion,
    InfOrNan,
    UnsupportedType,
    InvalidPropertyName,
    Utf16,
    NonBackedEnum,
}

impl Error {
    fn msg(self) -> &'static str {
        match self {
            Error::None => "No error",
            Error::Depth => "Maximum stack depth exceeded",
            Error::StateMismatch => "State mismatch (invalid or malformed JSON)",
            Error::CtrlChar => "Control character error, possibly incorrectly encoded",
            Error::Syntax => "Syntax error",
            Error::Utf8 => "Malformed UTF-8 characters, possibly incorrectly encoded",
            Error::Recursion => "Recursion detected",
            Error::InfOrNan => "Inf and NaN cannot be JSON encoded",
            Error::UnsupportedType => "Type is not supported",
            Error::InvalidPropertyName => "The decoded property name is invalid",
            Error::Utf16 => "Single unpaired UTF-16 surrogate in unicode escape",
            Error::NonBackedEnum => "Non-backed enums have no default serialization",
        }
    }

    fn exception(self) -> Fatal {
        Fatal { code: self as i64, ..Fatal::new("JsonException", self.msg()) }
    }
}

thread_local! {
    static LAST_ERROR: Cell<Error> = const { Cell::new(Error::None) };
}

fn set_last(e: Error) {
    LAST_ERROR.with(|l| l.set(e));
}

pub fn constant(name: &str) -> Option<Value> {
    let rest = name.strip_prefix("JSON_")?;
    Some(Value::Int(match rest {
        "HEX_TAG" => HEX_TAG,
        "HEX_AMP" => HEX_AMP,
        "HEX_APOS" => HEX_APOS,
        "HEX_QUOT" => HEX_QUOT,
        "FORCE_OBJECT" => FORCE_OBJECT,
        "NUMERIC_CHECK" => NUMERIC_CHECK,
        "UNESCAPED_SLASHES" => UNESCAPED_SLASHES,
        "PRETTY_PRINT" => PRETTY_PRINT,
        "UNESCAPED_UNICODE" => UNESCAPED_UNICODE,
        "PARTIAL_OUTPUT_ON_ERROR" => PARTIAL_OUTPUT_ON_ERROR,
        "PRESERVE_ZERO_FRACTION" => PRESERVE_ZERO_FRACTION,
        "UNESCAPED_LINE_TERMINATORS" => UNESCAPED_LINE_TERMINATORS,
        "OBJECT_AS_ARRAY" => OBJECT_AS_ARRAY,
        "BIGINT_AS_STRING" => BIGINT_AS_STRING,
        "INVALID_UTF8_IGNORE" => INVALID_UTF8_IGNORE,
        "INVALID_UTF8_SUBSTITUTE" => INVALID_UTF8_SUBSTITUTE,
        "THROW_ON_ERROR" => THROW_ON_ERROR,
        "ERROR_NONE" => Error::None as i64,
        "ERROR_DEPTH" => Error::Depth as i64,
        "ERROR_STATE_MISMATCH" => Error::StateMismatch as i64,
        "ERROR_CTRL_CHAR" => Error::CtrlChar as i64,
        "ERROR_SYNTAX" => Error::Syntax as i64,
        "ERROR_UTF8" => Error::Utf8 as i64,
        "ERROR_RECURSION" => Error::Recursion as i64,
        "ERROR_INF_OR_NAN" => Error::InfOrNan as i64,
        "ERROR_UNSUPPORTED_TYPE" => Error::UnsupportedType as i64,
        "ERROR_INVALID_PROPERTY_NAME" => Error::InvalidPropertyName as i64,
        "ERROR_UTF16" => Error::Utf16 as i64,
        "ERROR_NON_BACKED_ENUM" => Error::NonBackedEnum as i64,
        _ => return None,
    }))
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "json_encode" => {
            let [v, flags, depth] = args::<1, 3>(name, vals)?;
            let flags = match flags {
                Some(f) => arg_int(lname, 2, "flags", &f)?,
                None => 0,
            };
            let depth = match depth {
                Some(d) => arg_int(lname, 3, "depth", &d)?,
                None => DEFAULT_DEPTH,
            };
            let mut enc = Encoder { rt, flags, max_depth: depth, depth: 0, error: Error::None, visiting: Vec::new() };
            let mut out = Vec::new();
            let _ = enc.value(&mut out, &v.unwrap())?;
            let error = enc.error;
            let partial = flags & PARTIAL_OUTPUT_ON_ERROR != 0;
            if flags & THROW_ON_ERROR == 0 || partial {
                set_last(error);
                if error != Error::None && !partial {
                    return Ok(Value::Bool(false));
                }
            } else if error != Error::None {
                return Err(error.exception());
            }
            Ok(Value::str(out))
        }
        "json_decode" => {
            let [s, assoc, depth, flags] = args::<1, 4>(name, vals)?;
            let s = arg_str(rt, lname, 1, "json", &s.unwrap())?;
            let mut flags = match flags {
                Some(f) => arg_int(lname, 4, "flags", &f)?,
                None => 0,
            };
            let depth = match depth {
                Some(d) => arg_int(lname, 3, "depth", &d)?,
                None => DEFAULT_DEPTH,
            };
            let throw = flags & THROW_ON_ERROR != 0;
            if !throw {
                set_last(Error::None);
            }
            if s.is_empty() {
                if throw {
                    return Err(Error::Syntax.exception());
                }
                set_last(Error::Syntax);
                return Ok(Value::Null);
            }
            if depth <= 0 {
                return Err(Fatal::value_error("json_decode(): Argument #3 ($depth) must be greater than 0"));
            }
            if depth > i32::MAX as i64 {
                return Err(Fatal::value_error(format!("json_decode(): Argument #3 ($depth) must be less than {}", i32::MAX)));
            }
            // The bool overrides JSON_OBJECT_AS_ARRAY; null leaves the flag deciding
            match assoc {
                None | Some(Value::Null) => {}
                Some(a) => {
                    if arg_bool(lname, 2, "associative", &a)? {
                        flags |= OBJECT_AS_ARRAY;
                    } else {
                        flags &= !OBJECT_AS_ARRAY;
                    }
                }
            }
            match decode(rt, &s, flags, depth as usize, false) {
                Ok(v) => Ok(v),
                Err(e) if throw => Err(e.exception()),
                Err(e) => {
                    set_last(e);
                    Ok(Value::Null)
                }
            }
        }
        "json_validate" => {
            let [s, depth, flags] = args::<1, 3>(name, vals)?;
            let s = arg_str(rt, lname, 1, "json", &s.unwrap())?;
            let depth = match depth {
                Some(d) => arg_int(lname, 2, "depth", &d)?,
                None => DEFAULT_DEPTH,
            };
            let flags = match flags {
                Some(f) => arg_int(lname, 3, "flags", &f)?,
                None => 0,
            };
            if flags != 0 && flags != INVALID_UTF8_IGNORE {
                return Err(Fatal::value_error(
                    "json_validate(): Argument #3 ($flags) must be a valid flag (allowed flags: JSON_INVALID_UTF8_IGNORE)",
                ));
            }
            if s.is_empty() {
                set_last(Error::Syntax);
                return Ok(Value::Bool(false));
            }
            set_last(Error::None);
            if depth <= 0 {
                return Err(Fatal::value_error("json_validate(): Argument #2 ($depth) must be greater than 0"));
            }
            if depth > i32::MAX as i64 {
                return Err(Fatal::value_error(format!("json_validate(): Argument #2 ($depth) must be less than {}", i32::MAX)));
            }
            match decode(rt, &s, flags, depth as usize, true) {
                Ok(_) => Ok(Value::Bool(true)),
                Err(e) => {
                    set_last(e);
                    Ok(Value::Bool(false))
                }
            }
        }
        "json_last_error" => {
            args::<0, 0>(name, vals)?;
            Ok(Value::Int(LAST_ERROR.with(Cell::get) as i64))
        }
        _ => {
            args::<0, 0>(name, vals)?;
            Ok(Value::str(LAST_ERROR.with(Cell::get).msg()))
        }
    }
}

// ---- Encoding ----

struct Encoder<'r> {
    rt: &'r mut dyn Runtime,
    flags: i64,
    max_depth: i64,
    depth: i64,
    error: Error,
    /// Arrays and objects being encoded, for recursion detection
    visiting: Vec<usize>,
}

const HEX: &[u8; 16] = b"0123456789abcdef";

impl Encoder<'_> {
    fn partial(&self) -> bool {
        self.flags & PARTIAL_OUTPUT_ON_ERROR != 0
    }

    /// Ok(false): failed with `self.error` set (the output is then only used
    /// with JSON_PARTIAL_OUTPUT_ON_ERROR)
    fn value(&mut self, out: &mut Vec<u8>, v: &Value) -> EResult<bool> {
        match v {
            Value::Ref(r) => {
                let inner = with_deref(r, Value::clone);
                self.value(out, &inner)
            }
            Value::Null => {
                out.extend_from_slice(b"null");
                Ok(true)
            }
            Value::Bool(b) => {
                out.extend_from_slice(if *b { b"true" } else { b"false" });
                Ok(true)
            }
            Value::Int(i) => {
                out.extend_from_slice(i.to_string().as_bytes());
                Ok(true)
            }
            Value::Float(f) if f.is_finite() => {
                self.double(out, *f);
                Ok(true)
            }
            Value::Float(_) => {
                self.error = Error::InfOrNan;
                out.push(b'0');
                Ok(false)
            }
            Value::Resource(_) => {
                self.error = Error::UnsupportedType;
                if self.partial() {
                    out.extend_from_slice(b"null");
                }
                Ok(false)
            }
            Value::Str(s) => Ok(self.string(out, s, self.flags)),
            Value::Array(a) => {
                let entries: Vec<(Option<Key>, Value)> = a.iter().map(|(k, v)| (Some(k.clone()), v.clone())).collect();
                let as_object = self.flags & FORCE_OBJECT != 0 || !a.keys().enumerate().all(|(i, k)| *k == Key::Int(i as i64));
                self.container(out, Rc::as_ptr(a) as usize, entries, as_object)
            }
            Value::Object(o) => self.object(out, o),
        }
    }

    fn object(&mut self, out: &mut Vec<u8>, o: &Rc<Object>) -> EResult<bool> {
        let addr = Rc::as_ptr(o) as usize;
        if o.class.is_subclass_of("JsonSerializable") {
            if self.visiting.contains(&addr) {
                self.error = Error::Recursion;
                if self.partial() {
                    out.extend_from_slice(b"null");
                }
                return Ok(false);
            }
            self.visiting.push(addr);
            let v = self.rt.call_method0(o, "jsonSerialize");
            self.visiting.pop();
            let v = v?;
            // `return $this` encodes the properties
            if let Value::Object(same) = &v
                && Rc::ptr_eq(same, o)
            {
                return self.plain_object(out, o);
            }
            self.visiting.push(addr);
            let r = self.value(out, &v);
            self.visiting.pop();
            return r;
        }
        if o.class.is_enum {
            if o.class.backing.is_none() {
                self.error = Error::NonBackedEnum;
                out.push(b'0');
                return Ok(false);
            }
            return self.value(out, &o.get("value"));
        }
        self.plain_object(out, o)
    }

    /// Public properties as a JSON object
    fn plain_object(&mut self, out: &mut Vec<u8>, o: &Rc<Object>) -> EResult<bool> {
        let entries = match &o.internal {
            Internal::None | Internal::DateTime(_) | Internal::TimeZone(_) | Internal::Interval(_) => o
                .export_props()
                .into_iter()
                .filter(|(k, _)| unmangle(k).1 == Visibility::Public)
                .map(|(k, v)| (Some(k), v))
                .collect(),
            _ => Vec::new(),
        };
        self.container(out, Rc::as_ptr(o) as usize, entries, true)
    }

    fn indent(&self, out: &mut Vec<u8>) {
        if self.flags & PRETTY_PRINT != 0 {
            out.push(b'\n');
            for _ in 0..self.depth {
                out.extend_from_slice(b"    ");
            }
        }
    }

    fn container(&mut self, out: &mut Vec<u8>, addr: usize, entries: Vec<(Option<Key>, Value)>, as_object: bool) -> EResult<bool> {
        if self.visiting.contains(&addr) {
            self.error = Error::Recursion;
            out.extend_from_slice(b"null");
            return Ok(false);
        }
        if self.depth as usize >= NATIVE_DEPTH || (self.depth >= self.max_depth && !self.partial()) {
            self.error = Error::Depth;
            if self.partial() {
                out.extend_from_slice(b"null");
            }
            return Ok(false);
        }
        self.visiting.push(addr);
        out.push(if as_object { b'{' } else { b'[' });
        self.depth += 1;
        let mut ok = true;
        let mut first = true;
        for (k, v) in &entries {
            if !first {
                out.push(b',');
            }
            first = false;
            self.indent(out);
            if as_object {
                match k {
                    Some(Key::Str(s)) => {
                        let (name, _, _) = unmangle(&Key::Str(s.clone()));
                        if !self.string(out, name.as_bytes(), self.flags & !NUMERIC_CHECK) && self.partial() {
                            // The key's "null" becomes an empty key
                            out.truncate(out.len() - 4);
                            out.extend_from_slice(b"\"\"");
                        }
                    }
                    Some(Key::Int(i)) => {
                        out.push(b'"');
                        out.extend_from_slice(i.to_string().as_bytes());
                        out.push(b'"');
                    }
                    None => {}
                }
                out.push(b':');
                if self.flags & PRETTY_PRINT != 0 {
                    out.push(b' ');
                }
            }
            if !self.value(out, v)? {
                ok = false;
                if !self.partial() {
                    self.visiting.pop();
                    return Ok(false);
                }
            }
        }
        self.visiting.pop();
        if self.depth > self.max_depth {
            self.error = Error::Depth;
            ok = false;
        }
        self.depth -= 1;
        if !first {
            self.indent(out);
        }
        out.push(if as_object { b'}' } else { b']' });
        Ok(ok)
    }

    fn double(&self, out: &mut Vec<u8>, f: f64) {
        let s = super::format::gcvt(f, None, 'e');
        out.extend_from_slice(s.as_bytes());
        if self.flags & PRESERVE_ZERO_FRACTION != 0 && !s.contains('.') {
            out.extend_from_slice(b".0");
        }
    }

    fn string(&mut self, out: &mut Vec<u8>, s: &[u8], flags: i64) -> bool {
        if s.is_empty() {
            out.extend_from_slice(b"\"\"");
            return true;
        }
        if flags & NUMERIC_CHECK != 0 {
            match parse_numeric(s) {
                (Num::Int(i), Numeric::Full) => {
                    out.extend_from_slice(i.to_string().as_bytes());
                    return true;
                }
                (Num::Float(f), Numeric::Full) if f.is_finite() => {
                    self.double(out, f);
                    return true;
                }
                _ => {}
            }
        }
        let checkpoint = out.len();
        out.push(b'"');
        let mut i = 0;
        let u = |out: &mut Vec<u8>, c: u32| {
            out.extend_from_slice(b"\\u");
            for shift in [12, 8, 4, 0] {
                out.push(HEX[((c >> shift) & 0xf) as usize]);
            }
        };
        while i < s.len() {
            let c = s[i];
            if c >= 0x80 {
                match next_utf8(&s[i..]) {
                    Some((cp, len)) => {
                        let unescaped = flags & UNESCAPED_UNICODE != 0
                            && (flags & UNESCAPED_LINE_TERMINATORS != 0 || !(0x2028..=0x2029).contains(&cp));
                        if unescaped {
                            out.extend_from_slice(&s[i..i + len]);
                        } else if cp >= 0x10000 {
                            let v = cp - 0x10000;
                            u(out, 0xd800 | (v >> 10));
                            u(out, 0xdc00 | (v & 0x3ff));
                        } else {
                            u(out, cp);
                        }
                        i += len;
                    }
                    None => {
                        i += 1;
                        if flags & INVALID_UTF8_IGNORE != 0 {
                            continue;
                        }
                        if flags & INVALID_UTF8_SUBSTITUTE != 0 {
                            if flags & UNESCAPED_UNICODE != 0 {
                                out.extend_from_slice("\u{fffd}".as_bytes());
                            } else {
                                out.extend_from_slice(b"\\ufffd");
                            }
                            continue;
                        }
                        out.truncate(checkpoint);
                        self.error = Error::Utf8;
                        if self.partial() {
                            out.extend_from_slice(b"null");
                        }
                        return false;
                    }
                }
                continue;
            }
            match c {
                b'"' if flags & HEX_QUOT != 0 => out.extend_from_slice(b"\\u0022"),
                b'"' => out.extend_from_slice(b"\\\""),
                b'\\' => out.extend_from_slice(b"\\\\"),
                b'/' if flags & UNESCAPED_SLASHES != 0 => out.push(b'/'),
                b'/' => out.extend_from_slice(b"\\/"),
                0x08 => out.extend_from_slice(b"\\b"),
                0x0c => out.extend_from_slice(b"\\f"),
                b'\n' => out.extend_from_slice(b"\\n"),
                b'\r' => out.extend_from_slice(b"\\r"),
                b'\t' => out.extend_from_slice(b"\\t"),
                b'<' if flags & HEX_TAG != 0 => out.extend_from_slice(b"\\u003C"),
                b'>' if flags & HEX_TAG != 0 => out.extend_from_slice(b"\\u003E"),
                b'&' if flags & HEX_AMP != 0 => out.extend_from_slice(b"\\u0026"),
                b'\'' if flags & HEX_APOS != 0 => out.extend_from_slice(b"\\u0027"),
                c if c < 0x20 => u(out, c as u32),
                c => out.push(c),
            }
            i += 1;
        }
        out.push(b'"');
        true
    }
}

/// A well-formed UTF-8 sequence at the start of `s`: its code point and length
fn next_utf8(s: &[u8]) -> Option<(u32, usize)> {
    let len = match s[0] {
        0x00..=0x7f => return Some((s[0] as u32, 1)),
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let seq = s.get(..len)?;
    std::str::from_utf8(seq).ok().and_then(|t| t.chars().next()).map(|c| (c as u32, len))
}

// ---- Decoding ----

enum Frame {
    Array(PhpArray),
    /// An object (or array with JSON_OBJECT_AS_ARRAY), the key of the member
    /// being read, and the object once its first member is in (php-src creates
    /// it then, which decides object ids)
    Object(PhpArray, Option<Vec<u8>>, Option<Rc<Object>>),
}

struct Scanner<'s> {
    s: &'s [u8],
    pos: usize,
    flags: i64,
}

#[derive(Debug)]
enum Tok {
    Punct(u8),
    Value(Value),
    Str(Vec<u8>),
    Eoi,
}

impl Scanner<'_> {
    fn next(&mut self) -> Result<Tok, Error> {
        while let Some(&c) = self.s.get(self.pos) {
            if matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let Some(&c) = self.s.get(self.pos) else { return Ok(Tok::Eoi) };
        let rest = &self.s[self.pos..];
        match c {
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                self.pos += 1;
                Ok(Tok::Punct(c))
            }
            b'"' => {
                self.pos += 1;
                self.string().map(Tok::Str)
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ if rest.starts_with(b"null") => {
                self.pos += 4;
                Ok(Tok::Value(Value::Null))
            }
            _ if rest.starts_with(b"true") => {
                self.pos += 4;
                Ok(Tok::Value(Value::Bool(true)))
            }
            _ if rest.starts_with(b"false") => {
                self.pos += 5;
                Ok(Tok::Value(Value::Bool(false)))
            }
            // A NUL ends the input only at its very end
            0 if self.pos + 1 == self.s.len() => Ok(Tok::Eoi),
            0x00..=0x1f => Err(Error::CtrlChar),
            _ if next_utf8(rest).is_some() => Err(Error::Syntax),
            _ => Err(Error::Utf8),
        }
    }

    fn number(&mut self) -> Result<Tok, Error> {
        let s = self.s;
        let start = self.pos;
        let mut i = start;
        let negative = s[i] == b'-';
        if negative {
            i += 1;
        }
        match s.get(i) {
            Some(b'0') => i += 1,
            Some(b'1'..=b'9') => {
                while s.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            // A lone "-" is just an unexpected character
            _ => return Err(Error::Syntax),
        }
        let int_end = i;
        let mut is_float = false;
        if s.get(i) == Some(&b'.') && s.get(i + 1).is_some_and(u8::is_ascii_digit) {
            i += 1;
            while s.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            is_float = true;
        }
        if matches!(s.get(i), Some(b'e' | b'E')) {
            let mut j = i + 1;
            if matches!(s.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if s.get(j).is_some_and(u8::is_ascii_digit) {
                while s.get(j).is_some_and(u8::is_ascii_digit) {
                    j += 1;
                }
                i = j;
                is_float = true;
            }
        }
        self.pos = i;
        let text = std::str::from_utf8(&s[start..i]).unwrap_or("0");
        if is_float {
            return Ok(Tok::Value(Value::Float(text.parse().unwrap_or(0.0))));
        }
        // Integers beyond PHP_INT_MAX become floats, or strings with JSON_BIGINT_AS_STRING
        let digits = int_end - start - usize::from(negative);
        let bigint = match digits.cmp(&19) {
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => {
                let d = &s[start + usize::from(negative)..int_end];
                let cmp = d.cmp(b"9223372036854775808".as_slice());
                !(cmp.is_lt() || (cmp.is_eq() && negative))
            }
        };
        if !bigint {
            return Ok(Tok::Value(Value::Int(text.parse().unwrap_or(0))));
        }
        if self.flags & BIGINT_AS_STRING != 0 {
            return Ok(Tok::Str(text.as_bytes().to_vec()));
        }
        Ok(Tok::Value(Value::Float(text.parse().unwrap_or(0.0))))
    }

    /// After the opening quote
    fn string(&mut self) -> Result<Vec<u8>, Error> {
        let s = self.s;
        let mut out = Vec::new();
        let hex4 = |at: usize| -> Option<u32> {
            let h = s.get(at..at + 4)?;
            std::str::from_utf8(h).ok().and_then(|t| u32::from_str_radix(t, 16).ok()).filter(|_| h.iter().all(u8::is_ascii_hexdigit))
        };
        loop {
            let Some(&c) = s.get(self.pos) else { return Err(Error::Syntax) };
            match c {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let esc = s.get(self.pos + 1).copied();
                    let simple = match esc {
                        Some(b'"') => Some(b'"'),
                        Some(b'\\') => Some(b'\\'),
                        Some(b'/') => Some(b'/'),
                        Some(b'b') => Some(0x08),
                        Some(b'f') => Some(0x0c),
                        Some(b'n') => Some(b'\n'),
                        Some(b'r') => Some(b'\r'),
                        Some(b't') => Some(b'\t'),
                        _ => None,
                    };
                    if let Some(b) = simple {
                        out.push(b);
                        self.pos += 2;
                        continue;
                    }
                    if esc != Some(b'u') {
                        return Err(Error::Syntax);
                    }
                    let Some(cp) = hex4(self.pos + 2) else { return Err(Error::Syntax) };
                    self.pos += 6;
                    let cp = if (0xd800..0xdc00).contains(&cp) {
                        // A high surrogate needs its low half right after it
                        let low = (s.get(self.pos..self.pos + 2) == Some(b"\\u")).then(|| hex4(self.pos + 2)).flatten();
                        match low {
                            Some(lo) if (0xdc00..0xe000).contains(&lo) => {
                                self.pos += 6;
                                0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00)
                            }
                            _ => return Err(Error::Utf16),
                        }
                    } else if (0xdc00..0xe000).contains(&cp) {
                        return Err(Error::Utf16);
                    } else {
                        cp
                    };
                    let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                    let mut buf = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0x00..=0x1f => return Err(Error::CtrlChar),
                0x80.. => match next_utf8(&s[self.pos..]) {
                    Some((_, len)) => {
                        out.extend_from_slice(&s[self.pos..self.pos + len]);
                        self.pos += len;
                    }
                    None if self.flags & INVALID_UTF8_SUBSTITUTE != 0 => {
                        out.extend_from_slice("\u{fffd}".as_bytes());
                        self.pos += 1;
                    }
                    None if self.flags & INVALID_UTF8_IGNORE != 0 => self.pos += 1,
                    None => return Err(Error::Utf8),
                },
                _ => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }
}

/// Parses JSON without native recursion. `validate` only checks it
fn decode(rt: &mut dyn Runtime, s: &[u8], flags: i64, max_depth: usize, validate: bool) -> Result<Value, Error> {
    let assoc = flags & OBJECT_AS_ARRAY != 0;
    let mut sc = Scanner { s, pos: 0, flags };
    let mut stack: Vec<Frame> = Vec::new();
    // What the parser expects next
    #[derive(PartialEq)]
    enum Want {
        Value,
        /// A value or `]` right after `[`
        FirstElem,
        /// A key or `}` right after `{`
        FirstKey,
        Key,
        Colon,
        /// `,` or the closing bracket
        Next,
    }
    let mut want = Want::Value;
    let mut result: Option<Value> = None;
    let make_object = |rt: &mut dyn Runtime, members: PhpArray, obj: Option<Rc<Object>>| -> Result<Value, Error> {
        if assoc || validate {
            return Ok(Value::Array(Rc::new(members)));
        }
        let obj = match obj {
            Some(o) => o,
            None => rt.new_object("stdClass").ok_or(Error::Syntax)?,
        };
        for (k, v) in members.iter() {
            obj.set_key(k.clone(), v.clone());
        }
        Ok(Value::Object(obj))
    };
    loop {
        let tok = sc.next()?;
        // A complete value: hand it to the enclosing container, or finish
        let mut done: Option<Value> = None;
        match (&want, tok) {
            (Want::Value | Want::FirstElem, Tok::Value(v)) => done = Some(v),
            (Want::Value | Want::FirstElem, Tok::Str(s)) => done = Some(Value::str(s)),
            (Want::Value | Want::FirstElem, Tok::Punct(b'[')) => {
                // php-src starts counting at 1
                if stack.len() + 1 >= max_depth.min(NATIVE_DEPTH * 100) {
                    return Err(Error::Depth);
                }
                stack.push(Frame::Array(PhpArray::new()));
                want = Want::FirstElem;
            }
            (Want::Value | Want::FirstElem, Tok::Punct(b'{')) => {
                // php-src starts counting at 1
                if stack.len() + 1 >= max_depth.min(NATIVE_DEPTH * 100) {
                    return Err(Error::Depth);
                }
                stack.push(Frame::Object(PhpArray::new(), None, None));
                want = Want::FirstKey;
            }
            (Want::FirstElem, Tok::Punct(b']')) | (Want::Next, Tok::Punct(b']')) if matches!(stack.last(), Some(Frame::Array(_))) => {
                let Some(Frame::Array(a)) = stack.pop() else { unreachable!() };
                done = Some(Value::Array(Rc::new(a)));
            }
            (Want::FirstKey, Tok::Punct(b'}')) | (Want::Next, Tok::Punct(b'}')) if matches!(stack.last(), Some(Frame::Object(..))) => {
                let Some(Frame::Object(members, _, obj)) = stack.pop() else { unreachable!() };
                done = Some(make_object(rt, members, obj)?);
            }
            // A bracket closing the other kind of container
            (Want::Next | Want::FirstElem | Want::FirstKey, Tok::Punct(b']' | b'}')) => return Err(Error::StateMismatch),
            (Want::Next, Tok::Punct(b',')) => {
                want = match stack.last() {
                    Some(Frame::Array(_)) => Want::Value,
                    _ => Want::Key,
                };
            }
            (Want::Key | Want::FirstKey, Tok::Str(k)) => {
                if let Some(Frame::Object(_, key, _)) = stack.last_mut() {
                    *key = Some(k);
                }
                want = Want::Colon;
            }
            (Want::Colon, Tok::Punct(b':')) => want = Want::Value,
            (_, Tok::Eoi) if stack.is_empty() && result.is_some() => return Ok(result.unwrap()),
            _ => return Err(Error::Syntax),
        }
        let Some(v) = done else { continue };
        match stack.last_mut() {
            None => {
                if result.is_some() {
                    return Err(Error::Syntax);
                }
                result = Some(v);
                want = Want::Next;
            }
            Some(Frame::Array(a)) => {
                if !validate {
                    let _ = a.push(v);
                }
                want = Want::Next;
            }
            Some(Frame::Object(members, key, obj)) => {
                let k = key.take().unwrap_or_default();
                if !assoc && k.first() == Some(&0) {
                    return Err(Error::InvalidPropertyName);
                }
                if !validate {
                    // Numeric keys of arrays become ints; object properties stay strings
                    let k = if assoc { Key::from_bytes(&k) } else { Key::Str(k.into()) };
                    members.insert(k, v);
                    if !assoc && obj.is_none() {
                        *obj = Some(rt.new_object("stdClass").ok_or(Error::Syntax)?);
                    }
                }
                want = Want::Next;
            }
        }
        if stack.is_empty() {
            // Only the end of input may follow the top-level value
            match sc.next()? {
                Tok::Eoi => return Ok(result.take().unwrap()),
                _ => return Err(Error::Syntax),
            }
        }
    }
}
