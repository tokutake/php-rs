//! Message digests: md5(), sha1(), hash(), hash_hmac() and their *_file forms,
//! with the MD5, SHA-1, SHA-2, SHA-3, CRC, Adler, FNV and Jenkins algorithms

use super::{arg_bool, arg_str, args, Runtime};
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "md5", "sha1", "md5_file", "sha1_file", "hash", "hash_file", "hash_hmac", "hash_hmac_file", "hash_algos",
    "hash_hmac_algos", "hash_equals",
];

/// An algorithm: name, block size (for HMAC; 0 for the non-cryptographic
/// checksums) and the digest function
type Algo = (&'static str, usize, fn(&[u8]) -> Vec<u8>);

/// The algorithms, in hash_algos() order
const ALGOS: &[Algo] = &[
    ("md5", 64, md5),
    ("sha1", 64, sha1),
    ("sha224", 64, |m| sha256_with(m, &SHA224_IV, 28)),
    ("sha256", 64, |m| sha256_with(m, &SHA256_IV, 32)),
    ("sha384", 128, |m| sha512_with(m, &SHA384_IV, 48)),
    ("sha512/224", 128, |m| sha512_with(m, &SHA512_224_IV, 28)),
    ("sha512/256", 128, |m| sha512_with(m, &SHA512_256_IV, 32)),
    ("sha512", 128, |m| sha512_with(m, &SHA512_IV, 64)),
    ("sha3-224", 144, |m| sha3(m, 28)),
    ("sha3-256", 136, |m| sha3(m, 32)),
    ("sha3-384", 104, |m| sha3(m, 48)),
    ("sha3-512", 72, |m| sha3(m, 64)),
    ("adler32", 0, adler32),
    // php-src's "crc32" is the bzip2 CRC, written least significant byte first
    ("crc32", 0, |m| crc_msb(m, 0x04C1_1DB7).to_le_bytes().to_vec()),
    ("crc32b", 0, |m| super::string::crc32(m).to_be_bytes().to_vec()),
    ("crc32c", 0, |m| crc_lsb(m, 0x82F6_3B78).to_be_bytes().to_vec()),
    ("fnv132", 0, |m| fnv32(m, false)),
    ("fnv1a32", 0, |m| fnv32(m, true)),
    ("fnv164", 0, |m| fnv64(m, false)),
    ("fnv1a64", 0, |m| fnv64(m, true)),
    ("joaat", 0, joaat),
];

fn find(algo: &[u8]) -> Option<&'static Algo> {
    ALGOS.iter().find(|(n, _, _)| n.as_bytes().eq_ignore_ascii_case(algo))
}

fn hex(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().flat_map(|b| format!("{b:02x}").into_bytes()).collect()
}

/// Merkle–Damgård padding: 0x80, zeros, then the bit length in `len_bytes`
/// bytes (big- or little-endian), up to a whole number of blocks
fn pad(m: &[u8], block: usize, len_bytes: usize, big_endian: bool) -> Vec<u8> {
    let mut data = m.to_vec();
    data.push(0x80);
    while data.len() % block != block - len_bytes {
        data.push(0);
    }
    let bits = (m.len() as u128).wrapping_mul(8);
    let len = if big_endian { bits.to_be_bytes() } else { bits.to_le_bytes() };
    if big_endian {
        data.extend_from_slice(&len[16 - len_bytes..]);
    } else {
        data.extend_from_slice(&len[..len_bytes]);
    }
    data
}

