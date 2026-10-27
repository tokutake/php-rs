//! Random numbers: mt_rand() and friends on php-src's Mt19937, which shuffle(),
//! str_shuffle() and array_rand() share, so seeded sequences match php-src;
//! random_int() and random_bytes() read the operating system's CSPRNG.

use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;

use super::{arg_int, arg_str, args, expect_array, Runtime};
use crate::array::{Key, PhpArray};
use crate::diag;
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "mt_srand", "srand", "mt_rand", "rand", "mt_getrandmax", "getrandmax", "random_int", "random_bytes", "lcg_value",
    "str_shuffle", "array_rand", "uniqid",
];

const MT_RAND_MT19937: i64 = 0;
const MT_RAND_PHP: i64 = 1;
const MT_RAND_MAX: i64 = 0x7fff_ffff;

pub fn constant(name: &str) -> Option<Value> {
    match name {
        "MT_RAND_MT19937" => Some(Value::Int(MT_RAND_MT19937)),
        "MT_RAND_PHP" => {
            diag::deprecated("Constant MT_RAND_PHP is deprecated since 8.3, as it uses a biased non-standard variant of Mt19937");
            Some(Value::Int(MT_RAND_PHP))
        }
        _ => None,
    }
}

const N: usize = 624;
const M: usize = 397;

/// php-src's Mt19937 state; `php_variant` is MT_RAND_PHP's buggy twist
struct Mt {
    state: [u32; N],
    count: usize,
    php_variant: bool,
}

impl Mt {
    fn seeded(seed: u32, php_variant: bool) -> Mt {
        let mut state = [0u32; N];
        state[0] = seed;
        for i in 1..N {
            let prev = state[i - 1];
            state[i] = 1812433253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        let mut mt = Mt { state, count: N, php_variant };
        mt.reload();
        mt
    }

    fn reload(&mut self) {
        let php = self.php_variant;
        let twist = |m: u32, u: u32, v: u32| {
            let mix = (u & 0x8000_0000) | (v & 0x7fff_ffff);
            let low = if php { u & 1 } else { v & 1 };
            m ^ (mix >> 1) ^ (0u32.wrapping_sub(low) & 0x9908_b0df)
        };
        let s = &mut self.state;
        for i in 0..N - M {
            s[i] = twist(s[i + M], s[i], s[i + 1]);
        }
        for i in N - M..N - 1 {
            s[i] = twist(s[i + M - N], s[i], s[i + 1]);
        }
        s[N - 1] = twist(s[M - 1], s[N - 1], s[0]);
        self.count = 0;
    }

    fn next(&mut self) -> u32 {
        if self.count >= N {
            self.reload();
        }
        let mut s = self.state[self.count];
        self.count += 1;
        s ^= s >> 11;
        s ^= (s << 7) & 0x9d2c_5680;
        s ^= (s << 15) & 0xefc6_0000;
        s ^ (s >> 18)
    }

    /// A uniform number in [0, umax], rejecting results that would bias it (php_random_range32)
    fn range32(&mut self, umax: u32) -> u32 {
        let mut r = self.next();
        if umax == u32::MAX {
            return r;
        }
        let n = umax + 1;
        if n & (n - 1) == 0 {
            return r & (n - 1);
        }
        let limit = u32::MAX - (u32::MAX % n) - 1;
        while r > limit {
            r = self.next();
        }
        r % n
    }

    /// php_random_range64: two outputs, little end first
    fn range64(&mut self, umax: u64) -> u64 {
        let mut next64 = || self.next() as u64 | (self.next() as u64) << 32;
        let mut r = next64();
        if umax == u64::MAX {
            return r;
        }
        let n = umax + 1;
        if n & (n - 1) == 0 {
            return r & (n - 1);
        }
        let limit = u64::MAX - (u64::MAX % n) - 1;
        while r > limit {
            r = next64();
        }
        r % n
    }

    fn range(&mut self, min: i64, max: i64) -> i64 {
        let umax = (max as u64).wrapping_sub(min as u64);
        if umax > u32::MAX as u64 {
            return self.range64(umax).wrapping_add(min as u64) as i64;
        }
        (self.range32(umax as u32) as u64).wrapping_add(min as u64) as i64
    }

    /// php_mt_rand_common: MT_RAND_PHP scales a 31-bit number instead
    fn common(&mut self, min: i64, max: i64) -> i64 {
        if !self.php_variant {
            return self.range(min, max);
        }
        let r = (self.next() >> 1) as f64;
        let offset = ((max as f64 - min as f64 + 1.0) * (r / (MT_RAND_MAX as f64 + 1.0))) as u64;
        offset.wrapping_add(min as u64) as i64
    }
}

thread_local! {
    /// The default engine, seeded from the CSPRNG on first use
    static MT: RefCell<Option<Mt>> = const { RefCell::new(None) };
    static LCG: RefCell<Option<(i32, i32)>> = const { RefCell::new(None) };
}

fn with_mt<T>(f: impl FnOnce(&mut Mt) -> T) -> T {
    MT.with(|mt| f(mt.borrow_mut().get_or_insert_with(|| Mt::seeded(csprng_u32(), false))))
}

/// Fills `buf` from the operating system's CSPRNG
pub fn os_random(buf: &mut [u8]) -> bool {
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).is_ok()
}

