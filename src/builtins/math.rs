//! Math functions.

use std::cmp::Ordering;

use super::{arg_int, args, expect_array, Runtime};
use crate::diag;
use crate::interp::{EResult, Fatal};
use crate::value::{self, loose_cmp, Num, Value};

pub const NAMES: &[&str] = &[
    "abs", "ceil", "floor", "round", "sqrt", "pow", "exp", "expm1", "log", "log10", "log1p", "sin", "cos",
    "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh", "pi", "fmod", "fdiv",
    "hypot", "deg2rad", "rad2deg", "is_nan", "is_finite", "is_infinite", "max", "min", "array_product", "decbin",
    "bindec", "decoct", "octdec", "dechex", "hexdec", "base_convert", "number_format",
];

pub const PHP_ROUND_HALF_UP: i64 = 1;
pub const PHP_ROUND_HALF_DOWN: i64 = 2;
pub const PHP_ROUND_HALF_EVEN: i64 = 3;
pub const PHP_ROUND_HALF_ODD: i64 = 4;
const ROUND_TOWARD_ZERO: i64 = 5;
const ROUND_AWAY_FROM_ZERO: i64 = 6;
const ROUND_FLOOR: i64 = 7;
const ROUND_CEILING: i64 = 8;

pub fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "PHP_ROUND_HALF_UP" => Value::Int(PHP_ROUND_HALF_UP),
        "PHP_ROUND_HALF_DOWN" => Value::Int(PHP_ROUND_HALF_DOWN),
        "PHP_ROUND_HALF_EVEN" => Value::Int(PHP_ROUND_HALF_EVEN),
        "PHP_ROUND_HALF_ODD" => Value::Int(PHP_ROUND_HALF_ODD),
        "M_LOG2E" => Value::Float(std::f64::consts::LOG2_E),
        "M_LOG10E" => Value::Float(std::f64::consts::LOG10_E),
        "M_LN2" => Value::Float(std::f64::consts::LN_2),
        "M_LN10" => Value::Float(std::f64::consts::LN_10),
        "M_PI_2" => Value::Float(std::f64::consts::FRAC_PI_2),
        "M_PI_4" => Value::Float(std::f64::consts::FRAC_PI_4),
        "M_1_PI" => Value::Float(std::f64::consts::FRAC_1_PI),
        "M_2_PI" => Value::Float(std::f64::consts::FRAC_2_PI),
        "M_2_SQRTPI" => Value::Float(std::f64::consts::FRAC_2_SQRT_PI),
        "M_SQRT1_2" => Value::Float(std::f64::consts::FRAC_1_SQRT_2),
        "PHP_FLOAT_EPSILON" => Value::Float(f64::EPSILON),
        _ => return None,
    })
}

/// A `int|float` parameter
fn arg_num(fname: &str, n: usize, param: &str, v: &Value) -> EResult<Num> {
    match v {
        Value::Int(i) => Ok(Num::Int(*i)),
        Value::Float(f) => Ok(Num::Float(*f)),
        Value::Ref(r) => value::with_deref(r, |inner| arg_num(fname, n, param, inner)),
        Value::Str(s) if crate::interp::strict_types() => Err(super::param_type_error(fname, n, param, "int|float", &Value::Str(s.clone()))),
        Value::Str(s) => match value::parse_numeric(s) {
            (num, value::Numeric::Full) => Ok(num),
            _ => Err(super::param_type_error(fname, n, param, "int|float", v)),
        },
        Value::Bool(b) if !crate::interp::strict_types() => Ok(Num::Int(*b as i64)),
        Value::Null if !crate::interp::strict_types() => {
            super::null_param(fname, n, param, "int|float");
            Ok(Num::Int(0))
        }
        other => Err(super::param_type_error(fname, n, param, "int|float", other)),
    }
}

