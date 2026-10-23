//! PHP values and their conversion, comparison and arithmetic rules (PHP 8).
//! Cases that can be undefined behavior in the C implementation (integer overflow,
//! i64::MIN % -1, out-of-range shift amounts, ...) are all handled explicitly.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::rc::Rc;

use crate::array::PhpArray;
use crate::diag;
use crate::object::{self, Object};

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// PHP strings are arbitrary byte sequences. Refcounted like zend_string, so
    /// copying a value is O(1) and `.=` can append in place when unshared
    Str(Rc<Vec<u8>>),
    /// Arrays are values in PHP; Rc::make_mut gives copy-on-write
    Array(Rc<PhpArray>),
    /// Objects are handles: copies share the same instance
    Object(Rc<Object>),
    /// A PHP reference (`&$x`). Only ever stored in variable slots and array
    /// elements, never produced as the result of an expression, and never nested
    Ref(Rc<RefCell<Value>>),
}

/// Runs `f` on the value behind a reference. A reference that is being written
/// right now (only reachable through a reference cycle) reads as null rather
/// than panicking on the double borrow
pub fn with_deref<T>(r: &RefCell<Value>, f: impl FnOnce(&Value) -> T) -> T {
    match r.try_borrow() {
        Ok(v) => f(&v),
        Err(_) => f(&Value::Null),
    }
}

/// Arrays can contain themselves through references, so recursive comparisons
/// are cut off at this depth instead of overflowing the stack
const MAX_COMPARE_DEPTH: u32 = 256;

thread_local! {
    static COMPARE_DEPTH: Cell<u32> = const { Cell::new(0) };
    static TOO_DEEP: Cell<bool> = const { Cell::new(false) };
}

/// Runs a nested comparison step, or returns None when nesting is too deep.
/// That case is remembered so the caller can raise PHP's error for it
fn nested<T>(f: impl FnOnce() -> T) -> Option<T> {
    let depth = COMPARE_DEPTH.with(Cell::get);
    if depth >= MAX_COMPARE_DEPTH {
        TOO_DEEP.with(|t| t.set(true));
        return None;
    }
    COMPARE_DEPTH.with(|d| d.set(depth + 1));
    let out = f();
    COMPARE_DEPTH.with(|d| d.set(depth));
    Some(out)
}

/// Whether a comparison since the last call gave up on too deep a nesting
/// (a structure containing itself), which PHP reports as an Error
pub fn take_too_deep() -> bool {
    TOO_DEEP.with(|t| t.replace(false))
}

/// `==` on Value is PHP's `===`
impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Ref(r), _) => with_deref(r, |v| v == other),
            (_, Value::Ref(r)) => with_deref(r, |v| self == v),
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            (Value::Array(a), Value::Array(b)) => {
                Rc::ptr_eq(a, b)
                    || a.len() == b.len()
                        && nested(|| a.iter().zip(b.iter()).all(|((ka, va), (kb, vb))| ka == kb && va == vb))
                            .unwrap_or(false)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    pub fn to_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }

    pub fn into_value(self) -> Value {
        match self {
            Num::Int(i) => Value::Int(i),
            Num::Float(f) => Value::Float(f),
        }
    }
}

/// How a string was interpreted as a number
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Numeric {
    /// Entirely numeric (surrounding whitespace allowed): "12", " 1.5 "
    Full,
    /// Only a numeric prefix: "12abc"
    Leading,
    /// Not numeric: "abc", ""
    No,
}

