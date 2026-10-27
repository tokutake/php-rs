//! password_hash(), password_verify() and crypt() with bcrypt ($2y$): the
//! eksblowfish key schedule over Blowfish, whose initial state (the hex
//! digits of pi) is in blowfish_pi

use std::cell::OnceCell;
use std::rc::Rc;

use super::{arg_str, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &["password_hash", "password_verify", "password_needs_rehash", "password_get_info", "password_algos", "crypt"];

/// PHP 8.4's default bcrypt cost
const DEFAULT_COST: i64 = 12;

pub fn constant(name: &str) -> Option<Value> {
    match name {
        "PASSWORD_DEFAULT" | "PASSWORD_BCRYPT" => Some(Value::str("2y")),
        "PASSWORD_BCRYPT_DEFAULT_COST" => Some(Value::Int(DEFAULT_COST)),
        "CRYPT_BLOWFISH" | "CRYPT_SALT_LENGTH" => Some(Value::Int(if name == "CRYPT_BLOWFISH" { 1 } else { 123 })),
        "CRYPT_STD_DES" | "CRYPT_EXT_DES" | "CRYPT_MD5" | "CRYPT_SHA256" | "CRYPT_SHA512" => Some(Value::Int(0)),
        _ => None,
    }
}

struct Blowfish {
    p: [u32; 18],
    s: [[u32; 256]; 4],
}

thread_local! {
    static INITIAL: OnceCell<Rc<Blowfish>> = const { OnceCell::new() };
}

fn initial() -> Rc<Blowfish> {
    INITIAL.with(|c| {
        c.get_or_init(|| {
            let digits = &super::blowfish_pi::PI_WORDS;
            let mut p = [0u32; 18];
            let mut s = [[0u32; 256]; 4];
            p.copy_from_slice(&digits[..18]);
            for (b, sbox) in s.iter_mut().enumerate() {
                sbox.copy_from_slice(&digits[18 + b * 256..18 + (b + 1) * 256]);
            }
            Rc::new(Blowfish { p, s })
        })
        .clone()
    })
}

impl Blowfish {
    #[inline(always)]
    fn f(&self, x: u32) -> u32 {
        let [a, b, c, d] = x.to_be_bytes();
        (self.s[0][a as usize].wrapping_add(self.s[1][b as usize]) ^ self.s[2][c as usize]).wrapping_add(self.s[3][d as usize])
    }

    #[inline(always)]
    fn encrypt(&self, l: &mut u32, r: &mut u32) {
        let (mut x, mut y) = (*l, *r);
        for i in 0..8 {
            x ^= self.p[2 * i];
            y ^= self.f(x);
            y ^= self.p[2 * i + 1];
            x ^= self.f(y);
        }
        *l = y ^ self.p[17];
        *r = x ^ self.p[16];
    }

    /// The next 32-bit word of a cyclic key stream
    fn stream_word(data: &[u8], pos: &mut usize) -> u32 {
        let mut w = 0u32;
        for _ in 0..4 {
            w = w << 8 | data[*pos] as u32;
            *pos = (*pos + 1) % data.len();
        }
        w
    }

    fn expand(&mut self, salt: &[u8], key: &[u8]) {
        let mut kp = 0;
        for i in 0..18 {
            self.p[i] ^= Self::stream_word(key, &mut kp);
        }
        let (mut l, mut r) = (0u32, 0u32);
        let mut sp = 0;
        for i in (0..18).step_by(2) {
            if !salt.is_empty() {
                l ^= Self::stream_word(salt, &mut sp);
                r ^= Self::stream_word(salt, &mut sp);
            }
            self.encrypt(&mut l, &mut r);
            self.p[i] = l;
            self.p[i + 1] = r;
        }
        for b in 0..4 {
            for i in (0..256).step_by(2) {
                if !salt.is_empty() {
                    l ^= Self::stream_word(salt, &mut sp);
                    r ^= Self::stream_word(salt, &mut sp);
                }
                self.encrypt(&mut l, &mut r);
                self.s[b][i] = l;
                self.s[b][i + 1] = r;
            }
        }
    }
}

const BCRYPT_ALPHABET: &[u8; 64] = b"./ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

fn bcrypt_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in data.chunks(3) {
        let c1 = chunk[0];
        out.push(BCRYPT_ALPHABET[(c1 >> 2) as usize]);
        let mut c = (c1 & 0x03) << 4;
        let Some(&c2) = chunk.get(1) else {
            out.push(BCRYPT_ALPHABET[c as usize]);
            break;
        };
        c |= c2 >> 4;
        out.push(BCRYPT_ALPHABET[c as usize]);
        c = (c2 & 0x0f) << 2;
        let Some(&c3) = chunk.get(2) else {
            out.push(BCRYPT_ALPHABET[c as usize]);
            break;
        };
        c |= c3 >> 6;
        out.push(BCRYPT_ALPHABET[c as usize]);
        out.push(BCRYPT_ALPHABET[(c3 & 0x3f) as usize]);
    }
    out
}