fn arg_float(fname: &str, n: usize, param: &str, v: &Value) -> EResult<f64> {
    arg_num(fname, n, param, v).map(Num::to_f64)
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    let unary = |f: fn(f64) -> f64, vals: Vec<Value>| -> EResult<Value> {
        let [x] = args::<1, 1>(name, vals)?;
        Ok(Value::Float(f(arg_float(lname, 1, "num", &x.unwrap())?)))
    };
    match lname {
        "abs" => {
            let [x] = args::<1, 1>(name, vals)?;
            Ok(match arg_num(lname, 1, "num", &x.unwrap())? {
                Num::Int(i64::MIN) => Value::Float(-(i64::MIN as f64)),
                Num::Int(i) => Value::Int(i.abs()),
                Num::Float(f) => Value::Float(f.abs()),
            })
        }
        "ceil" | "floor" => {
            let [x] = args::<1, 1>(name, vals)?;
            Ok(Value::Float(match arg_num(lname, 1, "num", &x.unwrap())? {
                Num::Int(i) => i as f64,
                Num::Float(f) if lname == "ceil" => f.ceil(),
                Num::Float(f) => f.floor(),
            }))
        }
        "round" => {
            let [x, places, mode] = args::<1, 3>(name, vals)?;
            let x = arg_num(lname, 1, "num", &x.unwrap())?;
            let places = match places {
                Some(p) => arg_int(lname, 2, "precision", &p)?,
                None => 0,
            };
            let mode = match mode {
                Some(m) => arg_int(lname, 3, "mode", &m)?,
                None => PHP_ROUND_HALF_UP,
            };
            // 5-8 are the RoundingMode cases without a PHP_ROUND_ constant
            if !(PHP_ROUND_HALF_UP..=ROUND_CEILING).contains(&mode) {
                return Err(Fatal::value_error(
                    "round(): Argument #3 ($mode) must be a valid rounding mode (RoundingMode::*)",
                ));
            }
            let places = places.clamp(i32::MIN as i64 + 1, i32::MAX as i64) as i32;
            Ok(Value::Float(match x {
                Num::Int(i) if places >= 0 => i as f64,
                Num::Int(i) => php_round(i as f64, places, mode),
                Num::Float(f) => php_round(f, places, mode),
            }))
        }
        "sqrt" => unary(f64::sqrt, vals),
        "exp" => unary(f64::exp, vals),
        "expm1" => unary(f64::exp_m1, vals),
        "log10" => unary(f64::log10, vals),
        "log1p" => unary(f64::ln_1p, vals),
        "sin" => unary(f64::sin, vals),
        "cos" => unary(f64::cos, vals),
        "tan" => unary(f64::tan, vals),
        "asin" => unary(f64::asin, vals),
        "acos" => unary(f64::acos, vals),
        "atan" => unary(f64::atan, vals),
        "sinh" => unary(f64::sinh, vals),
        "cosh" => unary(f64::cosh, vals),
        "tanh" => unary(f64::tanh, vals),
        "asinh" => unary(f64::asinh, vals),
        "acosh" => unary(f64::acosh, vals),
        "atanh" => unary(f64::atanh, vals),
        "deg2rad" => unary(f64::to_radians, vals),
        "rad2deg" => unary(f64::to_degrees, vals),
        "is_nan" | "is_finite" | "is_infinite" => {
            let [x] = args::<1, 1>(name, vals)?;
            let f = arg_float(lname, 1, "num", &x.unwrap())?;
            Ok(Value::Bool(match lname {
                "is_nan" => f.is_nan(),
                "is_finite" => f.is_finite(),
                _ => f.is_infinite(),
            }))
        }
        "log" => {
            let [x, base] = args::<1, 2>(name, vals)?;
            let x = arg_float(lname, 1, "num", &x.unwrap())?;
            Ok(Value::Float(match base {
                None => x.ln(),
                Some(b) => {
                    let b = arg_float(lname, 2, "base", &b)?;
                    if b <= 0.0 {
                        return Err(Fatal::value_error("log(): Argument #2 ($base) must be greater than 0"));
                    }
                    if b == 1.0 {
                        f64::NAN
                    } else if b == 2.0 {
                        x.log2()
                    } else if b == 10.0 {
                        x.log10()
                    } else {
                        x.ln() / b.ln()
                    }
                }
            }))
        }
        "pi" => {
            args::<0, 0>(name, vals)?;
            Ok(Value::Float(std::f64::consts::PI))
        }
        "pow" => {
            let [a, b] = args::<2, 2>(name, vals)?;
            value::pow(&a.unwrap(), &b.unwrap()).map_err(Fatal::from)
        }
        "fmod" | "atan2" | "hypot" | "fdiv" => {
            let [a, b] = args::<2, 2>(name, vals)?;
            let (p1, p2) = match lname {
                "fmod" | "fdiv" => ("num1", "num2"),
                "atan2" => ("y", "x"),
                _ => ("x", "y"),
            };
            let a = arg_float(lname, 1, p1, &a.unwrap())?;
            let b = arg_float(lname, 2, p2, &b.unwrap())?;
            Ok(Value::Float(match lname {
                "fmod" => a % b,
                "atan2" => a.atan2(b),
                "hypot" => a.hypot(b),
                _ => a / b,
            }))
        }
        "max" | "min" => {
            if vals.is_empty() {
                return Err(super::arg_count(lname, 1, 0));
            }
            let candidates: Vec<Value> = if vals.len() == 1 {
                let arr = match &vals[0] {
                    Value::Array(a) => a.clone(),
                    other => {
                        return Err(Fatal::type_error(format!(
                            "{lname}(): Argument #1 ($value) must be of type array, {} given",
                            other.value_name()
                        )))
                    }
                };
                if arr.is_empty() {
                    return Err(Fatal::value_error(format!(
                        "{lname}(): Argument #1 ($value) must contain at least one element"
                    )));
                }
                arr.values().map(Value::copy_out).collect()
            } else {
                vals
            };
            let mut it = candidates.into_iter();
            let mut best = it.next().unwrap();
            let want = if lname == "max" { Ordering::Greater } else { Ordering::Less };
            for v in it {
                // Uncomparable pairs count as "greater", as zend_compare reports 1 for them
                let ord = loose_cmp(&v, &best).unwrap_or(Ordering::Greater);
                if ord == want {
                    best = v;
                }
            }
            Ok(best)
        }
        "array_product" => {
            let [a] = args::<1, 1>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &a.unwrap())?;
            let mut product = Value::Int(1);
            for v in arr.values() {
                match value::mul(&product, v) {
                    Ok(p) => product = p,
                    Err(_) => diag::warning(format!(
                        "array_product(): Multiplication is not supported on type {}",
                        v.type_name()
                    )),
                }
            }
            Ok(product)
        }
        "decbin" | "decoct" | "dechex" => {
            let [x] = args::<1, 1>(name, vals)?;
            let x = arg_int(lname, 1, "num", &x.unwrap())? as u64;
            Ok(Value::str(match lname {
                "decbin" => format!("{x:b}"),
                "decoct" => format!("{x:o}"),
                _ => format!("{x:x}"),
            }))
        }
        "bindec" | "octdec" | "hexdec" => {
            let [s] = args::<1, 1>(name, vals)?;
            let s = super::arg_str(rt, lname, 1, "binary_string", &s.unwrap())?;
            let base = match lname {
                "bindec" => 2,
                "octdec" => 8,
                _ => 16,
            };
            Ok(basetodec(&s, base))
        }
        "base_convert" => {
            let [s, from, to] = args::<3, 3>(name, vals)?;
            let s = super::arg_str(rt, lname, 1, "num", &s.unwrap())?;
            let from = arg_int(lname, 2, "from_base", &from.unwrap())?;
            let to = arg_int(lname, 3, "to_base", &to.unwrap())?;
            if !(2..=36).contains(&from) {
                return Err(Fatal::value_error("base_convert(): Argument #2 ($from_base) must be between 2 and 36 (inclusive)"));
            }
            if !(2..=36).contains(&to) {
                return Err(Fatal::value_error("base_convert(): Argument #3 ($to_base) must be between 2 and 36 (inclusive)"));
            }
            let n = basetodec(&s, from as u32);
            Ok(Value::str(match n {
                Value::Int(i) => to_base(i as u64, to as u32),
                // Beyond 64 bits php-src converts the float digit by digit
                Value::Float(f) => float_to_base(f, to as u32),
                _ => unreachable!("basetodec gives numbers"),
            }))
        }
        "number_format" => {
            let [x, dec, point, sep] = args::<1, 4>(name, vals)?;
            let x = arg_float(lname, 1, "num", &x.unwrap())?;
            let dec = match dec {
                Some(d) => arg_int(lname, 2, "decimals", &d)?,
                None => 0,
            };
            let point = match point {
                Some(Value::Null) | None => b".".to_vec(),
                Some(p) => super::arg_str(rt, lname, 3, "decimal_separator", &p)?,
            };
            let sep = match sep {
                Some(Value::Null) | None => b",".to_vec(),
                Some(s) => super::arg_str(rt, lname, 4, "thousands_separator", &s)?,
            };
            Ok(Value::str(number_format(x, dec.clamp(i32::MIN as i64 + 1, i32::MAX as i64) as i32, &point, &sep)))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}

/// bindec() and friends: invalid digits are skipped (with a deprecation), and
/// numbers past PHP_INT_MAX continue as floats
fn basetodec(s: &[u8], base: u32) -> Value {
    let mut num: u64 = 0;
    let mut fnum = 0.0;
    let mut is_float = false;
    let mut invalid = false;
    let cutoff = i64::MAX as u64 / base as u64;
    let cutlim = i64::MAX as u64 % base as u64;
    for (i, &c) in s.iter().enumerate() {
        // An 0x / 0o / 0b prefix matching the base is allowed
        if i == 1 && s[0] == b'0' {
            let prefix = match base {
                16 => b'x',
                8 => b'o',
                2 => b'b',
                _ => 0,
            };
            if c.to_ascii_lowercase() == prefix {
                continue;
            }
        }
        let Some(d) = (c as char).to_digit(base) else {
            invalid = true;
            continue;
        };
        if is_float {
            fnum = fnum * base as f64 + d as f64;
        } else if num < cutoff || (num == cutoff && d as u64 <= cutlim) {
            num = num * base as u64 + d as u64;
        } else {
            fnum = num as f64 * base as f64 + d as f64;
            is_float = true;
        }
    }
    if invalid {
        diag::deprecated("Invalid characters passed for attempted conversion, these have been ignored");
    }
    if is_float { Value::Float(fnum) } else { Value::Int(num as i64) }
}

fn to_base(mut n: u64, base: u32) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut digits = Vec::new();
    while n > 0 {
        digits.push(std::char::from_digit((n % base as u64) as u32, base).unwrap());
        n /= base as u64;
    }
    digits.iter().rev().collect()
}

