//! pack() and unpack(), as php-src's pack.c

use std::rc::Rc;

use super::{arg_int, arg_str, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::diag;
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &["pack", "unpack"];

/// The byte size of a numeric format code
fn int_size(code: u8) -> Option<usize> {
    Some(match code {
        b'c' | b'C' => 1,
        b's' | b'S' | b'n' | b'v' => 2,
        b'i' | b'I' | b'l' | b'L' | b'N' | b'V' | b'f' | b'g' | b'G' => 4,
        b'q' | b'Q' | b'J' | b'P' | b'd' | b'e' | b'E' => 8,
        _ => return None,
    })
}

/// A repeater: a count, `*` (None) or nothing (Some(1))
enum Rep {
    Count(usize),
    Star,
    Default,
}

fn read_rep(f: &[u8], i: &mut usize) -> Rep {
    if f.get(*i) == Some(&b'*') {
        *i += 1;
        return Rep::Star;
    }
    let start = *i;
    while f.get(*i).is_some_and(u8::is_ascii_digit) {
        *i += 1;
    }
    if *i == start {
        return Rep::Default;
    }
    Rep::Count(std::str::from_utf8(&f[start..*i]).ok().and_then(|s| s.parse().ok()).unwrap_or(usize::MAX))
}

fn put_int(out: &mut Vec<u8>, code: u8, v: i64) {
    let le = v.to_le_bytes();
    let be = v.to_be_bytes();
    match code {
        b'c' | b'C' => out.push(v as u8),
        b's' | b'S' | b'v' => out.extend_from_slice(&le[..2]),
        b'n' => out.extend_from_slice(&be[6..]),
        b'i' | b'I' | b'l' | b'L' | b'V' => out.extend_from_slice(&le[..4]),
        b'N' => out.extend_from_slice(&be[4..]),
        b'q' | b'Q' | b'P' => out.extend_from_slice(&le),
        _ => out.extend_from_slice(&be), // J
    }
}

fn put_float(out: &mut Vec<u8>, code: u8, f: f64) {
    match code {
        b'f' | b'g' => out.extend_from_slice(&(f as f32).to_le_bytes()),
        b'G' => out.extend_from_slice(&(f as f32).to_be_bytes()),
        b'd' | b'e' => out.extend_from_slice(&f.to_le_bytes()),
        _ => out.extend_from_slice(&f.to_be_bytes()), // E
    }
}

fn get_int(b: &[u8], code: u8) -> i64 {
    let mut le = [0u8; 8];
    match code {
        b'c' => b[0] as i8 as i64,
        b'C' => b[0] as i64,
        b's' => i16::from_le_bytes([b[0], b[1]]) as i64,
        b'S' | b'v' => u16::from_le_bytes([b[0], b[1]]) as i64,
        b'n' => u16::from_be_bytes([b[0], b[1]]) as i64,
        b'i' | b'l' => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64,
        b'I' | b'L' | b'V' => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64,
        b'N' => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64,
        b'J' => {
            le.copy_from_slice(&b[..8]);
            i64::from_be_bytes(le)
        }
        _ => {
            le.copy_from_slice(&b[..8]);
            i64::from_le_bytes(le)
        }
    }
}

fn get_float(b: &[u8], code: u8) -> f64 {
    match code {
        b'f' | b'g' => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        b'G' => f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64,
        b'd' | b'e' => f64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])),
        _ => f64::from_be_bytes(b[..8].try_into().unwrap_or([0; 8])),
    }
}

