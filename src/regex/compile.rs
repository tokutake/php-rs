//! Turns the parsed tree into instructions for the matcher, and works out the
//! start-of-match optimizations (anchoring, possible first bytes, a byte that
//! every match must contain).

use super::parse::{Assertion, Builtin, Class, ClassItem, Cond, Node, Parsed, Posix, Ref, RepKind, Verb};
use super::unicode;
use super::{CompileError, Newline, Options};

/// Instructions beyond this make the pattern "too large"
const MAX_PROGRAM: usize = 1 << 20;

/// Something that matches exactly one character (one byte outside UTF mode)
#[derive(Clone, Copy, Debug)]
pub enum Unit {
    Byte(u8),
    /// An ASCII letter either way round
    ByteFold(u8, u8),
    /// A multi-byte UTF-8 character
    Char([u8; 4], u8),
    /// A character caselessly, in UTF mode
    CharFold(u32),
    /// `.`: anything but a newline
    AnyNotNl,
    /// `.` with /s, and `\C` (one byte)
    Any,
    AnyByte,
    Class(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerbKind {
    Commit,
    Prune,
    Skip,
    Then,
}

#[derive(Clone, Copy, Debug)]
pub enum SubKind {
    Atomic,
    Ahead { neg: bool },
    /// The condition of a conditional group: continue after the body if it
    /// holds (is false for `neg`), else at `no`
    Cond { neg: bool, no: usize },
}

#[derive(Clone, Debug)]
pub enum Inst {
    Unit(Unit),
    Lit(Box<[u8]>),
    Rep { unit: Unit, min: u32, max: u32, kind: RepKind },
    /// Try the first target, then on backtracking the second
    Split(usize, usize),
    Jmp(usize),
    Save(usize),
    Assert(Assertion),
    Backref { groups: Box<[usize]>, caseless: bool },
    /// Remember where an iteration of a loop started
    LoopStart(usize),
    /// After an iteration: an empty one leaves the loop (at `exit`), else go `back`
    LoopCheck { reg: usize, exit: usize, back: usize },
    /// A nested match of the body (the next instructions up to `end`, a SubEnd)
    Sub { kind: SubKind, end: usize },
    /// Lookbehind: one fixed-length body per branch, each ending in SubEnd;
    /// `cond_no` makes it a condition, `next` is where matching continues
    LookBehind { neg: bool, branches: Box<[(usize, usize)]>, next: usize, cond_no: Option<usize> },
    SubEnd,
    /// `(?(1)...)`: any of the groups set
    CondGroup { groups: Box<[usize]>, no: usize },
    /// `(?(R)...)`, `(?(R1)...)`
    CondRecursion { group: Option<usize>, no: usize },
    Call { target: usize, group: usize },
    /// The end of a group that is called: return if this call is for it
    GroupEnd(usize),
    Keep,
    Newline,
    ExtGrapheme,
    Fail,
    Accept,
    Verb(VerbKind),
    Mark(usize),
    Match,
}

/// A compiled class: membership of the first 256 (UTF: 128) characters as a
/// bitmap, the items for the rest
#[derive(Debug)]
pub struct CClass {
    pub bitmap: [u64; 4],
    pub limit: u32,
    pub class: Class,
    pub utf: bool,
    pub ucp: bool,
}

impl CClass {
    #[inline]
    pub fn matches(&self, c: u32) -> bool {
        if c < self.limit {
            return self.bitmap[(c >> 6) as usize] & (1 << (c & 63)) != 0;
        }
        class_matches(&self.class, c, self.utf, self.ucp)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    None,
    /// Only at the start of the subject
    Start,
    /// Only at the start offset
    StartOffset,
    /// At the start or after a newline (a leading `.*`)
    Line,
}

pub struct Regex {
    pub(super) prog: Vec<Inst>,
    pub(super) classes: Vec<CClass>,
    pub(super) marks: Vec<Vec<u8>>,
    /// Number of capturing groups
    pub capture_count: usize,
    /// Group names and their numbers, in pattern order
    pub names: Vec<(Vec<u8>, usize)>,
    pub(super) utf: bool,
    pub(super) ucp: bool,
    pub(super) newline: Newline,
    pub(super) bsr_anycrlf: bool,
    pub(super) anchor: Anchor,
    /// Bytes a match can start with, if known
    pub(super) first: Option<Box<[bool; 256]>>,
    /// A byte every match contains
    pub(super) required: Option<u8>,
    pub(super) regs: usize,
    pub(super) notempty: bool,
    pub(super) notempty_atstart: bool,
}

impl Regex {
    pub fn is_utf(&self) -> bool {
        self.utf
    }
}

struct Compiler<'a> {
    prog: Vec<Inst>,
    classes: Vec<CClass>,
    marks: Vec<Vec<u8>>,
    parsed: &'a Parsed,
    /// Groups that are called (recursion or subroutines), and where each starts
    called: Vec<bool>,
    group_start: Vec<Option<usize>>,
    regs: usize,
}

pub fn compile(parsed: Parsed, opts: &Options, pattern_len: usize) -> Result<Regex, CompileError> {
    let mut called = vec![false; parsed.groups + 1];
    mark_called(&parsed.node, &parsed, &mut called);
    let mut c = Compiler {
        prog: Vec::new(),
        classes: Vec::new(),
        marks: Vec::new(),
        parsed: &parsed,
        called,
        group_start: vec![None; parsed.groups + 1],
        regs: 0,
    };
    c.prog.push(Inst::Save(0));
    c.group_start[0] = Some(1);
    c.emit(&parsed.node)?;
    if c.called[0] {
        c.prog.push(Inst::GroupEnd(0));
    }
    c.prog.push(Inst::Save(1));
    c.prog.push(Inst::Match);
    if c.prog.len() > MAX_PROGRAM {
        return Err(CompileError { code: 120, offset: pattern_len });
    }
    auto_possessify(&mut c.prog, &c.classes, parsed.utf);
    // Calls jump to the start of their group
    for i in 0..c.prog.len() {
        if let Inst::Call { group, .. } = c.prog[i] {
            let target = c.group_start[group].expect("called groups are compiled");
            c.prog[i] = Inst::Call { target, group };
        }
    }
    let utf = parsed.utf;
    // /A anchors at the start offset, like \G
    let anchor = if opts.anchored { Anchor::StartOffset } else { anchor_of(&parsed.node) };
    let first = if parsed.no_start_optimize || anchor != Anchor::None {
        None
    } else {
        let mut set = Box::new([false; 256]);
        match first_bytes(&parsed.node, &c, &mut set, utf, 0) {
            Some(false) if set.iter().any(|&b| !b) => Some(set),
            _ => None,
        }
    };
    let required = if parsed.no_start_optimize { None } else { required_byte(&parsed.node, utf) };
    Ok(Regex {
        prog: c.prog,
        classes: c.classes,
        marks: c.marks,
        capture_count: parsed.groups,
        names: parsed.names.clone(),
        utf,
        ucp: parsed.ucp,
        newline: parsed.newline,
        bsr_anycrlf: parsed.bsr_anycrlf,
        anchor,
        first,
        required,
        regs: c.regs,
        notempty: parsed.notempty,
        notempty_atstart: parsed.notempty_atstart,
    })
}

fn resolve(parsed: &Parsed, r: &Ref) -> Vec<usize> {
    match r {
        Ref::Num(n) => vec![*n],
        Ref::Name(name) => parsed.names.iter().filter(|(n, _)| n == name).map(|(_, i)| *i).collect(),
    }
}

fn mark_called(node: &Node, parsed: &Parsed, called: &mut [bool]) {
    match node {
        Node::Call { target, .. } => {
            if let Some(&n) = resolve(parsed, target).first() {
                called[n] = true;
            }
        }
        Node::Seq(v) | Node::Alt(v) => v.iter().for_each(|n| mark_called(n, parsed, called)),
        Node::Capture { node, .. }
        | Node::Group(node)
        | Node::Atomic(node)
        | Node::Look { node, .. }
        | Node::Repeat { node, .. } => mark_called(node, parsed, called),
        Node::Cond { cond, yes, no } => {
            if let Cond::Look { node, .. } = cond {
                mark_called(node, parsed, called);
            }
            mark_called(yes, parsed, called);
            if let Some(no) = no {
                mark_called(no, parsed, called);
            }
        }
        _ => {}
    }
}

/// Whether a node can match the empty string
fn nullable(node: &Node) -> bool {
    match node {
        Node::Empty | Node::Assert(_) | Node::Look { .. } | Node::Keep | Node::Verb(_) => true,
        Node::Char { .. } | Node::Any { .. } | Node::Class(_) | Node::AnyByte | Node::Newline | Node::ExtGrapheme => false,
        Node::Seq(v) => v.iter().all(nullable),
        Node::Alt(v) => v.iter().any(nullable),
        Node::Capture { node, .. } | Node::Group(node) | Node::Atomic(node) => nullable(node),
        Node::Repeat { node, min, .. } => *min == 0 || nullable(node),
        // Unknown: a back reference can be to an empty group, a call can recurse
        Node::Backref { .. } | Node::Call { .. } | Node::Cond { .. } => true,
    }
}

impl Compiler<'_> {
    fn pc(&self) -> usize {
        self.prog.len()
    }

    fn push(&mut self, inst: Inst) -> Result<usize, CompileError> {
        if self.prog.len() >= MAX_PROGRAM {
            return Err(CompileError { code: 120, offset: 0 });
        }
        self.prog.push(inst);
        Ok(self.prog.len() - 1)
    }

    fn class(&mut self, class: &Class) -> u32 {
        let utf = self.parsed.utf;
        let ucp = self.parsed.ucp;
        let limit = if utf { 128 } else { 256 };
        let mut bitmap = [0u64; 4];
        for c in 0..limit {
            if class_matches(class, c, utf, ucp) {
                bitmap[(c >> 6) as usize] |= 1 << (c & 63);
            }
        }
        self.classes.push(CClass { bitmap, limit, class: class.clone(), utf, ucp });
        (self.classes.len() - 1) as u32
    }

    /// The single-character matcher for a node, if it is one
    fn unit(&mut self, node: &Node) -> Option<Unit> {
        let utf = self.parsed.utf;
        Some(match node {
            Node::Char { c, caseless } => {
                let c = *c;
                if *caseless {
                    if utf && c >= 128 {
                        if unicode::other_case(c) == c && unicode::caseless_set(c).is_none() {
                            return Some(char_unit(c));
                        }
                        return Some(Unit::CharFold(c));
                    }
                    if utf && c < 128 && (c as u8).is_ascii_alphabetic() {
                        // In UTF mode, k also matches the Kelvin sign and s the long s
                        if unicode::caseless_set(c).is_some() {
                            return Some(Unit::CharFold(c));
                        }
                    }
                    let b = c as u8;
                    if b.is_ascii_alphabetic() {
                        return Some(Unit::ByteFold(b.to_ascii_lowercase(), b.to_ascii_uppercase()));
                    }
                }
                if utf && c >= 128 {
                    char_unit(c)
                } else {
                    Unit::Byte(c as u8)
                }
            }
            Node::Any { dotall } => {
                if *dotall {
                    Unit::Any
                } else {
                    Unit::AnyNotNl
                }
            }
            Node::AnyByte => Unit::AnyByte,
            Node::Class(class) => Unit::Class(self.class(class)),
            _ => return None,
        })
    }

    fn emit(&mut self, node: &Node) -> Result<(), CompileError> {
        match node {
            Node::Empty => {}
            Node::Char { .. } | Node::Any { .. } | Node::AnyByte | Node::Class(_) => {
                let unit = self.unit(node).unwrap();
                self.push(Inst::Unit(unit))?;
            }
            Node::Seq(items) => {
                let mut i = 0;
                while i < items.len() {
                    // Runs of plain characters become one literal
                    let mut lit = Vec::new();
                    while let Some(Node::Char { c, caseless }) = items.get(i) {
                        let folds = *caseless
                            && ((*c < 128 && (*c as u8).is_ascii_alphabetic())
                                || self.parsed.utf
                                    && *c >= 128
                                    && (unicode::other_case(*c) != *c || unicode::caseless_set(*c).is_some()));
                        if folds {
                            break;
                        }
                        if self.parsed.utf {
                            super::encode_utf8(*c, &mut lit);
                        } else {
                            lit.push(*c as u8);
                        }
                        i += 1;
                    }
                    match lit.len() {
                        0 => {
                            self.emit(&items[i])?;
                            i += 1;
                        }
                        1 => {
                            self.push(Inst::Unit(Unit::Byte(lit[0])))?;
                        }
                        _ => {
                            self.push(Inst::Lit(lit.into_boxed_slice()))?;
                        }
                    }
                }
            }
            Node::Alt(branches) => {
                let mut jumps = Vec::new();
                for (i, b) in branches.iter().enumerate() {
                    if i + 1 < branches.len() {
                        let split = self.push(Inst::Split(0, 0))?;
                        self.emit(b)?;
                        jumps.push(self.push(Inst::Jmp(0))?);
                        let next = self.pc();
                        self.prog[split] = Inst::Split(split + 1, next);
                    } else {
                        self.emit(b)?;
                    }
                }
                let end = self.pc();
                for j in jumps {
                    self.prog[j] = Inst::Jmp(end);
                }
            }
            Node::Capture { index, node } => {
                let start = self.push(Inst::Save(index * 2))?;
                if self.group_start[*index].is_none() {
                    self.group_start[*index] = Some(start);
                }
                self.emit(node)?;
                self.push(Inst::Save(index * 2 + 1))?;
                if self.called[*index] {
                    self.push(Inst::GroupEnd(*index))?;
                }
            }
            Node::Group(node) => self.emit(node)?,
            Node::Atomic(node) => {
                self.sub(SubKind::Atomic, node)?;
            }
            Node::Look { behind: false, neg, node, .. } => {
                self.sub(SubKind::Ahead { neg: *neg }, node)?;
            }
            Node::Look { behind: true, neg, node, .. } => {
                self.lookbehind(*neg, node, false)?;
            }
            Node::Repeat { node, min, max, kind } => self.repeat(node, *min, *max, *kind)?,
            Node::Assert(a) => {
                self.push(Inst::Assert(*a))?;
            }
            Node::Backref { target, caseless, .. } => {
                let groups = resolve(self.parsed, target).into_boxed_slice();
                self.push(Inst::Backref { groups, caseless: *caseless })?;
            }
            Node::Call { target, .. } => {
                let group = resolve(self.parsed, target)[0];
                self.push(Inst::Call { target: 0, group })?;
            }
            Node::Cond { cond, yes, no } => self.cond(cond, yes, no.as_deref())?,
            Node::Keep => {
                self.push(Inst::Keep)?;
            }
            Node::Newline => {
                self.push(Inst::Newline)?;
            }
            Node::ExtGrapheme => {
                self.push(Inst::ExtGrapheme)?;
            }
            Node::Verb(v) => {
                let inst = match v {
                    Verb::Fail => Inst::Fail,
                    Verb::Accept => Inst::Accept,
                    Verb::Commit => Inst::Verb(VerbKind::Commit),
                    Verb::Prune => Inst::Verb(VerbKind::Prune),
                    Verb::Skip => Inst::Verb(VerbKind::Skip),
                    Verb::Then => Inst::Verb(VerbKind::Then),
                    Verb::Mark(name) => {
                        self.marks.push(name.clone());
                        Inst::Mark(self.marks.len() - 1)
                    }
                };
                self.push(inst)?;
            }
        }
        Ok(())
    }

    fn sub(&mut self, kind: SubKind, node: &Node) -> Result<usize, CompileError> {
        let at = self.push(Inst::Sub { kind, end: 0 })?;
        self.emit(node)?;
        let end = self.push(Inst::SubEnd)?;
        let kind = match self.prog[at] {
            Inst::Sub { kind, .. } => kind,
            _ => unreachable!(),
        };
        self.prog[at] = Inst::Sub { kind, end };
        Ok(at)
    }

    fn lookbehind(&mut self, neg: bool, body: &Node, cond: bool) -> Result<usize, CompileError> {
        let branches: Vec<&Node> = match body {
            Node::Alt(v) => v.iter().collect(),
            other => vec![other],
        };
        let at = self.push(Inst::LookBehind { neg, branches: Box::new([]), next: 0, cond_no: None })?;
        let mut starts = Vec::new();
        for b in branches {
            let len = super::parse::fixed_length(b, self.parsed).unwrap_or(0);
            starts.push((len, self.pc()));
            self.emit(b)?;
            self.push(Inst::SubEnd)?;
        }
        let next = self.pc();
        self.prog[at] =
            Inst::LookBehind { neg, branches: starts.into_boxed_slice(), next, cond_no: cond.then_some(0) };
        Ok(at)
    }

    fn cond(&mut self, cond: &Cond, yes: &Node, no: Option<&Node>) -> Result<(), CompileError> {
        // The test, with its "no" target patched once known
        let test = match cond {
            Cond::Define => {
                // Never matched directly; its groups are there to be called
                let jmp = self.push(Inst::Jmp(0))?;
                self.emit(yes)?;
                let end = self.pc();
                self.prog[jmp] = Inst::Jmp(end);
                return Ok(());
            }
            Cond::Group(r, _) => {
                let groups = resolve(self.parsed, r).into_boxed_slice();
                self.push(Inst::CondGroup { groups, no: 0 })?
            }
            Cond::Recursion(r, _) => {
                let group = r.as_ref().map(|r| resolve(self.parsed, r)[0]);
                self.push(Inst::CondRecursion { group, no: 0 })?
            }
            Cond::Look { behind: false, neg, node, .. } => self.sub(SubKind::Cond { neg: *neg, no: 0 }, node)?,
            Cond::Look { behind: true, neg, node, .. } => self.lookbehind(*neg, node, true)?,
        };
        self.emit(yes)?;
        let jmp = self.push(Inst::Jmp(0))?;
        let no_pc = self.pc();
        if let Some(no) = no {
            self.emit(no)?;
        }
        let end = self.pc();
        self.prog[jmp] = Inst::Jmp(end);
        match &mut self.prog[test] {
            Inst::CondGroup { no, .. } | Inst::CondRecursion { no, .. } => *no = no_pc,
            Inst::Sub { kind: SubKind::Cond { no, .. }, .. } => *no = no_pc,
            Inst::LookBehind { cond_no, .. } => *cond_no = Some(no_pc),
            _ => unreachable!(),
        }
        Ok(())
    }

    fn repeat(&mut self, node: &Node, min: u32, max: Option<u32>, kind: RepKind) -> Result<(), CompileError> {
        if let Some(unit) = self.unit(node) {
            self.push(Inst::Rep { unit, min, max: max.unwrap_or(u32::MAX), kind })?;
            return Ok(());
        }
        if kind == RepKind::Possessive {
            let inner = Node::Repeat { node: Box::new(node.clone()), min, max, kind: RepKind::Greedy };
            self.sub(SubKind::Atomic, &inner)?;
            return Ok(());
        }
        let greedy = kind == RepKind::Greedy;
        let empty_check = max.is_none() && nullable(node);
        // An unbounded repeat stops after an iteration that matched nothing,
        // the last required one included
        let mut first_check = None;
        for i in 0..min {
            if empty_check && i + 1 == min {
                let r = self.regs;
                self.regs += 1;
                self.push(Inst::LoopStart(r))?;
                self.emit(node)?;
                first_check = Some((r, self.push(Inst::Jmp(0))?));
            } else {
                self.emit(node)?;
            }
        }
        match max {
            None => {
                // loop: Split(body, exit); body; back to loop
                let split = self.push(Inst::Split(0, 0))?;
                let reg = if empty_check {
                    let r = self.regs;
                    self.regs += 1;
                    self.push(Inst::LoopStart(r))?;
                    Some(r)
                } else {
                    None
                };
                self.emit(node)?;
                let back = self.push(Inst::Jmp(split))?;
                let exit = self.pc();
                if let Some(reg) = reg {
                    self.prog[back] = Inst::LoopCheck { reg, exit, back: split };
                }
                if let Some((reg, at)) = first_check {
                    self.prog[at] = Inst::LoopCheck { reg, exit, back: split };
                }
                self.prog[split] = if greedy { Inst::Split(split + 1, exit) } else { Inst::Split(exit, split + 1) };
            }
            Some(max) => {
                let mut splits = Vec::new();
                for _ in min..max {
                    splits.push(self.push(Inst::Split(0, 0))?);
                    self.emit(node)?;
                }
                let exit = self.pc();
                for s in splits {
                    self.prog[s] = if greedy { Inst::Split(s + 1, exit) } else { Inst::Split(exit, s + 1) };
                }
            }
        }
        Ok(())
    }
}

fn char_unit(c: u32) -> Unit {
    let mut buf = Vec::with_capacity(4);
    super::encode_utf8(c, &mut buf);
    let mut bytes = [0; 4];
    bytes[..buf.len()].copy_from_slice(&buf);
    Unit::Char(bytes, buf.len() as u8)
}

fn ascii_space(c: u32) -> bool {
    matches!(c, 0x09..=0x0d | 0x20)
}

fn builtin_matches(b: Builtin, c: u32, ucp: bool) -> bool {
    match b {
        Builtin::Digit => {
            if ucp {
                unicode::chartype(c) == unicode::ND
            } else {
                (b'0' as u32..=b'9' as u32).contains(&c)
            }
        }
        Builtin::Space => {
            if ucp {
                unicode::is_hspace(c) || unicode::is_vspace(c) || unicode::category(c) == unicode::GC_Z
            } else {
                ascii_space(c)
            }
        }
        Builtin::Word => {
            if ucp {
                unicode::is_word(c)
            } else {
                c < 128 && ((c as u8).is_ascii_alphanumeric() || c == b'_' as u32)
            }
        }
        Builtin::HSpace => unicode::is_hspace(c),
        Builtin::VSpace => unicode::is_vspace(c),
    }
}

fn posix_matches(p: Posix, c: u32, ucp: bool) -> bool {
    if ucp {
        match p {
            Posix::Alpha => return unicode::category(c) == unicode::GC_L,
            Posix::Lower => return unicode::chartype(c) == unicode::LL,
            Posix::Upper => return unicode::chartype(c) == unicode::LU,
            Posix::Alnum => return matches!(unicode::category(c), unicode::GC_L | unicode::GC_N),
            Posix::Blank => return unicode::is_hspace(c),
            Posix::Cntrl => return unicode::chartype(c) == unicode::CC,
            Posix::Digit => return unicode::chartype(c) == unicode::ND,
            Posix::Graph => return unicode::is_graph(c),
            Posix::Print => return unicode::is_print(c),
            Posix::Punct => return unicode::is_punct(c),
            Posix::Space => return unicode::is_hspace(c) || unicode::is_vspace(c) || unicode::category(c) == unicode::GC_Z,
            Posix::Word => return unicode::is_word(c),
            Posix::Ascii | Posix::Xdigit => {}
        }
    }
    if c >= 128 {
        return false;
    }
    let b = c as u8;
    match p {
        Posix::Alpha => b.is_ascii_alphabetic(),
        Posix::Lower => b.is_ascii_lowercase(),
        Posix::Upper => b.is_ascii_uppercase(),
        Posix::Alnum => b.is_ascii_alphanumeric(),
        Posix::Ascii => true,
        Posix::Blank => b == b' ' || b == b'\t',
        Posix::Cntrl => b < 32 || b == 127,
        Posix::Digit => b.is_ascii_digit(),
        Posix::Graph => b.is_ascii_graphic(),
        Posix::Print => b.is_ascii_graphic() || b == b' ',
        Posix::Punct => b.is_ascii_punctuation(),
        Posix::Space => ascii_space(c),
        Posix::Word => b.is_ascii_alphanumeric() || b == b'_',
        Posix::Xdigit => b.is_ascii_hexdigit(),
    }
}

fn in_ranges(class: &Class, c: u32) -> bool {
    class.items.iter().any(|it| matches!(it, ClassItem::Range(lo, hi) if *lo <= c && c <= *hi))
}

pub fn class_matches(class: &Class, c: u32, utf: bool, ucp: bool) -> bool {
    let mut hit = class.items.iter().any(|it| match it {
        ClassItem::Range(lo, hi) => *lo <= c && c <= *hi,
        ClassItem::Builtin(b, neg) => builtin_matches(*b, c, ucp) != *neg,
        ClassItem::Prop(p, neg) => unicode::has_prop(c, *p) != *neg,
        ClassItem::Posix(p, neg) => {
            let m = posix_matches(*p, c, ucp)
                // Caseless [:lower:] and [:upper:] match letters of both cases
                || class.caseless && !neg && matches!(p, Posix::Lower | Posix::Upper) && posix_matches(Posix::Alpha, c, ucp) && {
                    if ucp { matches!(unicode::chartype(c), unicode::LL | unicode::LU | unicode::LT) } else { true }
                };
            m != *neg
        }
    });
    if !hit && class.caseless {
        if utf {
            let other = unicode::other_case(c);
            hit = other != c && in_ranges(class, other)
                || unicode::caseless_set(c).is_some_and(|set| set.iter().any(|&o| o != c && in_ranges(class, o)));
        } else if c < 128 && (c as u8).is_ascii_alphabetic() {
            hit = in_ranges(class, (c as u8 ^ 0x20) as u32);
        }
    }
    hit != class.negated
}

/// How a pattern is anchored, from its first item
fn anchor_of(node: &Node) -> Anchor {
    match node {
        Node::Assert(Assertion::Start { multiline: false } | Assertion::SubjectStart) => Anchor::Start,
        Node::Assert(Assertion::StartOffset) => Anchor::StartOffset,
        Node::Seq(v) => v.first().map_or(Anchor::None, |first| match first {
            Node::Repeat { node, min: 0, max: None, kind: RepKind::Greedy | RepKind::Possessive }
                if matches!(**node, Node::Any { .. }) =>
            {
                // A leading .* can only match where the previous attempt failed
                // if that was at a newline
                match **node {
                    Node::Any { dotall: true } => Anchor::StartOffset,
                    _ => Anchor::Line,
                }
            }
            other => anchor_of(other),
        }),
        Node::Alt(v) => {
            let first = anchor_of(&v[0]);
            if first != Anchor::None && v.iter().all(|b| anchor_of(b) == first) { first } else { Anchor::None }
        }
        Node::Capture { node, .. } | Node::Group(node) | Node::Atomic(node) => anchor_of(node),
        _ => Anchor::None,
    }
}

/// Adds the bytes a match of `node` can start with to `set`. Some(nullable),
/// or None when anything goes
fn first_bytes(node: &Node, c: &Compiler, set: &mut [bool; 256], utf: bool, depth: usize) -> Option<bool> {
    if depth > 50 {
        return None;
    }
    let add_char = |set: &mut [bool; 256], ch: u32| {
        if utf && ch >= 128 {
            let mut buf = Vec::new();
            super::encode_utf8(ch, &mut buf);
            set[buf[0] as usize] = true;
        } else {
            set[ch as usize & 0xff] = true;
        }
    };
    match node {
        Node::Empty | Node::Assert(_) | Node::Look { .. } | Node::Keep | Node::Verb(Verb::Mark(_)) => Some(true),
        Node::Char { c: ch, caseless } => {
            add_char(set, *ch);
            if *caseless {
                if utf {
                    let o = unicode::other_case(*ch);
                    add_char(set, o);
                    if let Some(s) = unicode::caseless_set(*ch) {
                        for &x in s {
                            add_char(set, x);
                        }
                    }
                } else if (*ch as u8).is_ascii_alphabetic() {
                    set[(*ch as u8 ^ 0x20) as usize] = true;
                }
            }
            Some(false)
        }
        Node::Class(class) => {
            let limit = if utf { 128 } else { 256 };
            for b in 0..limit {
                if class_matches(class, b, utf, c.parsed.ucp) {
                    set[b as usize] = true;
                }
            }
            if utf {
                // Any non-ASCII character could match unless the class is plain ranges below 128
                let ascii_only = !class.negated
                    && class.items.iter().all(|it| matches!(it, ClassItem::Range(_, hi) if *hi < 128))
                    && !class.caseless;
                if !ascii_only {
                    set[0xc2..=0xf4].fill(true);
                }
            }
            Some(false)
        }
        Node::Seq(v) => {
            for n in v {
                if !first_bytes(n, c, set, utf, depth + 1)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        Node::Alt(v) => {
            let mut any_null = false;
            for n in v {
                any_null |= first_bytes(n, c, set, utf, depth + 1)?;
            }
            Some(any_null)
        }
        Node::Capture { node, .. } | Node::Group(node) | Node::Atomic(node) => first_bytes(node, c, set, utf, depth + 1),
        Node::Repeat { node, min, .. } => {
            let null = first_bytes(node, c, set, utf, depth + 1)?;
            Some(null || *min == 0)
        }
        _ => None,
    }
}

/// A byte that must appear in every match: from a literal character that is
/// not optional
fn required_byte(node: &Node, utf: bool) -> Option<u8> {
    match node {
        Node::Char { c, caseless } => {
            if *caseless && (*c >= 128 || (*c as u8).is_ascii_alphabetic()) {
                return None;
            }
            if utf && *c >= 128 {
                let mut buf = Vec::new();
                super::encode_utf8(*c, &mut buf);
                return buf.last().copied();
            }
            Some(*c as u8)
        }
        Node::Seq(v) => v.iter().rev().find_map(|n| required_byte(n, utf)),
        Node::Capture { node, .. } | Node::Group(node) | Node::Atomic(node) => required_byte(node, utf),
        Node::Repeat { node, min, .. } if *min > 0 => required_byte(node, utf),
        _ => None,
    }
}

/// PCRE's auto-possessification, in its simplest form: a greedy repeat
/// followed by a literal it cannot match never needs to give characters back
fn auto_possessify(prog: &mut [Inst], classes: &[CClass], utf: bool) {
    for pc in 0..prog.len().saturating_sub(1) {
        let Inst::Rep { unit, kind: RepKind::Greedy, .. } = prog[pc] else { continue };
        let next: &[u8] = match &prog[pc + 1] {
            Inst::Unit(Unit::Byte(b)) => std::slice::from_ref(b),
            Inst::Unit(Unit::ByteFold(a, b)) => &[*a, *b],
            Inst::Lit(l) => &l[..1],
            _ => continue,
        };
        let next = next.to_vec();
        if next.iter().any(|&b| utf && b >= 0x80 || unit_may_match(unit, b, classes)) {
            continue;
        }
        if let Inst::Rep { kind, .. } = &mut prog[pc] {
            *kind = RepKind::Possessive;
        }
    }
}

fn unit_may_match(unit: Unit, b: u8, classes: &[CClass]) -> bool {
    match unit {
        Unit::Byte(x) => x == b,
        Unit::ByteFold(x, y) => x == b || y == b,
        Unit::Char(bytes, _) => bytes[0] == b,
        Unit::CharFold(c) => unicode::caseless_eq(c, b as u32),
        Unit::AnyNotNl | Unit::Any | Unit::AnyByte => true,
        Unit::Class(i) => classes[i as usize].matches(b as u32),
    }
}
