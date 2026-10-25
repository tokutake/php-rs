//! A PCRE2-compatible regular expression engine for the preg_* functions.
//!
//! Patterns are parsed (parse.rs), compiled to a small instruction set
//! (compile.rs) and run by a backtracking matcher (exec.rs) that keeps its
//! choice points on a heap stack. Matching is bounded like php-src's: a
//! backtracking budget (pcre.backtrack_limit), a cap on the choice point
//! stack (the JIT stack limit) and on nested assertions and recursion, so a
//! pathological pattern ends in an error instead of a hang or a crash.

mod compile;
mod exec;
mod parse;
mod ucd;
mod unicode;

pub use exec::{Captures, ExecError};

#[derive(Clone, Copy, Default, Debug)]
pub struct Options {
    pub caseless: bool,
    pub multiline: bool,
    pub dotall: bool,
    pub extended: bool,
    pub no_auto_capture: bool,
    pub anchored: bool,
    pub dollar_endonly: bool,
    pub ungreedy: bool,
    pub utf: bool,
    pub ucp: bool,
    pub dupnames: bool,
}

/// Which character sequences are newlines, for `.`, `^`, `$` and `\N`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Newline {
    Cr,
    Lf,
    CrLf,
    AnyCrLf,
    Any,
    Nul,
}

/// A compile error: a PCRE2 error number (compile errors from 100, UTF-8
/// errors as their negative match error number) and an offset in the pattern
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileError {
    pub code: u32,
    pub offset: usize,
}

impl CompileError {
    pub fn message(&self) -> &'static str {
        if self.code < 100 {
            return UTF8_ERRORS.get(self.code as usize).copied().unwrap_or("internal error");
        }
        COMPILE_ERRORS.get((self.code - 100) as usize).copied().unwrap_or("internal error")
    }
}

/// PCRE2's compile error texts, by number
const COMPILE_ERRORS: &[&str] = &[
    "no error",
    "\\ at end of pattern",
    "\\c at end of pattern",
    "unrecognized character follows \\",
    "numbers out of order in {} quantifier",
    "number too big in {} quantifier",
    "missing terminating ] for character class",
    "escape sequence is invalid in character class",
    "range out of order in character class",
    "quantifier does not follow a repeatable item",
    "internal error: unexpected repeat",
    "unrecognized character after (? or (?-",
    "POSIX named classes are supported only within a class",
    "POSIX collating elements are not supported",
    "missing closing parenthesis",
    "reference to non-existent subpattern",
    "pattern passed as NULL",
    "unrecognised compile-time option bit(s)",
    "missing ) after (?# comment",
    "parentheses are too deeply nested",
    "regular expression is too large",
    "failed to allocate heap memory",
    "unmatched closing parenthesis",
    "internal error: code overflow",
    "missing closing parenthesis for condition",
    "lookbehind assertion is not fixed length",
    "a relative value of zero is not allowed",
    "conditional subpattern contains more than two branches",
    "assertion expected after (?( or (?(?C)",
    "digit expected after (?+ or (?-",
    "unknown POSIX class name",
    "internal error in pcre2_study(): should not occur",
    "this version of PCRE2 does not have Unicode support",
    "parentheses are too deeply nested (stack check)",
    "character code point value in \\x{} or \\o{} is too large",
    "lookbehind is too complicated",
    "\\C is not allowed in a lookbehind assertion in UTF-8 mode",
    "PCRE2 does not support \\F, \\L, \\l, \\N{name}, \\U, or \\u",
    "number after (?C is greater than 255",
    "closing parenthesis for (?C expected",
    "invalid escape sequence in (*VERB) name",
    "unrecognized character after (?P",
    "syntax error in subpattern name (missing terminator?)",
    "two named subpatterns have the same name (PCRE2_DUPNAMES not set)",
    "subpattern name must start with a non-digit",
    "this version of PCRE2 does not have support for \\P, \\p, or \\X",
    "malformed \\P or \\p sequence",
    "unknown property after \\P or \\p",
    "subpattern name is too long (maximum 32 code units)",
    "too many named subpatterns (maximum 10000)",
    "invalid range in character class",
    "octal value is greater than \\377 in 8-bit non-UTF-8 mode",
    "internal error: overran compiling workspace",
    "internal error: previously-checked referenced subpattern not found",
    "DEFINE subpattern contains more than one branch",
    "missing opening brace after \\o",
    "internal error: unknown newline setting",
    "\\g is not followed by a braced, angle-bracketed, or quoted name/number or by a plain number",
    "(?R (recursive pattern call) must be followed by a closing parenthesis",
    "obsolete error (should not occur)",
    "(*VERB) not recognized or malformed",
    "subpattern number is too big",
    "subpattern name expected",
    "internal error: parsed pattern overflow",
    "non-octal character in \\o{} (closing brace missing?)",
    "different names for subpatterns of the same number are not allowed",
    "(*MARK) must have an argument",
    "non-hex character in \\x{} (closing brace missing?)",
    "\\c must be followed by a printable ASCII character",
    "\\k is not followed by a braced, angle-bracketed, or quoted name",
    "internal error: unknown meta code in check_lookbehinds()",
    "\\N is not supported in a class",
    "callout string is too long",
    "disallowed Unicode code point (>= 0xd800 && <= 0xdfff)",
    "using UTF is disabled by the application",
    "using UCP is disabled by the application",
    "name is too long in (*MARK), (*PRUNE), (*SKIP), or (*THEN)",
    "character code point value in \\u.... sequence is too large",
    "digits missing in \\x{} or \\o{} or \\N{U+}",
    "syntax error or number too big in (?(VERSION condition",
    "internal error: unknown opcode in auto_possessify()",
    "missing terminating delimiter for callout with string argument",
    "unrecognized string delimiter follows (?C",
    "using \\C is disabled by the application",
    "(?| and/or (?J: or (?x: parentheses are too deeply nested",
    "using \\C is disabled in this PCRE2 library",
    "regular expression is too complicated",
    "lookbehind assertion is too long",
    "pattern string is longer than the limit set by the application",
    "internal error: unknown code in parsed pattern",
    "internal error: bad code value in parsed_skip()",
    "PCRE2_EXTRA_ALLOW_SURROGATE_ESCAPES is not allowed in UTF-16 mode",
    "invalid option bits with PCRE2_LITERAL",
    "\\N{U+dddd} is supported only in Unicode (UTF) mode",
    "invalid hyphen in option setting",
    "(*alpha_assertion) not recognized",
    "script runs require Unicode support, which this version of PCRE2 does not have",
    "too many capturing groups (maximum 65535)",
    "atomic assertion expected after (?( or (?(?C)",
    "\\K is not allowed in lookarounds (but see PCRE2_EXTRA_ALLOW_LOOKAROUND_BSK)",
];

