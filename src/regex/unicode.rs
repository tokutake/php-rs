//! Lookups in the Unicode tables, the way PCRE2 does them.

use super::parse::Prop;
use super::ucd::{self, PropType};

/// General categories (the values of PropType::Gc)
pub const GC_C: u8 = 0;
pub const GC_L: u8 = 1;
pub const GC_N: u8 = 3;
pub const GC_P: u8 = 4;
pub const GC_S: u8 = 5;
pub const GC_Z: u8 = 6;

/// Particular categories used directly (the values of PropType::Pc)
pub const CC: u8 = 0;
pub const CF: u8 = 1;
pub const LL: u8 = 5;
pub const LT: u8 = 8;
pub const LU: u8 = 9;
pub const ND: u8 = 13;
pub const ZL: u8 = 27;
pub const ZP: u8 = 28;

type Record = (u8, u8, u8, u8, i32, u16, u16);

#[inline]
fn record(c: u32) -> &'static Record {
    let c = c as usize;
    if c > 0x10ffff {
        return &ucd::RECORDS[0];
    }
    let block = ucd::STAGE1[c / 128] as usize;
    &ucd::RECORDS[ucd::STAGE2[block * 128 + c % 128] as usize]
}

#[inline]
pub fn chartype(c: u32) -> u8 {
    record(c).1
}

/// The general category of a particular category
#[inline]
pub fn gentype(chartype: u8) -> u8 {
    match chartype {
        0..=4 => 0,
        5..=9 => 1,
        10..=12 => 2,
        13..=15 => 3,
        16..=22 => 4,
        23..=26 => 5,
        _ => 6,
    }
}

#[inline]
pub fn category(c: u32) -> u8 {
    gentype(chartype(c))
}

pub fn grapheme_break(c: u32) -> u8 {
    record(c).2
}

/// The character's other case (itself when it has none)
#[inline]
pub fn other_case(c: u32) -> u32 {
    (c as i64 + record(c).4 as i64) as u32
}

/// The set of characters that are all caselessly equal to `c`, if there are
/// more than two
pub fn caseless_set(c: u32) -> Option<&'static [u32]> {
    let offset = record(c).3 as usize;
    if offset == 0 {
        return None;
    }
    let set = &ucd::CASELESS_SETS[offset..];
    let end = set.iter().position(|&x| x == u32::MAX).unwrap_or(set.len());
    Some(&set[..end])
}

/// Whether two characters are the same apart from case
pub fn caseless_eq(a: u32, b: u32) -> bool {
    if a == b {
        return true;
    }
    if other_case(a) == b {
        return true;
    }
    caseless_set(a).is_some_and(|s| s.contains(&b))
}

/// `\w` in UCP mode: letters, numbers and underscore
#[inline]
pub fn is_word(c: u32) -> bool {
    if c < 128 {
        return (c as u8).is_ascii_alphanumeric() || c == b'_' as u32;
    }
    matches!(category(c), GC_L | GC_N)
}

pub fn is_hspace(c: u32) -> bool {
    matches!(
        c,
        0x09 | 0x20 | 0xa0 | 0x1680 | 0x180e | 0x2000..=0x200a | 0x202f | 0x205f | 0x3000
    )
}

pub fn is_vspace(c: u32) -> bool {
    matches!(c, 0x0a..=0x0d | 0x85 | 0x2028 | 0x2029)
}

/// Whether `c` has a `\p` property
pub fn has_prop(c: u32, prop: Prop) -> bool {
    let rec = record(c);
    let v = prop.value;
    match prop.ptype {
        PropType::Any => true,
        PropType::Lamp => matches!(rec.1, LL | LU | LT),
        PropType::Gc => gentype(rec.1) as u16 == v,
        PropType::Pc => rec.1 as u16 == v,
        PropType::Sc => rec.0 as u16 == v,
        PropType::Scx => {
            rec.0 as u16 == v || {
                let base = (rec.5 & 0x3ff) as usize;
                ucd::SCRIPT_SETS[base + v as usize / 32] & (1 << (v % 32)) != 0
            }
        }
        PropType::Alnum => matches!(gentype(rec.1), GC_L | GC_N),
        PropType::Space | PropType::Pxspace => is_hspace(c) || is_vspace(c) || gentype(rec.1) == GC_Z,
        PropType::Word => matches!(gentype(rec.1), GC_L | GC_N) || c == b'_' as u32,
        PropType::Ucnc => c == b'$' as u32 || c == b'@' as u32 || c == b'`' as u32 || (0xa0..=0xd7ff).contains(&c) || c >= 0xe000,
        PropType::Bidicl => (rec.5 >> 11) == v,
        PropType::Bool => {
            let base = (rec.6 & 0xfff) as usize;
            ucd::BOOLPROP_SETS[base + v as usize / 32] & (1 << (v % 32)) != 0
        }
    }
}

/// POSIX [:graph:] in UCP mode
pub fn is_graph(c: u32) -> bool {
    let ct = chartype(c);
    let gt = gentype(ct);
    gt != GC_Z && (gt != GC_C || ct == CF && c != 0x061c && c != 0x180e && !(0x2066..=0x2069).contains(&c))
}

/// POSIX [:print:] in UCP mode
pub fn is_print(c: u32) -> bool {
    let ct = chartype(c);
    ct != ZL && ct != ZP && (gentype(ct) != GC_C || ct == CF && c != 0x061c && !(0x2066..=0x2069).contains(&c))
}

/// POSIX [:punct:] in UCP mode
pub fn is_punct(c: u32) -> bool {
    let gt = category(c);
    gt == GC_P || c < 128 && gt == GC_S
}

/// The end of the extended grapheme cluster that starts with `c`, whose
/// next character is at `pos`
pub fn grapheme_end(s: &[u8], mut pos: usize, c: u32, utf: bool) -> usize {
    let mut lgb = grapheme_break(c);
    while pos < s.len() {
        let (c, len) = if utf { super::decode_utf8(&s[pos..]) } else { (s[pos] as u32, 1) };
        let rgb = grapheme_break(c);
        if ucd::GB_TABLE[lgb as usize] & (1 << rgb) == 0 {
            break;
        }
        // Regional indicators pair up: no break only after an even number of them
        if lgb == ucd::GBREGIONAL_INDICATOR && rgb == ucd::GBREGIONAL_INDICATOR {
            let mut count = 0;
            let mut b = prev_char(s, pos, utf);
            while b > 0 {
                let p = prev_char(s, b, utf);
                let ch = if utf { super::decode_utf8(&s[p..]).0 } else { s[p] as u32 };
                if grapheme_break(ch) != ucd::GBREGIONAL_INDICATOR {
                    break;
                }
                count += 1;
                b = p;
            }
            if count % 2 == 1 {
                break;
            }
        }
        if (rgb != ucd::GBEXTEND && rgb != ucd::GBZWJ) || lgb != ucd::GBEXTENDED_PICTOGRAPHIC {
            lgb = rgb;
        }
        pos += len;
    }
    pos
}

/// The start of the character before `pos`
#[inline]
pub fn prev_char(s: &[u8], pos: usize, utf: bool) -> usize {
    let mut p = pos - 1;
    if utf {
        while p > 0 && s[p] & 0xc0 == 0x80 {
            p -= 1;
        }
    }
    p
}