/// Parses a numeric string following PHP 8 rules
pub fn parse_numeric(s: &[u8]) -> (Num, Numeric) {
    let is_ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let mut i = 0;
    while i < s.len() && is_ws(s[i]) {
        i += 1;
    }
    let start = i;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    let mut is_float = false;
    if i < s.len() && s[i] == b'.' {
        let mut j = i + 1;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if digits > 0 || j > i + 1 {
            digits += j - i - 1;
            is_float = true;
            i = j;
        }
    }
    if digits == 0 {
        return (Num::Int(0), Numeric::No);
    }
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        if j < s.len() && s[j].is_ascii_digit() {
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            is_float = true;
            i = j;
        }
    }
    // Only ASCII has been accepted so far, so from_utf8 cannot fail
    let text = std::str::from_utf8(&s[start..i]).unwrap();
    let num = if is_float {
        Num::Float(text.parse().unwrap_or(0.0))
    } else {
        // Too large for an integer: becomes a float
        text.parse::<i64>().map(Num::Int).unwrap_or_else(|_| Num::Float(text.parse().unwrap_or(0.0)))
    };
    let mut end = i;
    while end < s.len() && is_ws(s[end]) {
        end += 1;
    }
    let kind = if end == s.len() { Numeric::Full } else { Numeric::Leading };
    (num, kind)
}

pub fn float_fits_int(f: f64) -> bool {
    (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f)
}

/// Implicit float -> int conversion (operators, array keys, int parameters),
/// with PHP's diagnostics. `src` is the original value, for the message
pub fn float_to_int_implicit(f: f64, src: &Value) -> i64 {
    if !float_fits_int(f) {
        diag::warning(format!("The float {} is not representable as an int, cast occurred", format_float_repr(f)));
        return float_to_int(f);
    }
    if f.fract() != 0.0 {
        diag::deprecated(match src {
            Value::Str(s) => {
                format!("Implicit conversion from float-string \"{}\" to int loses precision", String::from_utf8_lossy(s))
            }
            _ => format!("Implicit conversion from float {} to int loses precision", format_float_repr(f)),
        });
    }
    f as i64
}

/// float -> int conversion. Like PHP 8, out-of-range values wrap modulo 2^64 and NaN/Inf become 0
pub fn float_to_int(f: f64) -> i64 {
    if !f.is_finite() {
        return 0;
    }
    if float_fits_int(f) {
        return f as i64;
    }
    let two64 = 18446744073709551616.0_f64;
    let mut m = f.trunc() % two64;
    if m < 0.0 {
        m += two64;
    }
    // m is in [0, 2^64), so it fits in u64
    (m as u64) as i64
}

/// Float to string as used by echo etc. (%G with precision=14)
pub fn format_float(f: f64) -> String {
    format_float_prec(f, 14)
}

/// Shortest round-trippable representation as used by var_dump etc. (serialize_precision=-1)
pub fn format_float_repr(f: f64) -> String {
    if !f.is_finite() || f == 0.0 {
        return format_float(f);
    }
    // Rust's `{:e}` prints the shortest round-trippable digits
    let sci = format!("{f:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits = mantissa.trim_start_matches('-').replace('.', "");
    render_g(f.is_sign_negative(), &digits, exp, 17)
}

fn format_float_prec(f: f64, prec: usize) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF".into() } else { "-INF".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let sci = format!("{:.*e}", prec - 1, f.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits = mantissa.replace('.', "");
    let digits = digits.trim_end_matches('0');
    render_g(f < 0.0, digits, exp, prec as i32)
}

