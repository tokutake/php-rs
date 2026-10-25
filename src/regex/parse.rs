//! Parses PCRE2 pattern syntax into a tree, with PCRE2's error messages and
//! offsets. Nesting is limited (as in PCRE2), so the recursive descent here,
//! and the recursive walks over the tree, stay shallow.

use super::ucd::PropType;
use super::{CompileError, Options};

/// PCRE2's PARENS_NEST_LIMIT
const NEST_LIMIT: usize = 250;
/// Longest group name
const MAX_NAME: usize = 32;
/// Largest number in a {} quantifier
const MAX_REPEAT: u32 = 65535;
const MAX_GROUPS: usize = 65535;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RepKind {
    Greedy,
    Lazy,
    Possessive,
}

/// A group referred to by number or by name (resolved once the whole pattern is read)
#[derive(Clone, Debug)]
pub enum Ref {
    Num(usize),
    Name(Vec<u8>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Assertion {
    /// `^`
    Start { multiline: bool },
    /// `$`
    End { multiline: bool, endonly: bool },
    /// `\A`
    SubjectStart,
    /// `\z`
    SubjectEnd,
    /// `\Z`
    SubjectEndNewline,
    /// `\G`
    StartOffset,
    WordBoundary,
    NotWordBoundary,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Builtin {
    Digit,
    Space,
    Word,
    HSpace,
    VSpace,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Posix {
    Alpha,
    Lower,
    Upper,
    Alnum,
    Ascii,
    Blank,
    Cntrl,
    Digit,
    Graph,
    Print,
    Punct,
    Space,
    Word,
    Xdigit,
}

const POSIX_NAMES: [(&str, Posix); 14] = [
    ("alpha", Posix::Alpha),
    ("lower", Posix::Lower),
    ("upper", Posix::Upper),
    ("alnum", Posix::Alnum),
    ("ascii", Posix::Ascii),
    ("blank", Posix::Blank),
    ("cntrl", Posix::Cntrl),
    ("digit", Posix::Digit),
    ("graph", Posix::Graph),
    ("print", Posix::Print),
    ("punct", Posix::Punct),
    ("space", Posix::Space),
    ("word", Posix::Word),
    ("xdigit", Posix::Xdigit),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Prop {
    pub ptype: PropType,
    pub value: u16,
}

#[derive(Clone, Debug)]
pub enum ClassItem {
    Range(u32, u32),
    Builtin(Builtin, bool),
    Prop(Prop, bool),
    Posix(Posix, bool),
}

#[derive(Clone, Debug)]
pub struct Class {
    pub negated: bool,
    pub caseless: bool,
    pub items: Vec<ClassItem>,
}

#[derive(Clone, Debug)]
pub enum Verb {
    Fail,
    Accept,
    Commit,
    Prune,
    Skip,
    Then,
    Mark(Vec<u8>),
}

#[derive(Clone, Debug)]
pub enum Cond {
    /// The group and where the condition starts, for errors
    Group(Ref, usize),
    Recursion(Option<Ref>, usize),
    Define,
    Look { behind: bool, neg: bool, node: Box<Node>, offset: usize },
}

#[derive(Clone, Debug)]
pub enum Node {
    Empty,
    Char { c: u32, caseless: bool },
    /// `.`; `\N` is a dot that never matches newlines
    Any { dotall: bool },
    Class(Box<Class>),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Capture { index: usize, node: Box<Node> },
    Group(Box<Node>),
    Atomic(Box<Node>),
    /// A lookaround; `offset` is where it starts, for the lookbehind length error
    Look { behind: bool, neg: bool, node: Box<Node>, offset: usize },
    Repeat { node: Box<Node>, min: u32, max: Option<u32>, kind: RepKind },
    Assert(Assertion),
    Backref { target: Ref, caseless: bool, offset: usize },
    /// Recursion or a subroutine call: `(?R)`, `(?1)`, `(?&name)`
    Call { target: Ref, offset: usize },
    Cond { cond: Cond, yes: Box<Node>, no: Option<Box<Node>> },
    /// `\K`
    Keep,
    /// `\R`
    Newline,
    /// `\X`
    ExtGrapheme,
    /// `\C`
    AnyByte,
    Verb(Verb),
}

#[derive(Clone, Copy)]
struct Flags {
    caseless: bool,
    multiline: bool,
    dotall: bool,
    extended: bool,
    extended_more: bool,
    no_auto_capture: bool,
    ungreedy: bool,
    dupnames: bool,
}

/// The result of parsing: the tree and what it says about its groups
pub struct Parsed {
    pub node: Node,
    pub groups: usize,
    pub names: Vec<(Vec<u8>, usize)>,
    pub utf: bool,
    pub ucp: bool,
    pub newline: super::Newline,
    pub bsr_anycrlf: bool,
    pub no_start_optimize: bool,
    pub notempty: bool,
    pub notempty_atstart: bool,
}

struct Parser<'p> {
    p: &'p [u8],
    pos: usize,
    flags: Flags,
    utf: bool,
    endonly: bool,
    groups: usize,
    /// Groups in the whole pattern, counted up front: `\10` is a back
    /// reference only if there are that many groups
    total_groups: usize,
    names: Vec<(Vec<u8>, usize)>,
    depth: usize,
    /// Set when the item just read was a comment or `\E`, which a quantifier looks through
    transparent: bool,
}

type PResult<T> = Result<T, CompileError>;

fn err(code: u32, offset: usize) -> CompileError {
    CompileError { code, offset }
}

pub fn parse(pattern: &[u8], opts: &Options) -> PResult<Parsed> {
    let mut start = 0;
    let mut utf = opts.utf;
    let mut ucp = opts.ucp;
    let mut newline = super::Newline::Lf;
    let mut bsr_anycrlf = false;
    let mut no_start_optimize = false;
    let mut notempty = false;
    let mut notempty_atstart = false;
    // Leading option settings: (*UTF), (*UCP), newline conventions, ...
    while pattern[start..].starts_with(b"(*") {
        let Some(close) = pattern[start..].iter().position(|&c| c == b')') else { break };
        let name = &pattern[start + 2..start + close];
        let (word, num) = match name.iter().position(|&c| c == b'=') {
            Some(eq) => (&name[..eq], Some(&name[eq + 1..])),
            None => (name, None),
        };
        match (word, num) {
            (b"UTF", None) => utf = true,
            (b"UCP", None) => ucp = true,
            (b"CR", None) => newline = super::Newline::Cr,
            (b"LF", None) => newline = super::Newline::Lf,
            (b"CRLF", None) => newline = super::Newline::CrLf,
            (b"ANYCRLF", None) => newline = super::Newline::AnyCrLf,
            (b"ANY", None) => newline = super::Newline::Any,
            (b"NUL", None) => newline = super::Newline::Nul,
            (b"BSR_ANYCRLF", None) => bsr_anycrlf = true,
            (b"BSR_UNICODE", None) => bsr_anycrlf = false,
            (b"NO_START_OPT", None) => no_start_optimize = true,
            (b"NOTEMPTY", None) => notempty = true,
            (b"NOTEMPTY_ATSTART", None) => notempty_atstart = true,
            (b"NO_AUTO_POSSESS" | b"NO_DOTSTAR_ANCHOR" | b"NO_JIT", None) => {}
            (b"LIMIT_MATCH" | b"LIMIT_DEPTH" | b"LIMIT_HEAP" | b"LIMIT_RECURSION", Some(n))
                if !n.is_empty() && n.iter().all(u8::is_ascii_digit) => {}
            _ => break,
        }
        start += close + 1;
    }
    if utf
        && let Err((code, offset)) = super::validate_utf8(pattern) {
            return Err(err(code, offset));
        }
    let mut parser = Parser {
        p: pattern,
        pos: start,
        flags: Flags {
            caseless: opts.caseless,
            multiline: opts.multiline,
            dotall: opts.dotall,
            extended: opts.extended,
            extended_more: false,
            no_auto_capture: opts.no_auto_capture,
            ungreedy: opts.ungreedy,
            dupnames: opts.dupnames,
        },
        utf,
        endonly: opts.dollar_endonly,
        groups: 0,
        total_groups: count_groups(&pattern[start..], opts.no_auto_capture),
        names: Vec::new(),
        depth: 0,
        transparent: false,
    };
    let node = parser.alternation()?;
    if parser.pos < pattern.len() {
        // Only a `)` stops the top level early
        return Err(err(122, parser.pos));
    }
    let mut parsed = Parsed {
        node,
        groups: parser.groups,
        names: parser.names,
        utf,
        ucp: ucp || utf && opts.ucp,
        newline,
        bsr_anycrlf,
        no_start_optimize,
        notempty,
        notempty_atstart,
    };
    check_refs(&parsed.node, &parsed)?;
    check_lookbehinds(&mut parsed)?;
    Ok(parsed)
}

/// Counts capturing groups the quick way, for deciding what `\10` means
fn count_groups(p: &[u8], no_auto: bool) -> usize {
    let mut n = 0;
    let mut i = 0;
    let mut in_class = false;
    while i < p.len() {
        match p[i] {
            b'\\' => {
                if p.get(i + 1) == Some(&b'Q') {
                    i += 2;
                    while i < p.len() && !(p[i] == b'\\' && p.get(i + 1) == Some(&b'E')) {
                        i += 1;
                    }
                }
                i += 1;
            }
            b'[' if !in_class => {
                in_class = true;
                if p.get(i + 1) == Some(&b'^') {
                    i += 1;
                }
                if p.get(i + 1) == Some(&b']') {
                    i += 1;
                }
            }
            b']' if in_class => in_class = false,
            // The condition of `(?(1)...)` is not a group
            b'(' if !in_class && !(i >= 2 && &p[i - 2..i] == b"(?") => {
                let rest = &p[i + 1..];
                if rest.first() != Some(&b'?') && rest.first() != Some(&b'*') {
                    if !no_auto {
                        n += 1;
                    }
                } else if rest.starts_with(b"?<") && !rest.starts_with(b"?<=") && !rest.starts_with(b"?<!")
                    || rest.starts_with(b"?'")
                    || rest.starts_with(b"?P<")
                {
                    n += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    n
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.p.get(self.pos).copied()
    }

    fn at(&self, i: usize) -> Option<u8> {
        self.p.get(self.pos + i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn rest(&self) -> &[u8] {
        &self.p[self.pos..]
    }

    /// One pattern character: a code point in UTF mode, else a byte
    fn next_char(&mut self) -> u32 {
        if self.utf && self.p[self.pos] >= 0x80 {
            let (c, len) = super::decode_utf8(&self.p[self.pos..]);
            self.pos += len;
            c
        } else {
            self.pos += 1;
            self.p[self.pos - 1] as u32
        }
    }

    /// Whitespace and #-comments in extended mode
    fn skip_extended(&mut self) {
        if !self.flags.extended {
            return;
        }
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) => self.pos += 1,
                Some(b'#') => {
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        if c == b'\n' {
                            break;
                        }
                    }
                }
                _ if self.utf && self.rest().starts_with("\u{85}".as_bytes()) => self.pos += 2,
                _ if self.utf
                    && ["\u{200e}", "\u{200f}", "\u{2028}", "\u{2029}"].iter().any(|s| self.rest().starts_with(s.as_bytes())) =>
                {
                    self.pos += 3
                }
                _ => return,
            }
        }
    }

    fn alternation(&mut self) -> PResult<Node> {
        let mut branches = Vec::new();
        loop {
            branches.push(self.sequence()?);
            if !self.eat(b'|') {
                break;
            }
        }
        Ok(if branches.len() == 1 { branches.pop().unwrap() } else { Node::Alt(branches) })
    }

    /// Alternatives of a `(?|` group: each starts numbering groups afresh
    fn branch_reset(&mut self) -> PResult<Node> {
        let base = self.groups;
        let mut highest = base;
        let mut branches = Vec::new();
        loop {
            self.groups = base;
            branches.push(self.sequence()?);
            highest = highest.max(self.groups);
            if !self.eat(b'|') {
                break;
            }
        }
        self.groups = highest;
        Ok(if branches.len() == 1 { branches.pop().unwrap() } else { Node::Alt(branches) })
    }

    fn sequence(&mut self) -> PResult<Node> {
        let mut items: Vec<Node> = Vec::new();
        // Whether the last item can take a quantifier
        let mut repeatable = false;
        loop {
            self.skip_extended();
            let Some(c) = self.peek() else { break };
            let start = self.pos;
            match c {
                b'|' | b')' => break,
                b'*' | b'+' | b'?' => {
                    if !repeatable {
                        return Err(err(109, start));
                    }
                    self.pos += 1;
                    let (min, max) = match c {
                        b'*' => (0, None),
                        b'+' => (1, None),
                        _ => (0, Some(1)),
                    };
                    self.quantify(&mut items, min, max);
                    repeatable = false;
                    continue;
                }
                b'{' => {
                    if let Some((min, max)) = self.braces()? {
                        if !repeatable {
                            return Err(err(109, self.pos - 1));
                        }
                        self.quantify(&mut items, min, max);
                        repeatable = false;
                        continue;
                    }
                    self.pos += 1;
                    items.push(self.literal(b'{' as u32));
                    repeatable = true;
                    continue;
                }
                _ => {}
            }
            match self.atom()? {
                Some((node, rep)) => {
                    // A literal string from \Q..\E is quantified character by character
                    match node {
                        Node::Seq(chars) if rep => items.extend(chars),
                        node => items.push(node),
                    }
                    repeatable = rep;
                }
                None => {
                    // After an option setting a quantifier has nothing to repeat; comments
                    // and \E are looked through
                    if !std::mem::take(&mut self.transparent) {
                        repeatable = false;
                    }
                }
            }
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Seq(items),
        })
    }

    /// Applies a quantifier (and its `?` or `+` suffix) to the last item
    fn quantify(&mut self, items: &mut [Node], min: u32, max: Option<u32>) {
        let mut kind = if self.flags.ungreedy { RepKind::Lazy } else { RepKind::Greedy };
        if self.eat(b'?') {
            kind = if self.flags.ungreedy { RepKind::Greedy } else { RepKind::Lazy };
        } else if self.eat(b'+') {
            kind = RepKind::Possessive;
        }
        let last = items.last_mut().expect("repeatable item");
        let node = std::mem::replace(last, Node::Empty);
        // A lookaround repeated is just optional, or there once
        *last = match node {
            look @ Node::Look { .. } => {
                if min == 0 {
                    Node::Repeat { node: Box::new(look), min: 0, max: Some(1), kind }
                } else {
                    look
                }
            }
            node => Node::Repeat { node: Box::new(node), min, max, kind },
        };
    }

    /// A `{n}`, `{n,}` or `{n,m}` quantifier, or None when the brace is a literal
    fn braces(&mut self) -> PResult<Option<(u32, Option<u32>)>> {
        let s = self.rest();
        let mut i = 1;
        let digits = |i: &mut usize| {
            let start = *i;
            while s.get(*i).is_some_and(u8::is_ascii_digit) {
                *i += 1;
            }
            start..*i
        };
        let a = digits(&mut i);
        if a.is_empty() {
            return Ok(None);
        }
        let b = if s.get(i) == Some(&b',') {
            i += 1;
            Some(digits(&mut i))
        } else {
            None
        };
        if s.get(i) != Some(&b'}') {
            return Ok(None);
        }
        let num = |r: std::ops::Range<usize>, end: usize| -> PResult<u32> {
            let mut n: u32 = 0;
            for &d in &s[r] {
                n = n * 10 + (d - b'0') as u32;
                if n > MAX_REPEAT {
                    return Err(err(105, self.pos + end));
                }
            }
            Ok(n)
        };
        let min = num(a.clone(), a.end)?;
        let max = match b {
            None => Some(min),
            Some(r) if r.is_empty() => None,
            Some(r) => {
                let m = num(r.clone(), r.end)?;
                if m < min {
                    return Err(err(104, self.pos + i));
                }
                Some(m)
            }
        };
        self.pos += i + 1;
        Ok(Some((min, max)))
    }

    fn literal(&self, c: u32) -> Node {
        Node::Char { c, caseless: self.flags.caseless }
    }

    /// One item and whether a quantifier may follow it. None: nothing to match
    /// (an option setting or a comment)
    fn atom(&mut self) -> PResult<Option<(Node, bool)>> {
        let start = self.pos;
        let c = self.peek().unwrap();
        Ok(Some(match c {
            b'(' => return self.group(),
            b'[' => {
                if let Some(e) = self.posix_outside_class() {
                    return Err(e);
                }
                (self.class()?, true)
            }
            b'.' => {
                self.pos += 1;
                (Node::Any { dotall: self.flags.dotall }, true)
            }
            b'^' => {
                self.pos += 1;
                (Node::Assert(Assertion::Start { multiline: self.flags.multiline }), false)
            }
            b'$' => {
                self.pos += 1;
                (Node::Assert(Assertion::End { multiline: self.flags.multiline, endonly: self.endonly }), false)
            }
            b'\\' => return self.escape_atom(),
            _ => {
                let c = self.next_char();
                let _ = start;
                (self.literal(c), true)
            }
        }))
    }

    /// `[:alpha:]` outside a class is an error in PCRE2
    fn posix_outside_class(&self) -> Option<CompileError> {
        let s = self.rest();
        if s.len() > 2 && matches!(s[1], b':' | b'.' | b'=') && self.posix_end(self.pos + 1).is_some() {
            return Some(err(if s[1] == b':' { 112 } else { 113 }, self.pos));
        }
        None
    }

    /// Where a `[:name:]` starting at `i` (the colon) ends, if it is one
    fn posix_end(&self, i: usize) -> Option<usize> {
        let term = self.p[i];
        let mut j = i + 1;
        while j + 1 < self.p.len() {
            if self.p[j] == b'\\' && matches!(self.p.get(j + 1), Some(b']' | b'\\')) {
                j += 2;
                continue;
            }
            if self.p[j] == term && self.p[j + 1] == b']' {
                return Some(j);
            }
            if self.p[j] == b']' || self.p[j] == b'[' && matches!(self.p.get(j + 1), Some(b':' | b'.' | b'=')) {
                return None;
            }
            j += 1;
        }
        None
    }

    fn escape_atom(&mut self) -> PResult<Option<(Node, bool)>> {
        let start = self.pos;
        self.pos += 1;
        let Some(c) = self.peek() else { return Err(err(101, self.pos)) };
        // \Q...\E: literal text
        if c == b'Q' {
            self.pos += 1;
            let mut chars = Vec::new();
            while self.pos < self.p.len() {
                if self.rest().starts_with(b"\\E") {
                    self.pos += 2;
                    break;
                }
                let ch = self.next_char();
                chars.push(self.literal(ch));
            }
            return Ok(match chars.len() {
                0 => {
                    self.transparent = true;
                    None
                }
                1 => Some((chars.pop().unwrap(), true)),
                _ => Some((Node::Seq(chars), true)),
            });
        }
        if c == b'E' {
            self.pos += 1;
            self.transparent = true;
            return Ok(None);
        }
        let node = match c {
            b'd' | b'D' | b's' | b'S' | b'w' | b'W' | b'h' | b'H' | b'v' | b'V' => {
                self.pos += 1;
                let (b, neg) = builtin(c);
                Node::Class(Box::new(Class { negated: false, caseless: false, items: vec![ClassItem::Builtin(b, neg)] }))
            }
            b'p' | b'P' => {
                self.pos += 1;
                let (prop, neg) = self.property()?;
                Node::Class(Box::new(Class {
                    negated: false,
                    caseless: false,
                    items: vec![ClassItem::Prop(prop, neg != (c == b'P'))],
                }))
            }
            b'b' | b'B' | b'A' | b'z' | b'Z' | b'G' | b'K' => {
                self.pos += 1;
                let node = match c {
                    b'b' => Node::Assert(Assertion::WordBoundary),
                    b'B' => Node::Assert(Assertion::NotWordBoundary),
                    b'A' => Node::Assert(Assertion::SubjectStart),
                    b'z' => Node::Assert(Assertion::SubjectEnd),
                    b'Z' => Node::Assert(Assertion::SubjectEndNewline),
                    b'G' => Node::Assert(Assertion::StartOffset),
                    _ => Node::Keep,
                };
                return Ok(Some((node, false)));
            }
            b'R' => {
                self.pos += 1;
                Node::Newline
            }
            b'X' => {
                self.pos += 1;
                Node::ExtGrapheme
            }
            b'C' => {
                self.pos += 1;
                Node::AnyByte
            }
            b'N' if self.at(1) == Some(b'{') => {
                // \N{U+hhhh}
                if !self.utf {
                    return Err(err(193, self.pos + 1));
                }
                let s = self.rest();
                if !s.starts_with(b"N{U+") {
                    return Err(err(137, self.pos + 1));
                }
                let mut i = 4;
                let mut v: u32 = 0;
                while let Some(d) = s.get(i).and_then(|&d| (d as char).to_digit(16)) {
                    v = v.saturating_mul(16).saturating_add(d);
                    i += 1;
                }
                if i == 4 || s.get(i) != Some(&b'}') {
                    return Err(err(if i == 4 { 178 } else { 167 }, self.pos + i));
                }
                if v > 0x10ffff {
                    return Err(err(134, self.pos + i));
                }
                self.pos += i + 1;
                self.literal(v)
            }
            b'N' => {
                self.pos += 1;
                Node::Any { dotall: false }
            }
            b'g' => return self.g_escape(start).map(|n| Some((n, true))),
            b'k' => {
                self.pos += 1;
                let close = match self.peek() {
                    Some(b'<') => b'>',
                    Some(b'\'') => b'\'',
                    Some(b'{') => b'}',
                    _ => return Err(err(169, self.pos)),
                };
                self.pos += 1;
                let name_start = self.pos;
                let name = self.group_name(close, 169)?;
                Node::Backref { target: Ref::Name(name), caseless: self.flags.caseless, offset: name_start }
            }
            b'1'..=b'9' => {
                // A back reference, or an octal escape when there are not that many groups
                let digits_end =
                    self.rest().iter().position(|d| !d.is_ascii_digit()).map_or(self.p.len(), |i| self.pos + i);
                let n: usize = std::str::from_utf8(&self.p[self.pos..digits_end]).unwrap().parse().unwrap_or(usize::MAX);
                if n < 10 || n <= self.total_groups {
                    let offset = self.pos;
                    self.pos = digits_end;
                    Node::Backref { target: Ref::Num(n), caseless: self.flags.caseless, offset }
                } else if c >= b'8' {
                    return Err(err(115, self.pos));
                } else {
                    let v = self.octal_digits(3);
                    self.char_value(v)?
                }
            }
            _ => {
                let v = self.char_escape(false)?;
                self.literal(v)
            }
        };
        Ok(Some((node, true)))
    }

    fn char_value(&self, v: u32) -> PResult<Node> {
        if !self.utf && v > 0xff {
            return Err(err(151, self.pos));
        }
        Ok(self.literal(v))
    }

    /// Up to `max` octal digits
    fn octal_digits(&mut self, max: usize) -> u32 {
        let mut v = 0;
        for _ in 0..max {
            match self.peek() {
                Some(d @ b'0'..=b'7') => {
                    v = v * 8 + (d - b'0') as u32;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        v
    }

    /// An escape standing for one character, with `self.pos` at the character
    /// after the backslash
    fn char_escape(&mut self, in_class: bool) -> PResult<u32> {
        let c = self.peek().unwrap();
        self.pos += 1;
        Ok(match c {
            b'a' => 7,
            b'e' => 0x1b,
            b'f' => 0x0c,
            b'n' => b'\n' as u32,
            b'r' => b'\r' as u32,
            b't' => b'\t' as u32,
            b'b' if in_class => 8,
            b'0' => self.octal_digits(2),
            b'1'..=b'7' if in_class => {
                self.pos -= 1;
                self.octal_digits(3)
            }
            b'8' | b'9' if in_class => c as u32,
            b'o' => {
                if !self.eat(b'{') {
                    return Err(err(155, self.pos - 1));
                }
                let start = self.pos;
                let mut v: u32 = 0;
                while let Some(d @ b'0'..=b'7') = self.peek() {
                    v = v.saturating_mul(8).saturating_add((d - b'0') as u32);
                    self.pos += 1;
                    if v > if self.utf { 0x10ffff } else { 0xff } {
                        while self.peek().is_some_and(|d| (b'0'..=b'7').contains(&d)) {
                            self.pos += 1;
                        }
                        return Err(err(134, self.pos));
                    }
                }
                if self.pos == start {
                    if self.peek() == Some(b'}') {
                        return Err(err(178, self.pos));
                    }
                    return Err(err(164, self.pos));
                }
                if !self.eat(b'}') {
                    return Err(err(164, self.pos));
                }
                v
            }
            b'x' => {
                if self.eat(b'{') {
                    let start = self.pos;
                    let mut v: u32 = 0;
                    while let Some(d) = self.peek().and_then(|d| (d as char).to_digit(16)) {
                        v = v.saturating_mul(16).saturating_add(d);
                        self.pos += 1;
                        if v > if self.utf { 0x10ffff } else { 0xff } {
                            while self.peek().is_some_and(|d| d.is_ascii_hexdigit()) {
                                self.pos += 1;
                            }
                            return Err(err(134, self.pos));
                        }
                    }
                    if self.pos == start {
                        if self.peek() == Some(b'}') {
                            return Err(err(178, self.pos));
                        }
                        return Err(err(167, self.pos));
                    }
                    if !self.eat(b'}') {
                        return Err(err(167, self.pos));
                    }
                    if self.utf && (0xd800..=0xdfff).contains(&v) {
                        return Err(err(173, self.pos));
                    }
                    v
                } else {
                    let mut v = 0;
                    for _ in 0..2 {
                        match self.peek().and_then(|d| (d as char).to_digit(16)) {
                            Some(d) => {
                                v = v * 16 + d;
                                self.pos += 1;
                            }
                            None => break,
                        }
                    }
                    v
                }
            }
            b'c' => {
                let Some(x) = self.peek() else { return Err(err(102, self.pos)) };
                if !(32..127).contains(&x) {
                    return Err(err(168, self.pos));
                }
                self.pos += 1;
                (x.to_ascii_uppercase() ^ 0x40) as u32
            }
            b'L' | b'l' | b'U' | b'u' | b'N' => return Err(err(137, self.pos)),
            c if c.is_ascii_alphanumeric() => return Err(err(103, self.pos - 1)),
            _ => {
                self.pos -= 1;
                self.next_char()
            }
        })
    }

    /// `\g` references: `\g1`, `\g{-1}`, `\g{name}` (back references) and
    /// `\g<name>`, `\g'1'` (subroutine calls)
    fn g_escape(&mut self, _start: usize) -> PResult<Node> {
        self.pos += 1;
        let caseless = self.flags.caseless;
        match self.peek() {
            Some(open @ (b'<' | b'\'')) => {
                self.pos += 1;
                let close = if open == b'<' { b'>' } else { b'\'' };
                let target = self.ref_body(close, 142)?;
                Ok(Node::Call { target, offset: self.pos })
            }
            Some(b'{') => {
                self.pos += 1;
                let s = self.rest();
                if s.first().is_some_and(|c| c.is_ascii_digit() || *c == b'-' || *c == b'+') {
                    let n = self.signed_number(157)?;
                    if !self.eat(b'}') {
                        return Err(err(157, self.pos));
                    }
                    let target = self.relative(n)?;
                    return Ok(Node::Backref { target, caseless, offset: self.pos });
                }
                let name = self.group_name(b'}', 157)?;
                Ok(Node::Backref { target: Ref::Name(name), caseless, offset: self.pos })
            }
            Some(c) if c.is_ascii_digit() || c == b'-' || c == b'+' => {
                let n = self.signed_number(157)?;
                let target = self.relative(n)?;
                Ok(Node::Backref { target, caseless, offset: self.pos })
            }
            _ => Err(err(157, self.pos)),
        }
    }

    /// A number with an optional sign, as (sign, value)
    fn signed_number(&mut self, code: u32) -> PResult<(i8, usize)> {
        let sign = match self.peek() {
            Some(b'-') => -1,
            Some(b'+') => 1,
            _ => 0,
        };
        if sign != 0 {
            self.pos += 1;
        }
        let start = self.pos;
        let mut n: usize = 0;
        while let Some(d @ b'0'..=b'9') = self.peek() {
            n = n.saturating_mul(10).saturating_add((d - b'0') as usize);
            self.pos += 1;
        }
        if self.pos == start {
            return Err(err(if sign != 0 { 129 } else { code }, self.pos));
        }
        if n > MAX_GROUPS {
            return Err(err(161, self.pos));
        }
        Ok((sign, n))
    }

    /// Resolves a relative group number against the groups opened so far
    fn relative(&self, (sign, n): (i8, usize)) -> PResult<Ref> {
        match sign {
            0 => Ok(Ref::Num(n)),
            _ if n == 0 => Err(err(126, self.pos)),
            -1 => {
                if n > self.groups {
                    return Err(err(115, self.pos));
                }
                Ok(Ref::Num(self.groups + 1 - n))
            }
            _ => Ok(Ref::Num(self.groups + n)),
        }
    }

    /// A group number (possibly relative) or a name, up to `close`
    fn ref_body(&mut self, close: u8, code: u32) -> PResult<Ref> {
        if self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'-' || c == b'+') {
            let n = self.signed_number(code)?;
            if !self.eat(close) {
                return Err(err(code, self.pos));
            }
            return self.relative(n);
        }
        Ok(Ref::Name(self.group_name(close, code)?))
    }

    /// A group name followed by `close`
    fn group_name(&mut self, close: u8, missing: u32) -> PResult<Vec<u8>> {
        let start = self.pos;
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            return Err(err(144, self.pos));
        }
        while self.peek().is_some_and(is_word) || self.utf && self.peek().is_some_and(|c| c >= 0x80) && {
            let (ch, _) = super::decode_utf8(self.rest());
            super::unicode::is_word(ch)
        } {
            if self.peek().unwrap() >= 0x80 {
                self.next_char();
            } else {
                self.pos += 1;
            }
        }
        if self.pos - start > MAX_NAME {
            return Err(err(148, self.pos));
        }
        if self.pos == start {
            if self.peek() == Some(close) {
                return Err(err(162, self.pos));
            }
            return Err(err(if missing == 142 { 162 } else { missing }, self.pos));
        }
        if !self.eat(close) {
            return Err(err(if missing == 157 || missing == 169 { missing } else { 142 }, self.pos));
        }
        Ok(self.p[start..self.pos - 1].to_vec())
    }

    fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > NEST_LIMIT {
            return Err(err(119, self.pos));
        }
        Ok(())
    }

    /// The body of a group up to its `)`, with option changes inside it undone after
    fn group_body(&mut self, reset: bool) -> PResult<Node> {
        self.enter()?;
        let saved = self.flags;
        let node = if reset { self.branch_reset()? } else { self.alternation()? };
        self.flags = saved;
        if !self.eat(b')') {
            return Err(err(114, self.pos));
        }
        self.depth -= 1;
        Ok(node)
    }

    fn new_group(&mut self) -> PResult<usize> {
        self.groups += 1;
        if self.groups > MAX_GROUPS {
            return Err(err(197, self.pos));
        }
        Ok(self.groups)
    }

    fn add_name(&mut self, name: Vec<u8>, index: usize, offset: usize) -> PResult<()> {
        for (n, i) in &self.names {
            if *n == name {
                if *i == index {
                    return Ok(());
                }
                if !self.flags.dupnames {
                    return Err(err(143, offset));
                }
            } else if *i == index {
                return Err(err(165, offset));
            }
        }
        self.names.push((name, index));
        Ok(())
    }

    fn capture(&mut self, name: Option<(Vec<u8>, usize)>) -> PResult<Node> {
        let index = self.new_group()?;
        if let Some((name, offset)) = name {
            self.add_name(name, index, offset)?;
        }
        let node = self.group_body(false)?;
        Ok(Node::Capture { index, node: Box::new(node) })
    }

    fn group(&mut self) -> PResult<Option<(Node, bool)>> {
        let start = self.pos;
        self.pos += 1;
        if self.eat(b'*') {
            return self.verb_or_alpha(start);
        }
        if !self.eat(b'?') {
            if self.flags.no_auto_capture {
                let node = self.group_body(false)?;
                return Ok(Some((Node::Group(Box::new(node)), true)));
            }
            return Ok(Some((self.capture(None)?, true)));
        }
        let Some(c) = self.peek() else { return Err(err(114, self.pos)) };
        let node = match c {
            b'#' => {
                while let Some(ch) = self.peek() {
                    self.pos += 1;
                    if ch == b')' {
                        self.transparent = true;
                        return Ok(None);
                    }
                }
                return Err(err(118, self.pos));
            }
            b':' => {
                self.pos += 1;
                Node::Group(Box::new(self.group_body(false)?))
            }
            b'|' => {
                self.pos += 1;
                Node::Group(Box::new(self.group_body(true)?))
            }
            b'>' => {
                self.pos += 1;
                Node::Atomic(Box::new(self.group_body(false)?))
            }
            // `(?*`: a non-atomic lookahead, treated as an ordinary one
            b'=' | b'!' | b'*' => {
                self.pos += 1;
                let node = self.group_body(false)?;
                return Ok(Some((Node::Look { behind: false, neg: c == b'!', node: Box::new(node), offset: start }, true)));
            }
            b'<' if matches!(self.at(1), Some(b'=' | b'!' | b'*')) => {
                let neg = self.at(1) == Some(b'!');
                self.pos += 2;
                let node = self.group_body(false)?;
                return Ok(Some((Node::Look { behind: true, neg, node: Box::new(node), offset: start }, true)));
            }
            b'<' | b'\'' => {
                self.pos += 1;
                let close = if c == b'<' { b'>' } else { b'\'' };
                let name = self.group_name(close, 142)?;
                let offset = self.pos;
                self.capture(Some((name, offset)))?
            }
            b'P' => {
                self.pos += 1;
                match self.peek() {
                    Some(b'<') => {
                        self.pos += 1;
                        let name = self.group_name(b'>', 142)?;
                        let offset = self.pos;
                        self.capture(Some((name, offset)))?
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        let offset = self.pos;
                        let name = self.group_name(b')', 142)?;
                        Node::Backref { target: Ref::Name(name), caseless: self.flags.caseless, offset }
                    }
                    Some(b'>') => {
                        self.pos += 1;
                        let offset = self.pos;
                        let name = self.group_name(b')', 142)?;
                        Node::Call { target: Ref::Name(name), offset }
                    }
                    None => return Err(err(114, self.pos)),
                    _ => return Err(err(141, self.pos)),
                }
            }
            b'&' => {
                self.pos += 1;
                let offset = self.pos;
                let name = self.group_name(b')', 142)?;
                Node::Call { target: Ref::Name(name), offset }
            }
            b'R' if self.at(1) == Some(b')') || self.at(1).is_none() || self.at(1) != Some(b')') => {
                self.pos += 1;
                if !self.eat(b')') {
                    return Err(err(158, self.pos));
                }
                Node::Call { target: Ref::Num(0), offset: self.pos }
            }
            b'0'..=b'9' | b'+' | b'-' if c != b'-' || self.at(1).is_some_and(|d| d.is_ascii_digit()) => {
                let n = self.signed_number(129)?;
                let offset = self.pos;
                if !self.eat(b')') {
                    return Err(err(115, self.pos));
                }
                let target = match n {
                    (0, n) => Ref::Num(n),
                    (_, 0) => return Err(err(126, offset)),
                    (-1, n) if n > self.groups => return Err(err(115, offset)),
                    (-1, n) => Ref::Num(self.groups + 1 - n),
                    (_, n) => Ref::Num(self.groups + n),
                };
                Node::Call { target, offset }
            }
            b'(' => return self.conditional(start).map(|n| Some((n, true))),
            b'C' => {
                // Callouts are accepted and do nothing
                while let Some(ch) = self.peek() {
                    self.pos += 1;
                    if ch == b')' {
                        return Ok(None);
                    }
                }
                return Err(err(139, self.pos));
            }
            _ => return self.options(),
        };
        Ok(Some((node, true)))
    }

    /// `(?i)`, `(?-s)`, `(?^x)`, or `(?i:...)`
    fn options(&mut self) -> PResult<Option<(Node, bool)>> {
        let mut f = self.flags;
        let mut on = true;
        if self.eat(b'^') {
            f.caseless = false;
            f.multiline = false;
            f.dotall = false;
            f.extended = false;
            f.extended_more = false;
            f.no_auto_capture = false;
            f.ungreedy = false;
        }
        loop {
            let Some(c) = self.peek() else { return Err(err(114, self.pos)) };
            self.pos += 1;
            match c {
                b'-' if on => on = false,
                b'i' => f.caseless = on,
                b'm' => f.multiline = on,
                b's' => f.dotall = on,
                b'n' => f.no_auto_capture = on,
                b'U' => f.ungreedy = on,
                b'J' => f.dupnames = on,
                b'x' => {
                    f.extended = on;
                    if self.eat(b'x') {
                        f.extended_more = on;
                    } else if !on {
                        f.extended_more = false;
                    }
                }
                b')' => {
                    self.flags = f;
                    return Ok(None);
                }
                b':' => {
                    let saved = self.flags;
                    self.flags = f;
                    let node = self.group_body(false);
                    self.flags = saved;
                    return Ok(Some((Node::Group(Box::new(node?)), true)));
                }
                _ => return Err(err(111, self.pos - 1)),
            }
        }
    }

    /// `(?(condition)yes|no)`
    fn conditional(&mut self, group_start: usize) -> PResult<Node> {
        self.enter()?;
        self.pos += 1;
        let cond_start = self.pos;
        let cond = if self.peek() == Some(b'?') || self.peek() == Some(b'*') {
            // An assertion condition
            self.pos -= 1;
            let Some((Node::Look { behind, neg, node, offset }, _)) = self.group()? else {
                return Err(err(128, cond_start));
            };
            Cond::Look { behind, neg, node, offset }
        } else {
            let s = self.rest();
            
            if s.first().is_some_and(|c| c.is_ascii_digit() || *c == b'+' || *c == b'-') {
                let n = self.signed_number(115)?;
                let r = self.relative(n)?;
                if !self.eat(b')') {
                    return Err(err(124, self.pos));
                }
                if let Ref::Num(i) = r
                    && i > self.total_groups.max(self.groups)
                {
                    return Err(err(115, cond_start - 1));
                }
                Cond::Group(r, cond_start - 1)
            } else if s.first() == Some(&b'<') || s.first() == Some(&b'\'') {
                let close = if s[0] == b'<' { b'>' } else { b'\'' };
                self.pos += 1;
                let name = self.group_name(close, 142)?;
                if !self.eat(b')') {
                    return Err(err(124, self.pos));
                }
                Cond::Group(Ref::Name(name), cond_start + 1)
            } else if s.starts_with(b"R&") {
                self.pos += 2;
                let name = self.group_name(b')', 142)?;
                Cond::Recursion(Some(Ref::Name(name)), cond_start)
            } else if s.first() == Some(&b'R') && s.get(1).is_some_and(|c| c.is_ascii_digit() || *c == b')') {
                self.pos += 1;
                let r = if self.peek() == Some(b')') {
                    None
                } else {
                    Some(Ref::Num(self.signed_number(115)?.1))
                };
                if !self.eat(b')') {
                    return Err(err(124, self.pos));
                }
                Cond::Recursion(r, cond_start)
            } else if s.starts_with(b"DEFINE)") {
                self.pos += 7;
                Cond::Define
            } else if s.starts_with(b"VERSION") {
                return Err(err(179, self.pos));
            } else {
                // A bare name
                let name = self.group_name(b')', 124)?;
                Cond::Group(Ref::Name(name), cond_start)
            }
        };
        let saved = self.flags;
        let mut branches = Vec::new();
        loop {
            branches.push(self.sequence()?);
            if !self.eat(b'|') {
                break;
            }
        }
        self.flags = saved;
        if !self.eat(b')') {
            return Err(err(114, self.pos));
        }
        self.depth -= 1;
        if matches!(cond, Cond::Define) && branches.len() > 1 {
            return Err(err(154, cond_start));
        }
        if branches.len() > 2 {
            return Err(err(127, group_start));
        }
        let no = if branches.len() == 2 { Some(Box::new(branches.pop().unwrap())) } else { None };
        let yes = Box::new(branches.pop().unwrap());
        Ok(Node::Cond { cond, yes, no })
    }

    /// `(*VERB)`, `(*VERB:NAME)` and `(*alpha_assertion:...)`
    fn verb_or_alpha(&mut self, _start: usize) -> PResult<Option<(Node, bool)>> {
        let name_start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') {
            self.pos += 1;
        }
        let word = self.p[name_start..self.pos].to_vec();
        if self.peek() == Some(b':') && !word.is_empty() {
            let look = match &word[..] {
                b"pla" | b"positive_lookahead" | b"napla" | b"non_atomic_positive_lookahead" => Some((false, false)),
                b"naplb" | b"non_atomic_positive_lookbehind" => Some((true, false)),
                b"nla" | b"negative_lookahead" => Some((false, true)),
                b"plb" | b"positive_lookbehind" => Some((true, false)),
                b"nlb" | b"negative_lookbehind" => Some((true, true)),
                _ => None,
            };
            if let Some((behind, neg)) = look {
                self.pos += 1;
                let node = self.group_body(false)?;
                return Ok(Some((Node::Look { behind, neg, node: Box::new(node), offset: name_start - 2 }, true)));
            }
            if matches!(&word[..], b"atomic") {
                self.pos += 1;
                let node = self.group_body(false)?;
                return Ok(Some((Node::Atomic(Box::new(node)), true)));
            }
        }
        let mut arg = if self.eat(b':') {
            let s = self.pos;
            while self.peek().is_some_and(|c| c != b')') {
                self.pos += 1;
            }
            Some(self.p[s..self.pos].to_vec())
        } else {
            None
        };
        if !self.eat(b')') {
            return Err(err(160, self.pos));
        }
        let verb = match &word[..] {
            b"FAIL" | b"F" => Verb::Fail,
            b"ACCEPT" => Verb::Accept,
            b"COMMIT" => Verb::Commit,
            b"PRUNE" => Verb::Prune,
            b"SKIP" if arg.is_none() => Verb::Skip,
            b"THEN" => Verb::Then,
            b"MARK" | b"" => match arg.take() {
                Some(name) if !name.is_empty() => Verb::Mark(name),
                _ => return Err(err(166, self.pos - 1)),
            },
            _ => return Err(err(160, self.pos - 1)),
        };
        // A name on another verb also sets the mark
        if let Some(name) = arg {
            if name.is_empty() {
                return Err(err(166, self.pos - 1));
            }
            return Ok(Some((Node::Seq(vec![Node::Verb(Verb::Mark(name)), Node::Verb(verb)]), false)));
        }
        Ok(Some((Node::Verb(verb), false)))
    }

    /// `\p` / `\P` after the letter: `\pL`, `\p{Lu}`, `\p{^Greek}`, `\p{sc:Latin}`
    fn property(&mut self) -> PResult<(Prop, bool)> {
        let Some(c) = self.peek() else { return Err(err(146, self.pos)) };
        let mut neg = false;
        let mut name = Vec::new();
        if c == b'{' {
            self.pos += 1;
            if self.eat(b'^') {
                neg = true;
            }
            loop {
                let Some(ch) = self.peek() else { return Err(err(146, self.pos)) };
                self.pos += 1;
                if ch == b'_' || ch == b'-' || ch.is_ascii_whitespace() {
                    continue;
                }
                if ch == b'}' {
                    break;
                }
                if ch == 0 || name.len() >= 49 {
                    return Err(err(146, self.pos));
                }
                name.push(ch.to_ascii_lowercase());
            }
        } else if c.is_ascii_alphabetic() {
            self.pos += 1;
            name.push(c.to_ascii_lowercase());
        } else {
            self.pos += 1;
            return Err(err(146, self.pos));
        }
        let mut script = None;
        if let Some(sep) = name.iter().position(|&c| c == b':' || c == b'=') {
            let (kind, value) = (&name[..sep], name[sep + 1..].to_vec());
            name = match kind {
                b"bidiclass" | b"bc" => [b"bidi".as_slice(), &value].concat(),
                b"script" | b"sc" => {
                    script = Some(PropType::Sc);
                    value
                }
                b"scriptextensions" | b"scx" => {
                    script = Some(PropType::Scx);
                    value
                }
                _ => return Err(err(147, self.pos)),
            };
        }
        let name = String::from_utf8_lossy(&name).into_owned();
        let Ok(i) = super::ucd::PROPERTIES.binary_search_by(|(n, _, _)| (*n).cmp(name.as_str())) else {
            return Err(err(147, self.pos));
        };
        let (_, ptype, value) = super::ucd::PROPERTIES[i];
        let ptype = match script {
            None => ptype,
            Some(want) => match ptype {
                PropType::Sc => PropType::Sc,
                PropType::Scx => want,
                _ => return Err(err(147, self.pos)),
            },
        };
        Ok((Prop { ptype, value }, neg))
    }

    /// A character class `[...]`
    fn class(&mut self) -> PResult<Node> {
        self.pos += 1;
        let mut negated = false;
        if self.eat(b'^') {
            negated = true;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { return Err(err(106, self.pos)) };
            if c == b']' && !first {
                self.pos += 1;
                break;
            }
            first = false;
            if self.flags.extended_more && (c == b' ' || c == b'\t') {
                self.pos += 1;
                continue;
            }
            // POSIX classes
            if c == b'[' && matches!(self.at(1), Some(b':' | b'.' | b'='))
                && let Some(end) = self.posix_end(self.pos + 1)
            {
                if self.at(1) != Some(b':') {
                    return Err(err(113, self.pos));
                }
                let mut name = &self.p[self.pos + 2..end];
                let neg = name.first() == Some(&b'^');
                if neg {
                    name = &name[1..];
                }
                let Some((_, kind)) = POSIX_NAMES.iter().find(|(n, _)| n.as_bytes() == name) else {
                    return Err(err(130, self.pos + 2 + usize::from(neg)));
                };
                items.push(ClassItem::Posix(*kind, neg));
                self.pos = end + 2;
                // Nor can a POSIX class
                if self.peek() == Some(b'-') && self.at(1).is_some_and(|c| c != b']') {
                    return Err(err(150, self.pos));
                }
                continue;
            }
            let lo_start = self.pos;
            let lo = match self.class_atom()? {
                ClassAtom::Char(c) => c,
                ClassAtom::Item(item) => {
                    items.push(item);
                    // A class escape cannot start a range
                    if self.peek() == Some(b'-') && self.at(1).is_some_and(|c| c != b']') {
                        return Err(err(150, self.pos));
                    }
                    continue;
                }
                ClassAtom::Nothing => continue,
            };
            // A range?
            if self.peek() == Some(b'-') && self.at(1).is_some_and(|c| c != b']') {
                let save = self.pos;
                self.pos += 1;
                // A range cannot end with a POSIX class
                if self.peek() == Some(b'[')
                    && matches!(self.at(1), Some(b':' | b'.' | b'='))
                    && self.posix_end(self.pos + 1).is_some()
                {
                    return Err(err(150, self.pos + 1));
                }
                let hi_start = self.pos;
                let is_prop = self.rest().starts_with(b"\\p") || self.rest().starts_with(b"\\P");
                match self.class_atom()? {
                    ClassAtom::Char(hi) => {
                        if hi < lo {
                            return Err(err(108, hi_start));
                        }
                        items.push(ClassItem::Range(lo, hi));
                        continue;
                    }
                    ClassAtom::Item(_) => return Err(err(150, if is_prop { hi_start + 2 } else { self.pos })),
                    ClassAtom::Nothing => {
                        self.pos = save;
                    }
                }
            }
            let _ = lo_start;
            items.push(ClassItem::Range(lo, lo));
        }
        Ok(Node::Class(Box::new(Class { negated, caseless: self.flags.caseless, items })))
    }

    fn class_atom(&mut self) -> PResult<ClassAtom> {
        let c = self.peek().unwrap();
        if c != b'\\' {
            return Ok(ClassAtom::Char(self.next_char()));
        }
        self.pos += 1;
        let Some(e) = self.peek() else { return Err(err(101, self.pos)) };
        Ok(match e {
            b'd' | b'D' | b's' | b'S' | b'w' | b'W' | b'h' | b'H' | b'v' | b'V' => {
                self.pos += 1;
                let (b, neg) = builtin(e);
                ClassAtom::Item(ClassItem::Builtin(b, neg))
            }
            b'p' | b'P' => {
                self.pos += 1;
                let (prop, neg) = self.property()?;
                ClassAtom::Item(ClassItem::Prop(prop, neg != (e == b'P')))
            }
            b'Q' => {
                self.pos += 1;
                if self.rest().starts_with(b"\\E") {
                    self.pos += 2;
                    return Ok(ClassAtom::Nothing);
                }
                // Inside a class \Q only quotes up to \E; treat the next character literally
                let ch = self.next_char();
                if self.rest().starts_with(b"\\E") {
                    self.pos += 2;
                }
                ClassAtom::Char(ch)
            }
            b'E' => {
                self.pos += 1;
                ClassAtom::Nothing
            }
            b'N' => return Err(err(171, self.pos + 1)),
            b'R' | b'X' | b'B' | b'A' | b'z' | b'Z' | b'G' | b'K' | b'g' | b'k' | b'C' => {
                return Err(err(107, self.pos));
            }
            _ => {
                let v = self.char_escape(true)?;
                if !self.utf && v > 0xff {
                    return Err(err(151, self.pos));
                }
                ClassAtom::Char(v)
            }
        })
    }
}

enum ClassAtom {
    Char(u32),
    Item(ClassItem),
    /// `\E` and empty `\Q\E`
    Nothing,
}

fn builtin(c: u8) -> (Builtin, bool) {
    let b = match c.to_ascii_lowercase() {
        b'd' => Builtin::Digit,
        b's' => Builtin::Space,
        b'w' => Builtin::Word,
        b'h' => Builtin::HSpace,
        _ => Builtin::VSpace,
    };
    (b, c.is_ascii_uppercase())
}

/// Checks that referenced groups exist
fn check_refs(node: &Node, parsed: &Parsed) -> PResult<()> {
    let check = |r: &Ref, offset: usize| -> PResult<()> {
        match r {
            Ref::Num(n) if *n > parsed.groups => Err(err(115, offset)),
            Ref::Name(name) if !parsed.names.iter().any(|(n, _)| n == name) => Err(err(115, offset)),
            _ => Ok(()),
        }
    };
    match node {
        Node::Seq(v) | Node::Alt(v) => v.iter().try_for_each(|n| check_refs(n, parsed)),
        Node::Capture { node, .. }
        | Node::Group(node)
        | Node::Atomic(node)
        | Node::Look { node, .. }
        | Node::Repeat { node, .. } => check_refs(node, parsed),
        Node::Backref { target, offset, .. } => {
            if let Ref::Num(0) = target {
                return Err(err(115, *offset));
            }
            check(target, *offset)
        }
        Node::Call { target, offset } => check(target, *offset),
        Node::Cond { cond, yes, no } => {
            match cond {
                Cond::Group(r, offset) => check(r, *offset)?,
                Cond::Recursion(Some(r), offset) => check(r, *offset)?,
                Cond::Look { node, .. } => check_refs(node, parsed)?,
                _ => {}
            }
            check_refs(yes, parsed)?;
            if let Some(no) = no {
                check_refs(no, parsed)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The number of characters a node always matches, if fixed
pub fn fixed_length(node: &Node, parsed: &Parsed) -> Option<usize> {
    fixed_len_at(node, parsed, 0)
}

fn fixed_len_at(node: &Node, parsed: &Parsed, depth: usize) -> Option<usize> {
    if depth > 20 {
        return None;
    }
    match node {
        Node::Empty | Node::Assert(_) | Node::Look { .. } | Node::Verb(_) | Node::Keep => Some(0),
        Node::Char { .. } | Node::Any { .. } | Node::Class(_) | Node::AnyByte => Some(1),
        Node::Seq(v) => v.iter().try_fold(0usize, |acc, n| Some(acc + fixed_len_at(n, parsed, depth)?)),
        Node::Alt(v) => {
            let first = fixed_len_at(&v[0], parsed, depth)?;
            v[1..].iter().all(|n| fixed_len_at(n, parsed, depth) == Some(first)).then_some(first)
        }
        Node::Capture { node, .. } | Node::Group(node) | Node::Atomic(node) => fixed_len_at(node, parsed, depth),
        Node::Repeat { node, min, max: Some(max), .. } if min == max => {
            Some(fixed_len_at(node, parsed, depth)? * *min as usize)
        }
        Node::Call { target, .. } => {
            let n = match target {
                Ref::Num(n) => *n,
                Ref::Name(name) => parsed.names.iter().find(|(n, _)| n == name)?.1,
            };
            if n == 0 {
                return None;
            }
            let group = find_group(&parsed.node, n)?;
            fixed_len_at(group, parsed, depth + 1)
        }
        Node::Cond { cond: Cond::Define, .. } => Some(0),
        Node::Cond { yes, no, .. } => {
            let y = fixed_len_at(yes, parsed, depth)?;
            let n = match no {
                Some(n) => fixed_len_at(n, parsed, depth)?,
                None => 0,
            };
            (y == n).then_some(y)
        }
        _ => None,
    }
}

pub fn find_group(node: &Node, n: usize) -> Option<&Node> {
    match node {
        Node::Capture { index, node } if *index == n => Some(node),
        Node::Seq(v) | Node::Alt(v) => v.iter().find_map(|c| find_group(c, n)),
        Node::Capture { node, .. }
        | Node::Group(node)
        | Node::Atomic(node)
        | Node::Look { node, .. }
        | Node::Repeat { node, .. } => find_group(node, n),
        Node::Cond { cond, yes, no } => {
            if let Cond::Look { node, .. } = cond
                && let Some(g) = find_group(node, n)
            {
                return Some(g);
            }
            find_group(yes, n).or_else(|| no.as_ref().and_then(|no| find_group(no, n)))
        }
        _ => None,
    }
}

/// Every lookbehind's branches must each have a fixed length
fn check_lookbehinds(parsed: &mut Parsed) -> PResult<()> {
    fn walk(node: &Node, parsed: &Parsed) -> PResult<()> {
        match node {
            Node::Look { behind: true, node: body, offset, .. } => {
                let branches: Vec<&Node> = match &**body {
                    Node::Alt(v) => v.iter().collect(),
                    other => vec![other],
                };
                for b in branches {
                    if fixed_length(b, parsed).is_none() {
                        return Err(err(125, *offset));
                    }
                }
                walk(body, parsed)
            }
            Node::Cond { cond, yes, no } => {
                if let Cond::Look { behind: true, node: body, offset, .. } = cond {
                    walk(&Node::Look { behind: true, neg: false, node: body.clone(), offset: *offset }, parsed)?;
                } else if let Cond::Look { node: body, .. } = cond {
                    walk(body, parsed)?;
                }
                walk(yes, parsed)?;
                if let Some(no) = no {
                    walk(no, parsed)?;
                }
                Ok(())
            }
            Node::Seq(v) | Node::Alt(v) => v.iter().try_for_each(|n| walk(n, parsed)),
            Node::Capture { node, .. }
            | Node::Group(node)
            | Node::Atomic(node)
            | Node::Look { node, .. }
            | Node::Repeat { node, .. } => walk(node, parsed),
            _ => Ok(()),
        }
    }
    walk(&parsed.node, parsed)
}