fn csprng_u32() -> u32 {
    let mut b = [0u8; 4];
    if !os_random(&mut b) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        return t.subsec_nanos() ^ t.as_secs() as u32 ^ std::process::id();
    }
    u32::from_le_bytes(b)
}

fn random_exception() -> Fatal {
    Fatal::new("Random\\RandomException", "Failed to open /dev/urandom")
}

/// A uniform number in [min, max] from the CSPRNG (php_random_int)
fn csprng_range(min: i64, max: i64) -> EResult<i64> {
    let umax = (max as u64).wrapping_sub(min as u64);
    let next = || {
        let mut b = [0u8; 8];
        if os_random(&mut b) { Ok(u64::from_le_bytes(b)) } else { Err(random_exception()) }
    };
    let mut r = next()?;
    if umax == u64::MAX {
        return Ok(r as i64);
    }
    let n = umax + 1;
    if n & (n - 1) == 0 {
        r &= n - 1;
    } else {
        let limit = u64::MAX - (u64::MAX % n) - 1;
        while r > limit {
            r = next()?;
        }
        r %= n;
    }
    Ok(r.wrapping_add(min as u64) as i64)
}

/// php_combined_lcg: two LCGs, seeded from the time and the process id
fn combined_lcg() -> f64 {
    LCG.with(|lcg| {
        let mut lcg = lcg.borrow_mut();
        let (s1, s2) = lcg.get_or_insert_with(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            ((t.as_secs() as i32) ^ ((t.subsec_micros() as i32) << 11), std::process::id() as i32 ^ (t.subsec_micros() as i32))
        });
        // Schrage's method, as php-src's MODMULT
        let modmult = |a: i32, b: i32, c: i32, m: i32, s: &mut i32| {
            let q = *s / a;
            *s = b.wrapping_mul(*s - a * q).wrapping_sub(c.wrapping_mul(q));
            if *s < 0 {
                *s += m;
            }
        };
        modmult(53668, 40014, 12211, 2147483563, s1);
        modmult(52774, 40692, 3791, 2147483399, s2);
        let mut z = *s1 - *s2;
        if z < 1 {
            z += 2147483562;
        }
        z as f64 * 4.656613e-10
    })
}