/// Renders %G-style output from significant `digits` (most significant first) and a decimal exponent
fn render_g(neg: bool, digits: &str, exp: i32, prec: i32) -> String {
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if exp < -4 || exp >= prec {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        out.push_str(&format!("E{}{}", if exp < 0 { '-' } else { '+' }, exp.abs()));
    } else if exp < 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-exp - 1) as usize));
        out.push_str(digits);
    } else {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            out.push_str(digits);
            out.push_str(&"0".repeat(int_len - digits.len()));
        } else {
            out.push_str(&digits[..int_len]);
            out.push('.');
            out.push_str(&digits[int_len..]);
        }
    }
    out
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            // PHP names the class in type errors ("..., Exception given")
            Value::Object(o) => o.class.name,
            Value::Ref(r) => with_deref(r, Value::type_name),
        }
    }

    /// The value to store elsewhere when reading a slot: references are followed
    pub fn copy_out(&self) -> Value {
        match self {
            Value::Ref(r) => with_deref(r, Value::clone),
            v => v.clone(),
        }
    }

    pub fn str(s: impl Into<Vec<u8>>) -> Value {
        Value::Str(Rc::new(s.into()))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn to_bool(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !(s.is_empty() || s.as_slice() == b"0"),
            Value::Array(a) => !a.is_empty(),
            Value::Object(_) => true,
            Value::Ref(r) => with_deref(r, Value::to_bool),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Value::Null | Value::Bool(false) => Vec::new(),
            Value::Bool(true) => b"1".to_vec(),
            Value::Int(i) => i.to_string().into_bytes(),
            Value::Float(f) => format_float(*f).into_bytes(),
            Value::Str(s) => s.to_vec(),
            Value::Array(_) => b"Array".to_vec(),
            // Only Throwables are stringable so far; callers check that via interp::to_str
            Value::Object(o) if o.class.is_throwable => object::throwable_to_string(o),
            Value::Object(_) => Vec::new(),
            Value::Ref(r) => with_deref(r, Value::to_bytes),
        }
    }

    /// Numeric value without diagnostics. Non-numeric strings become 0
    pub fn to_num(&self) -> Num {
        match self {
            Value::Null | Value::Bool(false) => Num::Int(0),
            Value::Bool(true) => Num::Int(1),
            Value::Int(i) => Num::Int(*i),
            Value::Float(f) => Num::Float(*f),
            Value::Str(s) => parse_numeric(s).0,
            Value::Array(a) => Num::Int(!a.is_empty() as i64),
            Value::Object(_) => Num::Int(1),
            Value::Ref(r) => with_deref(r, Value::to_num),
        }
    }

    /// `(int)` without diagnostics. Floats wrap around; numeric strings saturate
    /// (`(int)"1e19"` is PHP_INT_MAX) because php-src converts them via strtol
    pub fn to_int(&self) -> i64 {
        match (self, self.to_num()) {
            (Value::Ref(r), _) => with_deref(r, Value::to_int),
            (_, Num::Int(i)) => i,
            // `as` saturates and maps NaN to 0
            (Value::Str(_), Num::Float(f)) => f as i64,
            (_, Num::Float(f)) => float_to_int(f),
        }
    }

    /// `(int)` as an explicit cast, which warns for unrepresentable floats
    pub fn cast_int(&self) -> i64 {
        if let Value::Float(f) = self
            && !float_fits_int(*f) {
                diag::warning(format!("The float {} is not representable as an int, cast occurred", format_float_repr(*f)));
            }
        self.to_int()
    }

    pub fn to_float(&self) -> f64 {
        self.to_num().to_f64()
    }

    pub fn strict_eq(&self, other: &Value) -> bool {
        self == other
    }

    pub fn loose_eq(&self, other: &Value) -> bool {
        loose_cmp(self, other) == Some(Ordering::Equal)
    }
}

fn cmp_num(a: Num, b: Num) -> Option<Ordering> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Some(x.cmp(&y)),
        _ => a.to_f64().partial_cmp(&b.to_f64()),
    }
}

fn cmp_str(a: &[u8], b: &[u8]) -> Option<Ordering> {
    let (na, ka) = parse_numeric(a);
    let (nb, kb) = parse_numeric(b);
    if ka == Numeric::Full && kb == Numeric::Full {
        return cmp_num(na, nb);
    }
    Some(a.cmp(b))
}

