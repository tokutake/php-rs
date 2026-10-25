//! sprintf() and friends, ported from php-src's formatted_print.c.

use super::{arg_str, args, expect_array, Runtime};
use crate::diag;
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &["sprintf", "vsprintf"];

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(match lname {
        "sprintf" => sprintf(rt, lname, vals).map(Value::str),
        _ => vsprintf(rt, name, lname, vals).map(Value::str),
    })
}

/// sprintf() and printf(): the format, then its arguments
pub fn sprintf(rt: &mut dyn Runtime, lname: &str, vals: Vec<Value>) -> EResult<Vec<u8>> {
    let mut vals = vals.into_iter();
    let Some(format) = vals.next() else {
        return Err(super::arg_count(lname, 1, 0));
    };
    let format = arg_str(rt, lname, 1, "format", &format)?;
    formatted_print(rt, lname, &format, &vals.collect::<Vec<_>>(), Some(1))
}

/// vsprintf() and vprintf(): the format and an array of arguments
pub fn vsprintf(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Vec<u8>> {
    let [format, values] = args::<2, 2>(name, vals)?;
    let format = arg_str(rt, lname, 1, "format", &format.unwrap())?;
    let arr = expect_array(lname, 2, "values", &values.unwrap())?;
    let values: Vec<Value> = arr.values().map(Value::copy_out).collect();
    formatted_print(rt, lname, &format, &values, None)
}

const FLOAT_PRECISION: usize = 6;
const MAX_FLOAT_PRECISION: usize = 53;

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Right,
}

struct Spec {
    width: usize,
    precision: Option<usize>,
    padding: u8,
    align: Align,
    always_sign: bool,
}

impl Spec {
    /// php_sprintf_appendstring: pads `s` to the width; zero padding goes after the sign
    fn append(&self, out: &mut Vec<u8>, s: &[u8], neg: bool, truncate: bool) {
        let s = match self.precision {
            Some(p) if truncate => &s[..p.min(s.len())],
            _ => s,
        };
        let npad = self.width.saturating_sub(s.len());
        let mut s = s;
        if self.align == Align::Right {
            if (neg || self.always_sign) && self.padding == b'0' && !s.is_empty() {
                out.push(s[0]);
                s = &s[1..];
            }
            out.extend(std::iter::repeat_n(self.padding, npad));
        }
        out.extend_from_slice(s);
        if self.align == Align::Left {
            out.extend(std::iter::repeat_n(self.padding, npad));
        }
    }
}