/// PCRE2's UTF-8 error texts, by the absolute value of their error code
const UTF8_ERRORS: &[&str] = &[
    "no error",
    "no match",
    "partial match",
    "UTF-8 error: 1 byte missing at end",
    "UTF-8 error: 2 bytes missing at end",
    "UTF-8 error: 3 bytes missing at end",
    "UTF-8 error: 4 bytes missing at end",
    "UTF-8 error: 5 bytes missing at end",
    "UTF-8 error: byte 2 top bits not 0x80",
    "UTF-8 error: byte 3 top bits not 0x80",
    "UTF-8 error: byte 4 top bits not 0x80",
    "UTF-8 error: byte 5 top bits not 0x80",
    "UTF-8 error: byte 6 top bits not 0x80",
    "UTF-8 error: 5-byte character is not allowed (RFC 3629)",
    "UTF-8 error: 6-byte character is not allowed (RFC 3629)",
    "UTF-8 error: code points greater than 0x10ffff are not defined",
    "UTF-8 error: code points 0xd800-0xdfff are not defined",
    "UTF-8 error: overlong 2-byte sequence",
    "UTF-8 error: overlong 3-byte sequence",
    "UTF-8 error: overlong 4-byte sequence",
    "UTF-8 error: overlong 5-byte sequence",
    "UTF-8 error: overlong 6-byte sequence",
    "UTF-8 error: isolated byte with 0x80 bit set",
    "UTF-8 error: illegal byte (0xfe or 0xff)",
];

/// PCRE2's UTF-8 check: Err((error number, offset of the bad character)).
/// The numbers index UTF8_ERRORS
pub fn validate_utf8(s: &[u8]) -> Result<(), (u32, usize)> {
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c < 0x80 {
            i += 1;
            continue;
        }
        if c < 0xc0 {
            return Err((22, i));
        }
        if c >= 0xfe {
            return Err((23, i));
        }
        let extra = match c {
            0xc0..=0xdf => 1,
            0xe0..=0xef => 2,
            0xf0..=0xf7 => 3,
            0xf8..=0xfb => 4,
            _ => 5,
        };
        let left = s.len() - i - 1;
        if left < extra {
            return Err((3 + (extra - left) as u32 - 1, i));
        }
        for k in 1..=extra {
            if s[i + k] & 0xc0 != 0x80 {
                return Err((7 + k as u32, i));
            }
        }
        let d = s[i + 1];
        match extra {
            1 if c & 0x3e == 0 => return Err((17, i)),
            2 if c == 0xe0 && d & 0x20 == 0 => return Err((18, i)),
            2 if c == 0xed && d >= 0xa0 => return Err((16, i)),
            3 if c == 0xf0 && d & 0x30 == 0 => return Err((19, i)),
            3 if c > 0xf4 || c == 0xf4 && d > 0x8f => return Err((15, i)),
            4 => return Err((if c == 0xf8 && d & 0x38 == 0 { 20 } else { 13 }, i)),
            5 => return Err((if c == 0xfc && d & 0x3c == 0 { 21 } else { 14 }, i)),
            _ => {}
        }
        i += extra + 1;
    }
    Ok(())
}

/// Decodes the (valid) UTF-8 character at the start of `s`: (code point, length)
#[inline]
pub fn decode_utf8(s: &[u8]) -> (u32, usize) {
    let c = s[0] as u32;
    if c < 0x80 {
        (c, 1)
    } else if c < 0xe0 {
        (((c & 0x1f) << 6) | (s[1] as u32 & 0x3f), 2)
    } else if c < 0xf0 {
        (((c & 0x0f) << 12) | ((s[1] as u32 & 0x3f) << 6) | (s[2] as u32 & 0x3f), 3)
    } else {
        (((c & 0x07) << 18) | ((s[1] as u32 & 0x3f) << 12) | ((s[2] as u32 & 0x3f) << 6) | (s[3] as u32 & 0x3f), 4)
    }
}

pub fn encode_utf8(c: u32, out: &mut Vec<u8>) {
    match char::from_u32(c) {
        Some(ch) => out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
        None => out.push(c as u8),
    }
}

pub use compile::Regex;

/// Options for one match attempt
#[derive(Clone, Copy, Default)]
pub struct MatchOptions {
    /// An empty match at the start offset is not a match
    pub notempty_atstart: bool,
    /// Only a match starting at the start offset
    pub anchored: bool,
}

impl Regex {
    pub fn new(pattern: &[u8], opts: &Options) -> Result<Regex, CompileError> {
        let parsed = parse::parse(pattern, opts)?;
        compile::compile(parsed, opts, pattern.len())
    }
}