fn bcrypt_decode(s: &[u8], len: usize) -> Option<Vec<u8>> {
    let val = |c: u8| BCRYPT_ALPHABET.iter().position(|&a| a == c).map(|i| i as u8);
    let mut out = Vec::new();
    let mut i = 0;
    while out.len() < len {
        let c1 = val(*s.get(i)?)?;
        let c2 = val(*s.get(i + 1)?)?;
        out.push(c1 << 2 | (c2 & 0x30) >> 4);
        if out.len() >= len {
            break;
        }
        let c3 = val(*s.get(i + 2)?)?;
        out.push((c2 & 0x0f) << 4 | (c3 & 0x3c) >> 2);
        if out.len() >= len {
            break;
        }
        let c4 = val(*s.get(i + 3)?)?;
        out.push((c3 & 0x03) << 6 | c4);
        i += 4;
    }
    Some(out)
}

/// bcrypt with a `$2y$NN$` + 22-character salt setting; None if malformed
pub fn bcrypt(password: &[u8], setting: &[u8]) -> Option<Vec<u8>> {
    if setting.len() < 29 || setting[0] != b'$' || setting[1] != b'2' || setting[3] != b'$' || setting[6] != b'$' {
        return None;
    }
    let variant = setting[2];
    if !matches!(variant, b'a' | b'b' | b'x' | b'y') {
        return None;
    }
    let cost: u32 = std::str::from_utf8(&setting[4..6]).ok()?.parse().ok()?;
    if !(4..=31).contains(&cost) {
        return None;
    }
    let salt_text = &setting[7..29];
    let salt = bcrypt_decode(salt_text, 16)?;
    // The key is the password with its terminating NUL, at most 72 bytes
    let mut key = password.to_vec();
    key.push(0);
    key.truncate(72);
    let mut bf = Blowfish { p: initial().p, s: initial().s };
    bf.expand(&salt, &key);
    for _ in 0..(1u64 << cost) {
        bf.expand(&[], &key);
        bf.expand(&[], &salt);
    }
    let mut ctext: Vec<u32> = b"OrpheanBeholderScryDoubt".chunks(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
    for _ in 0..64 {
        for i in (0..6).step_by(2) {
            let (mut l, mut r) = (ctext[i], ctext[i + 1]);
            bf.encrypt(&mut l, &mut r);
            ctext[i] = l;
            ctext[i + 1] = r;
        }
    }
    let bytes: Vec<u8> = ctext.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut out = setting[..7].to_vec();
    // The salt is re-encoded, so its last character is normalized
    out.extend_from_slice(&bcrypt_encode(&salt)[..22]);
    out.extend_from_slice(&bcrypt_encode(&bytes[..23]));
    Some(out)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The cost of a `$2y$` hash
fn hash_cost(hash: &[u8]) -> Option<i64> {
    if hash.len() == 60 && hash.starts_with(b"$2y$") {
        std::str::from_utf8(&hash[4..6]).ok()?.parse().ok()
    } else {
        None
    }
}

fn options_cost(options: Option<Value>) -> EResult<i64> {
    let Some(Value::Array(o)) = options.map(|o| o.copy_out()) else { return Ok(DEFAULT_COST) };
    match o.get(&Key::Str((*b"cost").into())) {
        Some(c) => {
            let cost = c.to_int();
            if !(4..=31).contains(&cost) {
                return Err(Fatal::value_error("password_hash(): Argument #3 ($options) must contain a \"cost\" value between 4 and 31"));
            }
            Ok(cost)
        }
        None => Ok(DEFAULT_COST),
    }
}

fn algo_arg(rt: &mut dyn Runtime, fname: &str, v: Value) -> EResult<()> {
    match v.copy_out() {
        Value::Null => Ok(()),
        Value::Str(s) if s.as_slice() == b"2y" => Ok(()),
        Value::Int(1) => Ok(()),
        _ => {
            let _ = rt;
            Err(Fatal::value_error(format!("{fname}(): Argument #2 ($algo) must be a valid password hashing algorithm")))
        }
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "password_hash" => {
            let [pw, algo, options] = args::<2, 3>(name, vals)?;
            let pw = arg_str(rt, lname, 1, "password", &pw.unwrap())?;
            algo_arg(rt, lname, algo.unwrap())?;
            if pw.contains(&0) {
                return Err(Fatal::value_error("Bcrypt password must not contain a null character"));
            }
            let cost = options_cost(options)?;
            let mut salt = [0u8; 16];
            if !super::random::os_random(&mut salt) {
                return Err(Fatal::new("Random\\RandomException", "Failed to open /dev/urandom"));
            }
            let mut setting = format!("$2y${cost:02}$").into_bytes();
            setting.extend_from_slice(&bcrypt_encode(&salt)[..22]);
            Ok(bcrypt(&pw, &setting).map_or(Value::Bool(false), Value::str))
        }
        "password_verify" => {
            let [pw, hash] = args::<2, 2>(name, vals)?;
            let pw = arg_str(rt, lname, 1, "password", &pw.unwrap())?;
            let hash = arg_str(rt, lname, 2, "hash", &hash.unwrap())?;
            let computed = bcrypt(&pw, &hash);
            Ok(Value::Bool(computed.is_some_and(|c| constant_time_eq(&c, &hash))))
        }
        "password_needs_rehash" => {
            let [hash, algo, options] = args::<2, 3>(name, vals)?;
            let hash = arg_str(rt, lname, 1, "hash", &hash.unwrap())?;
            algo_arg(rt, lname, algo.unwrap())?;
            let cost = options_cost(options)?;
            Ok(Value::Bool(hash_cost(&hash) != Some(cost)))
        }
        "password_get_info" => {
            let [hash] = args::<1, 1>(name, vals)?;
            let hash = arg_str(rt, lname, 1, "hash", &hash.unwrap())?;
            let mut out = PhpArray::new();
            let mut options = PhpArray::new();
            match hash_cost(&hash) {
                Some(cost) => {
                    out.insert(Key::Str((*b"algo").into()), Value::str("2y"));
                    out.insert(Key::Str((*b"algoName").into()), Value::str("bcrypt"));
                    options.insert(Key::Str((*b"cost").into()), Value::Int(cost));
                }
                None => {
                    out.insert(Key::Str((*b"algo").into()), Value::Null);
                    out.insert(Key::Str((*b"algoName").into()), Value::str("unknown"));
                }
            }
            out.insert(Key::Str((*b"options").into()), Value::Array(Rc::new(options)));
            Ok(Value::Array(Rc::new(out)))
        }
        "password_algos" => {
            args::<0, 0>(name, vals)?;
            Ok(super::list([Value::str("2y")]))
        }
        _ => {
            let [s, salt] = args::<2, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let salt = arg_str(rt, lname, 2, "salt", &salt.unwrap())?;
            // Only Blowfish is available; other salts fail as php-src's "*0"
            Ok(Value::str(bcrypt(&s, &salt).unwrap_or_else(|| if salt.starts_with(b"*0") { b"*1".to_vec() } else { b"*0".to_vec() })))
        }
    }
}