/// shuffle(): the values in a random order, renumbered
pub fn shuffle(arr: &mut Rc<PhpArray>) {
    let mut values: Vec<Value> = arr.values().cloned().collect();
    with_mt(|mt| {
        for left in (1..values.len()).rev() {
            let j = mt.range(0, left as i64) as usize;
            values.swap(left, j);
        }
    });
    *arr = Rc::new(PhpArray::from_values(values));
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "mt_srand" | "srand" => {
            let [seed, mode] = args::<0, 2>(name, vals)?;
            let seed = match seed {
                None | Some(Value::Null) => None,
                Some(s) => Some(arg_int(lname, 1, "seed", &s)?),
            };
            let mode = match mode {
                Some(m) => arg_int(lname, 2, "mode", &m)?,
                None => MT_RAND_MT19937,
            };
            let php_variant = mode == MT_RAND_PHP;
            if php_variant {
                diag::deprecated("The MT_RAND_PHP variant of Mt19937 is deprecated");
            }
            let seed = seed.map_or_else(csprng_u32, |s| s as u32);
            MT.with(|mt| *mt.borrow_mut() = Some(Mt::seeded(seed, php_variant)));
            Ok(Value::Null)
        }
        "mt_rand" | "rand" => {
            if vals.is_empty() {
                return Ok(Value::Int((with_mt(Mt::next) >> 1) as i64));
            }
            let [min, max] = args::<2, 2>(name, vals)?;
            let min = arg_int(lname, 1, "min", &min.unwrap())?;
            let max = arg_int(lname, 2, "max", &max.unwrap())?;
            if max < min {
                if lname == "mt_rand" {
                    return Err(Fatal::value_error(
                        "mt_rand(): Argument #2 ($max) must be greater than or equal to argument #1 ($min)",
                    ));
                }
                return Ok(Value::Int(with_mt(|mt| mt.common(max, min))));
            }
            Ok(Value::Int(with_mt(|mt| mt.common(min, max))))
        }
        "mt_getrandmax" | "getrandmax" => {
            args::<0, 0>(name, vals)?;
            Ok(Value::Int(MT_RAND_MAX))
        }
        "random_int" => {
            let [min, max] = args::<2, 2>(name, vals)?;
            let min = arg_int(lname, 1, "min", &min.unwrap())?;
            let max = arg_int(lname, 2, "max", &max.unwrap())?;
            if min > max {
                return Err(Fatal::value_error(
                    "random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)",
                ));
            }
            Ok(Value::Int(csprng_range(min, max)?))
        }
        "random_bytes" => {
            let [len] = args::<1, 1>(name, vals)?;
            let len = arg_int(lname, 1, "length", &len.unwrap())?;
            if len < 1 {
                return Err(Fatal::value_error("random_bytes(): Argument #1 ($length) must be greater than 0"));
            }
            // A zend_string of that length would not fit in php-src's default memory limit
            if len as u64 > crate::stream::READ_LIMIT as u64 {
                return Err(Fatal::compile(format!(
                    "Allowed memory size of {} bytes exhausted (tried to allocate {} bytes)",
                    crate::stream::READ_LIMIT,
                    (len as u64).saturating_add(32) & !7
                )));
            }
            let mut buf = vec![0u8; len as usize];
            if !os_random(&mut buf) {
                return Err(random_exception());
            }
            Ok(Value::str(buf))
        }
        "lcg_value" => {
            args::<0, 0>(name, vals)?;
            diag::deprecated("Function lcg_value() is deprecated since 8.4, use \\Random\\Randomizer::getFloat() instead");
            Ok(Value::Float(combined_lcg()))
        }
        "str_shuffle" => {
            let [s] = args::<1, 1>(name, vals)?;
            let mut s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            with_mt(|mt| {
                for left in (1..s.len()).rev() {
                    let j = mt.range(0, left as i64) as usize;
                    s.swap(left, j);
                }
            });
            Ok(Value::str(s))
        }
        "array_rand" => {
            let [arr, num] = args::<1, 2>(name, vals)?;
            let arr = expect_array(lname, 1, "array", &arr.unwrap())?;
            let num = match num {
                Some(n) => arg_int(lname, 2, "num", &n)?,
                None => 1,
            };
            let avail = arr.len() as i64;
            if avail == 0 {
                return Err(Fatal::value_error("array_rand(): Argument #1 ($array) must not be empty"));
            }
            if num == 1 {
                let i = with_mt(|mt| mt.range(0, avail - 1));
                return Ok(arr.keys().nth(i as usize).map_or(Value::Null, Key::to_value));
            }
            if num <= 0 || num > avail {
                return Err(Fatal::value_error(
                    "array_rand(): Argument #2 ($num) must be between 1 and the number of elements in argument #1 ($array)",
                ));
            }
            // Picks the smaller of the chosen and the left-out positions
            let negative = num > avail >> 1;
            let mut want = if negative { avail - num } else { num };
            let mut picked = vec![false; avail as usize];
            with_mt(|mt| {
                while want > 0 {
                    let i = mt.range(0, avail - 1) as usize;
                    if !picked[i] {
                        picked[i] = true;
                        want -= 1;
                    }
                }
            });
            let keys = arr.keys().zip(picked).filter(|(_, p)| *p != negative).map(|(k, _)| k.to_value());
            Ok(super::list(keys))
        }
        "uniqid" => {
            let [prefix, more] = args::<0, 2>(name, vals)?;
            let mut out = match prefix {
                Some(p) => arg_str(rt, lname, 1, "prefix", &p)?,
                None => Vec::new(),
            };
            let more = match more {
                Some(m) => super::arg_bool(lname, 2, "more_entropy", &m)?,
                None => false,
            };
            // php-src waits for the clock to move on, so ids from one process differ
            if !more {
                std::thread::sleep(std::time::Duration::from_micros(1));
            }
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            out.extend(format!("{:08x}{:05x}", t.as_secs(), t.subsec_micros()).bytes());
            if more {
                out.extend(format!("{:.8}", combined_lcg() * 10.0).bytes());
            }
            Ok(Value::str(out))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