/// `nb_additional`: how many arguments precede the values, for the error
/// message (None for vsprintf, whose values come in an array)
pub(super) fn formatted_print(
    rt: &mut dyn Runtime,
    fname: &str,
    format: &[u8],
    args: &[Value],
    nb_additional: Option<usize>,
) -> EResult<Vec<u8>> {
    let mut out = Vec::with_capacity(format.len() + 16);
    let mut i = 0;
    let mut currarg = 0;
    let mut max_missing: Option<usize> = None;
    let at = |i: usize| format.get(i).copied().unwrap_or(0);
    // Reads digits; None when too large
    let number = |i: &mut usize| -> Option<usize> {
        let start = *i;
        while at(*i).is_ascii_digit() {
            *i += 1;
        }
        std::str::from_utf8(&format[start..*i]).ok()?.parse::<usize>().ok().filter(|&n| n < i32::MAX as usize)
    };
    // `N$`: an explicit argument number
    let argnum = |i: &mut usize| -> EResult<Option<usize>> {
        let mut j = *i;
        while at(j).is_ascii_digit() {
            j += 1;
        }
        if at(j) != b'$' {
            return Ok(None);
        }
        match number(i) {
            Some(n) if n > 0 => {
                *i += 1;
                Ok(Some(n - 1))
            }
            _ => Err(Fatal::value_error(format!(
                "Argument number specifier must be greater than zero and less than {}",
                i32::MAX
            ))),
        }
    };
    while i < format.len() {
        let Some(pct) = format[i..].iter().position(|&c| c == b'%') else {
            out.extend_from_slice(&format[i..]);
            break;
        };
        out.extend_from_slice(&format[i..i + pct]);
        i += pct + 1;
        if at(i) == b'%' {
            out.push(b'%');
            i += 1;
            continue;
        }
        let mut spec = Spec { width: 0, precision: None, padding: b' ', align: Align::Right, always_sign: false };
        let mut arg = None;
        if !at(i).is_ascii_alphabetic() {
            arg = argnum(&mut i)?;
            loop {
                match at(i) {
                    c @ (b' ' | b'0') => spec.padding = c,
                    b'-' => spec.align = Align::Left,
                    b'+' => spec.always_sign = true,
                    b'\'' => {
                        if i + 1 < format.len() {
                            i += 1;
                            spec.padding = format[i];
                        } else {
                            return Err(Fatal::value_error("Missing padding character"));
                        }
                    }
                    _ => break,
                }
                i += 1;
            }
            let mut missing = false;
            if at(i) == b'*' {
                i += 1;
                let n = match argnum(&mut i)? {
                    Some(n) => n,
                    None => {
                        currarg += 1;
                        currarg - 1
                    }
                };
                match args.get(n) {
                    None => {
                        max_missing = max_missing.max(Some(n));
                        missing = true;
                    }
                    Some(Value::Int(w)) if (0..=i32::MAX as i64).contains(w) => spec.width = *w as usize,
                    Some(Value::Int(_)) => {
                        return Err(Fatal::value_error(format!("Width must be between 0 and {}", i32::MAX)))
                    }
                    Some(_) => return Err(Fatal::value_error("Width must be an integer")),
                }
            } else if at(i).is_ascii_digit() {
                spec.width = number(&mut i)
                    .ok_or_else(|| Fatal::value_error(format!("Width must be between 0 and {}", i32::MAX)))?;
            }
            if at(i) == b'.' {
                i += 1;
                if at(i) == b'*' {
                    i += 1;
                    let n = match argnum(&mut i)? {
                        Some(n) => n,
                        None => {
                            currarg += 1;
                            currarg - 1
                        }
                    };
                    match args.get(n) {
                        None => {
                            max_missing = max_missing.max(Some(n));
                            missing = true;
                        }
                        Some(Value::Int(-1)) => spec.precision = Some(usize::MAX),
                        Some(Value::Int(p)) if (0..=i32::MAX as i64).contains(p) => spec.precision = Some(*p as usize),
                        Some(Value::Int(_)) => {
                            return Err(Fatal::value_error(format!("Precision must be between -1 and {}", i32::MAX)))
                        }
                        Some(_) => return Err(Fatal::value_error("Precision must be an integer")),
                    }
                } else if at(i).is_ascii_digit() {
                    spec.precision = Some(number(&mut i).ok_or_else(|| {
                        Fatal::value_error(format!("Precision must be between 0 and {}", i32::MAX))
                    })?);
                } else {
                    spec.precision = Some(0);
                }
            }
            if missing {
                continue;
            }
        }
        if at(i) == b'l' {
            i += 1;
        }
        let n = match arg {
            Some(n) => n,
            None => {
                currarg += 1;
                currarg - 1
            }
        };
        let Some(v) = args.get(n) else {
            max_missing = max_missing.max(Some(n));
            // php-src moves on without consuming the specifier character
            continue;
        };
        let conv = at(i);
        if spec.precision == Some(usize::MAX) && !matches!(conv, b'g' | b'G' | b'h' | b'H') {
            return Err(Fatal::value_error("Precision -1 is only supported for %g, %G, %h and %H"));
        }
        match conv {
            b's' => {
                let s = match v {
                    Value::Array(_) => {
                        diag::warning("Array to string conversion");
                        b"Array".to_vec()
                    }
                    v => rt.stringify(v)?,
                };
                spec.append(&mut out, &s, false, true);
            }
            b'd' => {
                let n = v.to_int();
                // Integers are never padded with zeros on the right
                if spec.align == Align::Left && spec.padding == b'0' {
                    spec.padding = b' ';
                }
                let s = if n >= 0 && spec.always_sign { format!("+{n}") } else { n.to_string() };
                spec.append(&mut out, s.as_bytes(), n < 0, false);
            }
            b'u' => {
                if spec.align == Align::Left && spec.padding == b'0' {
                    spec.padding = b' ';
                }
                spec.append(&mut out, (v.to_int() as u64).to_string().as_bytes(), false, false);
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'h' | b'H' => {
                append_double(&mut out, fname, &spec, v.to_float(), conv);
            }
            b'c' => out.push(v.to_int() as u8),
            b'o' | b'x' | b'X' | b'b' => {
                let n = v.to_int() as u64;
                // php-src truncates these to "precision" characters of nothing: only padding remains
                let s = match conv {
                    _ if spec.precision.is_some() => String::new(),
                    b'o' => format!("{n:o}"),
                    b'x' => format!("{n:x}"),
                    b'X' => format!("{n:X}"),
                    _ => format!("{n:b}"),
                };
                spec.append(&mut out, s.as_bytes(), false, true);
            }
            b'%' => out.push(b'%'),
            0 if i >= format.len() => return Err(Fatal::value_error("Missing format specifier at end of string")),
            c => {
                return Err(Fatal::value_error(format!("Unknown format specifier \"{}\"", c as char)));
            }
        }
        i += 1;
    }
    if let Some(m) = max_missing {
        return Err(match nb_additional {
            None => Fatal::value_error(format!("The arguments array must contain {} items, {} given", m + 1, args.len())),
            Some(extra) => Fatal::new(
                "ArgumentCountError",
                format!("{} arguments are required, {} given", m + extra + 1, args.len() + extra),
            ),
        });
    }
    Ok(out)
}

fn append_double(out: &mut Vec<u8>, fname: &str, spec: &Spec, f: f64, conv: u8) {
    let precision = match spec.precision {
        None => FLOAT_PRECISION,
        Some(p) if p > MAX_FLOAT_PRECISION && p != usize::MAX => {
            diag::notice(format!(
                "{fname}(): Requested precision of {p} digits was truncated to PHP maximum of {MAX_FLOAT_PRECISION} digits"
            ));
            MAX_FLOAT_PRECISION
        }
        Some(p) => p,
    };
    // php-src passes these with their own length as the width: never padded
    if f.is_nan() {
        out.extend_from_slice(b"NaN");
        return;
    }
    if f.is_infinite() {
        out.extend_from_slice(if f < 0.0 { b"-INF" } else { b"INF" });
        return;
    }
    let neg = f.is_sign_negative() && f != 0.0 || (f == 0.0 && f.is_sign_negative());
    let body = match conv {
        b'e' | b'E' | b'f' | b'F' => {
            let digits = if matches!(conv, b'f' | b'F') {
                conv_fp_f(f.abs(), precision)
            } else {
                conv_fp_e(f.abs(), precision, conv as char)
            };
            if neg {
                format!("-{digits}")
            } else if spec.always_sign {
                format!("+{digits}")
            } else {
                digits
            }
        }
        _ => {
            let precision = if precision == 0 { 1 } else { precision };
            let exp = if matches!(conv, b'G' | b'H') { 'E' } else { 'e' };
            let s = if precision == usize::MAX {
                gcvt(f, None, exp)
            } else {
                gcvt(f, Some(precision), exp)
            };
            if !s.starts_with('-') && spec.always_sign { format!("+{s}") } else { s }
        }
    };
    spec.append(out, body.as_bytes(), neg, false);
}

/// php_conv_fp in 'F' mode for a non-negative number: correctly rounded fixed point
pub fn conv_fp_f(f: f64, precision: usize) -> String {
    format!("{f:.precision$}")
}

/// php_conv_fp in 'e' mode: `d.ddde+N`, the exponent without padding
fn conv_fp_e(f: f64, precision: usize, e: char) -> String {
    let s = format!("{f:.precision$e}");
    let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i64 = exp.parse().unwrap_or(0);
    format!("{mantissa}{e}{}{}", if exp < 0 { '-' } else { '+' }, exp.abs())
}

/// zend_gcvt: `ndigit` significant digits (None: the shortest round trip, as
/// with precision -1), in E-style when the exponent is out of range
pub fn gcvt(f: f64, ndigit: Option<usize>, exp_char: char) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f < 0.0 { "-INF".into() } else { "INF".into() };
    }
    let ndigit_i = ndigit.unwrap_or(17) as i64;
    let (digits, decpt) = if f == 0.0 {
        ("0".to_string(), 1)
    } else {
        let s = match ndigit {
            Some(n) => format!("{:.*e}", n.saturating_sub(1).min(400), f.abs()),
            None => format!("{:e}", f.abs()),
        };
        let (mantissa, exp) = s.split_once('e').unwrap();
        let digits = mantissa.replace('.', "");
        let digits = digits.trim_end_matches('0');
        let digits = if digits.is_empty() { "0" } else { digits };
        (digits.to_string(), exp.parse::<i64>().unwrap() + 1)
    };
    let mut out = String::new();
    if f.is_sign_negative() {
        out.push('-');
    }
    if (decpt >= 0 && decpt > ndigit_i) || decpt < -3 {
        let e = decpt - 1;
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        out.push(exp_char);
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&e.abs().to_string());
    } else if decpt <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-decpt) as usize));
        out.push_str(&digits);
    } else {
        let d = decpt as usize;
        if digits.len() <= d {
            out.push_str(&digits);
            out.push_str(&"0".repeat(d - digits.len()));
        } else {
            out.push_str(&digits[..d]);
            out.push('.');
            out.push_str(&digits[d..]);
        }
    }
    out
}