/// PHP 8 loose comparison (`==`, `<`, `<=>`, ...). None when NaN is involved
pub fn loose_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    use Value::*;
    match (a, b) {
        (Ref(r), _) => with_deref(r, |v| loose_cmp(v, b)),
        (_, Ref(r)) => with_deref(r, |v| loose_cmp(a, v)),
        (Str(x), Str(y)) => cmp_str(x, y),
        (Null, Str(s)) => cmp_str(b"", s),
        (Str(s), Null) => cmp_str(s, b""),
        (Bool(_) | Null, _) | (_, Bool(_) | Null) => Some(a.to_bool().cmp(&b.to_bool())),
        (Int(_) | Float(_), Str(s)) => {
            let (n, kind) = parse_numeric(s);
            if kind == Numeric::Full {
                cmp_num(a.to_num(), n)
            } else {
                Some(a.to_bytes().as_slice().cmp(s.as_slice()))
            }
        }
        (Str(_), Int(_) | Float(_)) => loose_cmp(b, a).map(Ordering::reverse),
        (Array(x), Array(y)) => nested(|| cmp_arrays(x, y)).flatten(),
        (Object(x), Object(y)) if Rc::ptr_eq(x, y) => Some(Ordering::Equal),
        // Objects of the same class compare by their properties; anything else is uncomparable
        (Object(x), Object(y)) if Rc::ptr_eq(&x.class, &y.class) && x.closure().is_none() => {
            let px: PhpArray = x.ordered_props().into_iter().collect();
            let py: PhpArray = y.ordered_props().into_iter().collect();
            nested(|| cmp_arrays(&px, &py)).flatten()
        }
        (Object(_), _) | (_, Object(_)) => None,
        (Array(_), _) => Some(Ordering::Greater),
        (_, Array(_)) => Some(Ordering::Less),
        _ => cmp_num(a.to_num(), b.to_num()),
    }
}

/// Arrays compare by size first, then element-wise by the left array's keys.
/// A key missing on the right makes them uncomparable (None)
fn cmp_arrays(a: &PhpArray, b: &PhpArray) -> Option<Ordering> {
    if a.len() != b.len() {
        return Some(a.len().cmp(&b.len()));
    }
    for (k, va) in a.iter() {
        let vb = b.get(k)?;
        match loose_cmp(va, vb)? {
            Ordering::Equal => {}
            other => return Some(other),
        }
    }
    Some(Ordering::Equal)
}