fn float_to_base(f: f64, base: u32) -> String {
    if !f.is_finite() {
        diag::warning("Number too large");
        return String::new();
    }
    let mut f = f.abs().floor();
    let mut digits = Vec::new();
    loop {
        let d = (f % base as f64) as u32;
        digits.push(std::char::from_digit(d, base).unwrap());
        f = (f / base as f64).floor();
        if f < 1.0 {
            break;
        }
    }
    digits.iter().rev().collect()
}

fn intpow10(power: i32) -> f64 {
    if (0..=22).contains(&power) { 10f64.powi(power) } else { 10f64.powf(power as f64) }
}

/// php-src's _php_math_round (PHP 8.4+)
pub fn php_round(value: f64, places: i32, mode: i64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    if places == 0 && value == value.trunc() {
        return value;
    }
    let exponent = intpow10(places.abs());
    let scaled = if places > 0 { value * exponent } else { value / exponent };
    let (mut tmp, tmp2) = if value >= 0.0 {
        let t = scaled.floor();
        (t, t + 1.0)
    } else {
        let t = scaled.ceil();
        (t, t - 1.0)
    };
    // The integral part may be one off from floating point error: 0.285 * 1e10
    if (if places > 0 { tmp2 / exponent } else { tmp2 * exponent }) == value {
        tmp = tmp2;
    }
    if tmp.abs() >= 1e16 {
        return value;
    }
    let edge = |integral: f64| {
        let e = integral + 0.5f64.copysign(integral);
        if places > 0 { (e / exponent).abs() } else { (e * exponent).abs() }
    };
    let abs = value.abs();
    let up = |i: f64| i + 1f64.copysign(i);
    let zero_edge = |integral: f64| if places > 0 { (integral / exponent).abs() } else { (integral * exponent).abs() };
    tmp = match mode {
        ROUND_TOWARD_ZERO => tmp,
        ROUND_AWAY_FROM_ZERO if abs > zero_edge(tmp) => up(tmp),
        ROUND_FLOOR if value < 0.0 && abs > zero_edge(tmp) => tmp - 1.0,
        ROUND_CEILING if value > 0.0 && abs > zero_edge(tmp) => tmp + 1.0,
        PHP_ROUND_HALF_UP if abs >= edge(tmp) => up(tmp),
        PHP_ROUND_HALF_DOWN if abs > edge(tmp) => up(tmp),
        PHP_ROUND_HALF_EVEN | PHP_ROUND_HALF_ODD => {
            let e = edge(tmp);
            let even = tmp % 2.0 == 0.0;
            if abs > e || (abs == e && even == (mode == PHP_ROUND_HALF_ODD)) { up(tmp) } else { tmp }
        }
        _ => tmp,
    };
    if places.abs() < 23 {
        if places > 0 { tmp / exponent } else { tmp * exponent }
    } else {
        format!("{tmp:15}e{}", -places).trim().parse().unwrap_or(value)
    }
}

/// php-src's _php_math_number_format_ex
pub fn number_format(d: f64, dec: i32, point: &[u8], sep: &[u8]) -> Vec<u8> {
    let mut d = d;
    let mut negative = d < 0.0;
    if negative {
        d = -d;
    }
    d = php_round(d, dec, PHP_ROUND_HALF_UP);
    let dec = dec.max(0) as usize;
    let tmp = super::format::conv_fp_f(d, dec);
    if !tmp.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return tmp.into_bytes();
    }
    if negative && d == 0.0 {
        negative = false;
    }
    let (int_part, frac) = match tmp.split_once('.') {
        Some((i, f)) if dec > 0 => (i, f),
        _ => (tmp.as_str(), ""),
    };
    let mut out = Vec::new();
    if negative {
        out.push(b'-');
    }
    let digits = int_part.as_bytes();
    for (i, &c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.extend_from_slice(sep);
        }
        out.push(c);
    }
    if dec > 0 {
        out.extend_from_slice(point);
        out.extend_from_slice(frac.as_bytes());
        out.extend(std::iter::repeat_n(b'0', dec.saturating_sub(frac.len())));
    }
    out
}