fn hex_digit(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

fn pack(rt: &mut dyn Runtime, format: &[u8], vals: Vec<Value>) -> EResult<Value> {
    let mut out: Vec<u8> = Vec::new();
    let mut args = vals.into_iter().peekable();
    let mut i = 0;
    while i < format.len() {
        let code = format[i];
        i += 1;
        let rep = read_rep(format, &mut i);
        let too_few = || Fatal::value_error(format!("Type {}: too few arguments", code as char));
        match code {
            b'a' | b'A' | b'Z' | b'h' | b'H' => {
                let Some(v) = args.next() else { return Err(too_few()) };
                let s = rt.stringify(&v)?;
                if matches!(code, b'h' | b'H') {
                    let mut n = match rep {
                        Rep::Star => s.len(),
                        Rep::Count(n) => n,
                        Rep::Default => 1,
                    };
                    if n > s.len() {
                        diag::warning(format!("pack(): Type {}: not enough characters in string", code as char));
                        n = s.len();
                    }
                    let mut bytes = vec![0u8; n.div_ceil(2)];
                    for (k, &c) in s[..n].iter().enumerate() {
                        let nib = hex_digit(c);
                        let high = (k % 2 == 0) == (code == b'H');
                        bytes[k / 2] |= if high { nib << 4 } else { nib };
                    }
                    out.extend_from_slice(&bytes);
                    continue;
                }
                let n = match rep {
                    Rep::Star => s.len() + usize::from(code == b'Z'),
                    Rep::Count(n) => n,
                    Rep::Default => 1,
                };
                let pad = if code == b'A' { b' ' } else { 0 };
                // Z keeps room for its terminating NUL
                let take = if code == b'Z' { s.len().min(n.saturating_sub(1)) } else { s.len().min(n) };
                out.extend_from_slice(&s[..take]);
                out.resize(out.len() + (n - take), pad);
            }
            b'x' => match rep {
                Rep::Star => diag::warning("pack(): Type x: '*' ignored"),
                Rep::Count(n) => out.resize(out.len() + n, 0),
                Rep::Default => out.push(0),
            },
            b'X' => {
                let n = match rep {
                    Rep::Star => {
                        diag::warning("pack(): Type X: '*' ignored");
                        0
                    }
                    Rep::Count(n) => n,
                    Rep::Default => 1,
                };
                if n > out.len() {
                    diag::warning("pack(): Type X: outside of string");
                }
                out.truncate(out.len().saturating_sub(n));
            }
            b'@' => {
                let n = match rep {
                    Rep::Count(n) => n,
                    Rep::Star => {
                        diag::warning("pack(): Type @: '*' ignored");
                        out.len()
                    }
                    Rep::Default => 1,
                };
                out.resize(n, 0);
            }
            c if int_size(c).is_some() => {
                let n = match rep {
                    Rep::Star => args.len(),
                    Rep::Count(n) => n,
                    Rep::Default => 1,
                };
                if n > args.len() {
                    return Err(too_few());
                }
                for _ in 0..n {
                    let v = args.next().unwrap_or(Value::Null);
                    if matches!(c, b'f' | b'g' | b'G' | b'd' | b'e' | b'E') {
                        put_float(&mut out, c, v.to_float());
                    } else {
                        put_int(&mut out, c, v.to_int());
                    }
                }
            }
            c => return Err(Fatal::value_error(format!("Type {}: unknown format code", c as char))),
        }
    }
    let unused = args.count();
    if unused > 0 {
        diag::warning(format!("pack(): {unused} arguments unused"));
    }
    Ok(Value::str(out))
}

fn unpack(format: &[u8], data: &[u8], offset: usize) -> EResult<Value> {
    let mut out = PhpArray::new();
    let mut pos = offset;
    for part in format.split(|&c| c == b'/') {
        let Some(&code) = part.first() else { continue };
        let mut i = 1;
        let rep = read_rep(part, &mut i);
        let name = String::from_utf8_lossy(&part[i..]).into_owned();
        let left = data.len().saturating_sub(pos);
        let key = |k: usize, single: bool| -> Key {
            if single && !name.is_empty() {
                Key::from_bytes(name.as_bytes())
            } else {
                Key::from_bytes(format!("{name}{}", k + 1).as_bytes())
            }
        };
        match code {
            b'a' | b'A' | b'Z' => {
                let n = match rep {
                    Rep::Star => left,
                    Rep::Count(n) => n,
                    Rep::Default => 1,
                };
                if n > left {
                    diag::warning(format!("unpack(): Type {}: not enough input, need {n}, have {left}", code as char));
                    return Ok(Value::Bool(false));
                }
                let mut s = &data[pos..pos + n];
                match code {
                    b'A' => {
                        let end = s.iter().rposition(|c| !b" \t\r\n\0".contains(c)).map_or(0, |e| e + 1);
                        s = &s[..end];
                    }
                    b'Z' => s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())],
                    _ => {}
                }
                out.insert(Key::from_bytes(if name.is_empty() { b"1" } else { name.as_bytes() }), Value::str(s.to_vec()));
                pos += n;
            }
            b'h' | b'H' => {
                let nibbles = match rep {
                    Rep::Star => left * 2,
                    Rep::Count(n) => n,
                    Rep::Default => 1,
                };
                let bytes = nibbles.div_ceil(2);
                if bytes > left {
                    diag::warning(format!("unpack(): Type {}: not enough input, need {bytes}, have {left}", code as char));
                    return Ok(Value::Bool(false));
                }
                let hex = b"0123456789abcdef";
                let mut s = Vec::with_capacity(nibbles);
                for k in 0..nibbles {
                    let b = data[pos + k / 2];
                    let high = (k % 2 == 0) == (code == b'H');
                    s.push(hex[if high { b >> 4 } else { b & 15 } as usize]);
                }
                out.insert(Key::from_bytes(if name.is_empty() { b"1" } else { name.as_bytes() }), Value::str(s));
                pos += bytes;
            }
            b'x' | b'X' | b'@' => {
                let n = match rep {
                    Rep::Count(n) => n,
                    Rep::Star => 0,
                    Rep::Default => 1,
                };
                match code {
                    b'x' => {
                        if n > left {
                            diag::warning("unpack(): Type x: outside of string");
                            return Ok(Value::Bool(false));
                        }
                        pos += n;
                    }
                    b'X' => {
                        if n > pos {
                            diag::warning("unpack(): Type X: outside of string");
                            return Ok(Value::Bool(false));
                        }
                        pos -= n;
                    }
                    _ => {
                        if offset + n > data.len() {
                            diag::warning("unpack(): Type @: outside of string");
                            return Ok(Value::Bool(false));
                        }
                        pos = offset + n;
                    }
                }
            }
            c if int_size(c).is_some() => {
                let size = int_size(c).unwrap_or(1);
                let (count, single) = match rep {
                    Rep::Star => (left / size, false),
                    Rep::Count(n) => (n, false),
                    Rep::Default => (1, true),
                };
                for k in 0..count {
                    if pos + size > data.len() {
                        diag::warning(format!(
                            "unpack(): Type {}: not enough input values, need {size} values but only {} were provided",
                            c as char,
                            data.len().saturating_sub(pos)
                        ));
                        return Ok(Value::Bool(false));
                    }
                    let b = &data[pos..pos + size];
                    let v = if matches!(c, b'f' | b'g' | b'G' | b'd' | b'e' | b'E') {
                        Value::Float(get_float(b, c))
                    } else {
                        Value::Int(get_int(b, c))
                    };
                    out.insert(key(k, single), v);
                    pos += size;
                }
            }
            c => return Err(Fatal::value_error(format!("Invalid format type {}", c as char))),
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(match lname {
        "pack" => {
            let mut it = vals.into_iter();
            let Some(f) = it.next() else { return Some(Err(super::arg_count(name, 1, 0))) };
            match arg_str(rt, lname, 1, "format", &f) {
                Ok(f) => pack(rt, &f, it.collect()),
                Err(e) => Err(e),
            }
        }
        _ => (|| {
            let [f, data, off] = args::<2, 3>(name, vals)?;
            let f = arg_str(rt, lname, 1, "format", &f.unwrap())?;
            let data = arg_str(rt, lname, 2, "string", &data.unwrap())?;
            let off = match off {
                Some(o) => arg_int(lname, 3, "offset", &o)?,
                None => 0,
            };
            if off < 0 || off as usize > data.len() {
                return Err(Fatal::value_error("unpack(): Argument #3 ($offset) must be contained in argument #2 ($data)"));
            }
            unpack(&f, &data, off as usize)
        })(),
    })
}