pub fn md5(m: &[u8]) -> Vec<u8> {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15,
        21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let mut h: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    for block in pad(m, 64, 8, false).chunks(64) {
        let w: Vec<u32> = block.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let t = d;
            d = c;
            c = b;
            b = b.wrapping_add(a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(w[g]).rotate_left(S[i]));
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn sha1(m: &[u8]) -> Vec<u8> {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    for block in pad(m, 64, 8, true).chunks(64) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i / 20 {
                0 => ((b & c) | (!b & d), 0x5A827999),
                1 => (b ^ c ^ d, 0x6ED9EBA1),
                2 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().flat_map(|x| x.to_be_bytes()).collect()
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];
const SHA256_IV: [u32; 8] =
    [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
const SHA224_IV: [u32; 8] =
    [0xc1059ed8, 0x367cd507, 0x3070dd17, 0xf70e5939, 0xffc00b31, 0x68581511, 0x64f98fa7, 0xbefa4fa4];

fn sha256_with(m: &[u8], iv: &[u32; 8], out_len: usize) -> Vec<u8> {
    let mut h = *iv;
    for block in pad(m, 64, 8, true).chunks(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let [a, b, c, d, e, f, g, hh] = v;
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (x, y) in h.iter_mut().zip(v) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out: Vec<u8> = h.iter().flat_map(|x| x.to_be_bytes()).collect();
    out.truncate(out_len);
    out
}

const SHA512_K: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc, 0x3956c25bf348b538,
    0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118, 0xd807aa98a3030242, 0x12835b0145706fbe,
    0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2, 0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235,
    0xc19bf174cf692694, 0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5, 0x983e5152ee66dfab,
    0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4, 0xc6e00bf33da88fc2, 0xd5a79147930aa725,
    0x06ca6351e003826f, 0x142929670a0e6e70, 0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df, 0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30, 0xd192e819d6ef5218,
    0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8, 0x19a4c116b8d2d0c8, 0x1e376c085141ab53,
    0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8, 0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3, 0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b, 0xca273eceea26619c,
    0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178, 0x06f067aa72176fba, 0x0a637dc5a2c898a6,
    0x113f9804bef90dae, 0x1b710b35131c471b, 0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c, 0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];
const SHA512_IV: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1,
    0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
];
const SHA384_IV: [u64; 8] = [
    0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939, 0x67332667ffc00b31,
    0x8eb44a8768581511, 0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4,
];
const SHA512_224_IV: [u64; 8] = [
    0x8C3D37C819544DA2, 0x73E1996689DCD4D6, 0x1DFAB7AE32FF9C82, 0x679DD514582F9FCF, 0x0F6D2B697BD44DA8,
    0x77E36F7304C48942, 0x3F9D85A86A1D36C8, 0x1112E6AD91D692A1,
];
const SHA512_256_IV: [u64; 8] = [
    0x22312194FC2BF72C, 0x9F555FA3C84C64C2, 0x2393B86B6F53B151, 0x963877195940EABD, 0x96283EE2A88EFFE3,
    0xBE5E1E2553863992, 0x2B0199FC2C85B8AA, 0x0EB72DDC81C52CA2,
];

fn sha512_with(m: &[u8], iv: &[u64; 8], out_len: usize) -> Vec<u8> {
    let mut h = *iv;
    for block in pad(m, 128, 16, true).chunks(128) {
        let mut w = [0u64; 80];
        for (i, c) in block.chunks(8).enumerate() {
            w[i] = u64::from_be_bytes(c.try_into().unwrap());
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..80 {
            let [a, b, c, d, e, f, g, hh] = v;
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA512_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (x, y) in h.iter_mut().zip(v) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out: Vec<u8> = h.iter().flat_map(|x| x.to_be_bytes()).collect();
    out.truncate(out_len);
    out
}

fn keccak_f(a: &mut [u64; 25]) {
    const RC: [u64; 24] = [
        0x0000000000000001, 0x0000000000008082, 0x800000000000808a, 0x8000000080008000, 0x000000000000808b,
        0x0000000080000001, 0x8000000080008081, 0x8000000000008009, 0x000000000000008a, 0x0000000000000088,
        0x0000000080008009, 0x000000008000000a, 0x000000008000808b, 0x800000000000008b, 0x8000000000008089,
        0x8000000000008003, 0x8000000000008002, 0x8000000000000080, 0x000000000000800a, 0x800000008000000a,
        0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
    ];
    const ROT: [u32; 25] = [0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56, 14];
    for rc in RC {
        let mut c = [0u64; 5];
        for x in 0..5 {
            c[x] = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
        }
        for x in 0..5 {
            let d = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
            for y in 0..5 {
                a[x + 5 * y] ^= d;
            }
        }
        let mut b = [0u64; 25];
        for x in 0..5 {
            for y in 0..5 {
                b[y + 5 * ((2 * x + 3 * y) % 5)] = a[x + 5 * y].rotate_left(ROT[x + 5 * y]);
            }
        }
        for x in 0..5 {
            for y in 0..5 {
                a[x + 5 * y] = b[x + 5 * y] ^ (!b[(x + 1) % 5 + 5 * y] & b[(x + 2) % 5 + 5 * y]);
            }
        }
        a[0] ^= rc;
    }
}

fn sha3(m: &[u8], out_len: usize) -> Vec<u8> {
    let rate = 200 - 2 * out_len;
    let mut data = m.to_vec();
    data.push(0x06);
    while data.len() % rate != 0 {
        data.push(0);
    }
    *data.last_mut().unwrap() |= 0x80;
    let mut a = [0u64; 25];
    for block in data.chunks(rate) {
        for (i, c) in block.chunks(8).enumerate() {
            a[i] ^= u64::from_le_bytes(c.try_into().unwrap());
        }
        keccak_f(&mut a);
    }
    let mut out: Vec<u8> = a.iter().flat_map(|x| x.to_le_bytes()).collect();
    out.truncate(out_len);
    out
}

fn adler32(m: &[u8]) -> Vec<u8> {
    let (mut a, mut b) = (1u32, 0u32);
    for &c in m {
        a = (a + c as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16 | a).to_be_bytes().to_vec()
}

/// A CRC-32 processed most significant bit first
fn crc_msb(m: &[u8], poly: u32) -> u32 {
    let mut crc = !0u32;
    for &b in m {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ poly } else { crc << 1 };
        }
    }
    !crc
}

/// A CRC-32 processed least significant bit first, with the reflected polynomial
fn crc_lsb(m: &[u8], poly: u32) -> u32 {
    let mut crc = !0u32;
    for &b in m {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ poly } else { crc >> 1 };
        }
    }
    !crc
}

fn fnv32(m: &[u8], a: bool) -> Vec<u8> {
    let mut h = 0x811c9dc5u32;
    for &c in m {
        if a {
            h ^= c as u32;
            h = h.wrapping_mul(0x01000193);
        } else {
            h = h.wrapping_mul(0x01000193);
            h ^= c as u32;
        }
    }
    h.to_be_bytes().to_vec()
}

fn fnv64(m: &[u8], a: bool) -> Vec<u8> {
    let mut h = 0xcbf29ce484222325u64;
    for &c in m {
        if a {
            h ^= c as u64;
            h = h.wrapping_mul(0x100000001b3);
        } else {
            h = h.wrapping_mul(0x100000001b3);
            h ^= c as u64;
        }
    }
    h.to_be_bytes().to_vec()
}

fn joaat(m: &[u8]) -> Vec<u8> {
    let mut h = 0u32;
    for &c in m {
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h.to_be_bytes().to_vec()
}

fn hmac(f: fn(&[u8]) -> Vec<u8>, block: usize, key: &[u8], m: &[u8]) -> Vec<u8> {
    let mut k = if key.len() > block { f(key) } else { key.to_vec() };
    k.resize(block, 0);
    let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    inner.extend_from_slice(m);
    let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    outer.extend_from_slice(&f(&inner));
    f(&outer)
}

fn opt_binary(lname: &str, n: usize, v: Option<Value>) -> EResult<bool> {
    match v {
        Some(b) => arg_bool(lname, n, "binary", &b),
        None => Ok(false),
    }
}

fn digest_value(d: Vec<u8>, binary: bool) -> Value {
    Value::str(if binary { d } else { hex(&d) })
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    let algo_arg = |rt: &mut dyn Runtime, v: Value, hmac: bool| {
        let a = arg_str(rt, lname, 1, "algo", &v)?;
        match find(&a) {
            Some(algo) if !hmac || algo.1 > 0 => Ok(algo),
            _ => Err(Fatal::value_error(format!(
                "{lname}(): Argument #1 ($algo) must be a valid {}hashing algorithm",
                if hmac { "cryptographic " } else { "" }
            ))),
        }
    };
    match lname {
        "md5" | "sha1" => {
            let [s, binary] = args::<1, 2>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let binary = opt_binary(lname, 2, binary)?;
            Ok(digest_value(if lname == "md5" { md5(&s) } else { sha1(&s) }, binary))
        }
        "md5_file" | "sha1_file" => {
            let [path, binary] = args::<1, 2>(name, vals)?;
            let binary = opt_binary(lname, 2, binary)?;
            let data = match super::streams::read_file(rt, lname, 1, "filename", &path.unwrap())? {
                Ok(d) => d,
                Err(v) => return Ok(v),
            };
            Ok(digest_value(if lname == "md5_file" { md5(&data) } else { sha1(&data) }, binary))
        }
        "hash" | "hash_file" => {
            let [algo, data, binary, _options] = args::<2, 4>(name, vals)?;
            let (_, _, f) = algo_arg(rt, algo.unwrap(), false)?;
            let binary = opt_binary(lname, 3, binary)?;
            let data = if lname == "hash" {
                arg_str(rt, lname, 2, "data", &data.unwrap())?
            } else {
                match super::streams::read_file(rt, lname, 2, "filename", &data.unwrap())? {
                    Ok(d) => d,
                    Err(v) => return Ok(v),
                }
            };
            Ok(digest_value(f(&data), binary))
        }
        "hash_hmac" | "hash_hmac_file" => {
            let [algo, data, key, binary] = args::<3, 4>(name, vals)?;
            let (_, block, f) = algo_arg(rt, algo.unwrap(), true)?;
            let key = arg_str(rt, lname, 3, "key", &key.unwrap())?;
            let binary = opt_binary(lname, 4, binary)?;
            let data = if lname == "hash_hmac" {
                arg_str(rt, lname, 2, "data", &data.unwrap())?
            } else {
                match super::streams::read_file(rt, lname, 2, "filename", &data.unwrap())? {
                    Ok(d) => d,
                    Err(v) => return Ok(v),
                }
            };
            Ok(digest_value(hmac(*f, *block, &key, &data), binary))
        }
        "hash_algos" | "hash_hmac_algos" => {
            args::<0, 0>(name, vals)?;
            let hmac = lname == "hash_hmac_algos";
            Ok(super::list(ALGOS.iter().filter(|a| !hmac || a.1 > 0).map(|a| Value::str(a.0))))
        }
        "hash_equals" => {
            let [known, user] = args::<2, 2>(name, vals)?;
            let (known, user) = (known.unwrap(), user.unwrap());
            for (n, param, v) in [(1, "known_string", &known), (2, "user_string", &user)] {
                if !matches!(v.copy_out(), Value::Str(_)) {
                    return Err(Fatal::type_error(format!(
                        "hash_equals(): Argument #{n} (${param}) must be of type string, {} given",
                        v.value_name()
                    )));
                }
            }
            let (known, user) = (known.to_bytes(), user.to_bytes());
            // Compares every byte, so the time does not depend on where they differ
            let diff = known.iter().zip(&user).fold(0u8, |acc, (a, b)| acc | (a ^ b));
            Ok(Value::Bool(known.len() == user.len() && diff == 0))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