#[derive(Debug, Clone)]
pub enum ArithError {
    DivisionByZero(&'static str),
    Arithmetic(&'static str),
    Type(String),
}

/// An arithmetic operand. PHP 8 rejects arrays and non-numeric strings, and
/// warns for leading-numeric ones like "5 apples"
fn operand(v: &Value) -> Option<Num> {
    match v {
        Value::Ref(r) => with_deref(r, operand),
        Value::Array(_) | Value::Object(_) => None,
        Value::Str(s) => match parse_numeric(s) {
            (n, Numeric::Full) => Some(n),
            (n, Numeric::Leading) => {
                diag::warning("A non-numeric value encountered");
                Some(n)
            }
            (_, Numeric::No) => None,
        },
        _ => Some(v.to_num()),
    }
}

fn nums(op: &str, a: &Value, b: &Value) -> Result<(Num, Num), ArithError> {
    match (operand(a), operand(b)) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(ArithError::Type(format!(
            "Unsupported operand types: {} {op} {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn ints(op: &str, a: &Value, b: &Value) -> Result<(i64, i64), ArithError> {
    let (x, y) = nums(op, a, b)?;
    let to_int = |n: Num, src: &Value| match n {
        Num::Int(i) => i,
        Num::Float(f) => float_to_int_implicit(f, src),
    };
    Ok((to_int(x, a), to_int(y, b)))
}

/// `&`, `|` and `^`. Two strings combine byte-wise: & and ^ truncate to the
/// shorter one, | pads with the longer one
pub fn bitwise(op: &str, a: &Value, b: &Value) -> Result<Value, ArithError> {
    let f = |x: i64, y: i64| match op {
        "&" => x & y,
        "|" => x | y,
        _ => x ^ y,
    };
    if let (Value::Str(x), Value::Str(y)) = (a, b) {
        let (short, long) = if x.len() <= y.len() { (x, y) } else { (y, x) };
        let mut out: Vec<u8> = short.iter().zip(long.iter()).map(|(p, q)| f(*p as i64, *q as i64) as u8).collect();
        if op == "|" {
            out.extend_from_slice(&long[short.len()..]);
        }
        return Ok(Value::str(out));
    }
    let (x, y) = ints(op, a, b)?;
    Ok(Value::Int(f(x, y)))
}

pub fn add(a: &Value, b: &Value) -> Result<Value, ArithError> {
    if let (Value::Array(x), Value::Array(y)) = (a, b) {
        // Array union: keys already on the left win
        let mut out = x.clone();
        let target = Rc::make_mut(&mut out);
        for (k, v) in y.iter() {
            if !target.contains_key(k) {
                target.insert(k.clone(), v.clone());
            }
        }
        return Ok(Value::Array(out));
    }
    Ok(match nums("+", a, b)? {
        (Num::Int(x), Num::Int(y)) => x.checked_add(y).map(Value::Int).unwrap_or(Value::Float(x as f64 + y as f64)),
        (x, y) => Value::Float(x.to_f64() + y.to_f64()),
    })
}

pub fn sub(a: &Value, b: &Value) -> Result<Value, ArithError> {
    Ok(match nums("-", a, b)? {
        (Num::Int(x), Num::Int(y)) => x.checked_sub(y).map(Value::Int).unwrap_or(Value::Float(x as f64 - y as f64)),
        (x, y) => Value::Float(x.to_f64() - y.to_f64()),
    })
}

pub fn mul(a: &Value, b: &Value) -> Result<Value, ArithError> {
    Ok(match nums("*", a, b)? {
        (Num::Int(x), Num::Int(y)) => x.checked_mul(y).map(Value::Int).unwrap_or(Value::Float(x as f64 * y as f64)),
        (x, y) => Value::Float(x.to_f64() * y.to_f64()),
    })
}

pub fn div(a: &Value, b: &Value) -> Result<Value, ArithError> {
    let (x, y) = nums("/", a, b)?;
    if y.to_f64() == 0.0 {
        return Err(ArithError::DivisionByZero("Division by zero"));
    }
    if let (Num::Int(x), Num::Int(y)) = (x, y) {
        // i64::MIN / -1 does not fit in i64 (UB in C), so fall back to float
        if x.checked_rem(y) == Some(0)
            && let Some(q) = x.checked_div(y) {
                return Ok(Value::Int(q));
            }
    }
    Ok(Value::Float(x.to_f64() / y.to_f64()))
}

pub fn modulo(a: &Value, b: &Value) -> Result<Value, ArithError> {
    let (x, y) = ints("%", a, b)?;
    if y == 0 {
        return Err(ArithError::DivisionByZero("Modulo by zero"));
    }
    // i64::MIN % -1 is UB in C (crashes on many platforms). PHP returns 0
    Ok(Value::Int(x.checked_rem(y).unwrap_or(0)))
}

pub fn pow(a: &Value, b: &Value) -> Result<Value, ArithError> {
    Ok(match nums("**", a, b)? {
        (Num::Int(x), Num::Int(y)) if y >= 0 => {
            let checked = u32::try_from(y).ok().and_then(|e| x.checked_pow(e));
            match checked {
                Some(v) => Value::Int(v),
                None => Value::Float((x as f64).powf(y as f64)),
            }
        }
        (x, y) => Value::Float(x.to_f64().powf(y.to_f64())),
    })
}

/// `-$x` is compiled as `$x * -1` in PHP, which shows in its error messages
pub fn neg(a: &Value) -> Result<Value, ArithError> {
    Ok(match nums("*", a, &Value::Int(-1))?.0 {
        Num::Int(i) => i.checked_neg().map(Value::Int).unwrap_or(Value::Float(-(i as f64))),
        Num::Float(f) => Value::Float(-f),
    })
}

pub fn plus(a: &Value) -> Result<Value, ArithError> {
    Ok(nums("*", a, &Value::Int(1))?.0.into_value())
}

pub fn shift(a: &Value, b: &Value, left: bool) -> Result<Value, ArithError> {
    let (x, n) = ints(if left { "<<" } else { ">>" }, a, b)?;
    if n < 0 {
        return Err(ArithError::Arithmetic("Bit shift by negative number"));
    }
    // Shifting by >= the type width is UB in C. PHP yields 0 (-1 for right-shifting a negative)
    Ok(Value::Int(if n >= 64 {
        if !left && x < 0 { -1 } else { 0 }
    } else if left {
        x << n
    } else {
        x >> n
    }))
}
