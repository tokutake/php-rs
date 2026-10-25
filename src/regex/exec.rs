//! The backtracking matcher. Choice points live on a heap stack (`bt`)
//! together with the undo records for captures, so matching never recurses
//! natively except into assertions and atomic groups, whose nesting the
//! pattern bounds (and a depth limit guards, with recursion).

use super::compile::{Anchor, Inst, Regex, SubKind, Unit, VerbKind};
use super::parse::{Assertion, RepKind};
use super::unicode;
use super::{decode_utf8, MatchOptions, Newline};

/// php-src's pcre.backtrack_limit default
const BACKTRACK_LIMIT: u64 = 1_000_000;
/// Choice points (and undo records) at once
const STACK_LIMIT: usize = 4_000_000;
/// Assertions and atomic groups inside each other, counting recursion
const NESTING_LIMIT: usize = 1000;
/// Recursion and subroutine calls outstanding
const CALL_LIMIT: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecError {
    /// PREG_BACKTRACK_LIMIT_ERROR
    BacktrackLimit,
    /// PREG_RECURSION_LIMIT_ERROR
    RecursionLimit,
    /// PREG_JIT_STACKLIMIT_ERROR: the choice point stack is full
    StackLimit,
    /// A recursion that would loop forever (PREG_INTERNAL_ERROR)
    RecursionLoop,
}

const UNSET: usize = usize::MAX;

/// Why matching stopped early
enum Stop {
    Error(ExecError),
    /// Backtracking reached a (*COMMIT), (*PRUNE), (*SKIP) or (*THEN)
    Verb(VerbKind, usize),
}

impl From<ExecError> for Stop {
    fn from(e: ExecError) -> Self {
        Stop::Error(e)
    }
}

enum Bt {
    Alt { pc: u32, pos: usize, frame: u32 },
    Cap { slot: u32, old: usize },
    Reg { reg: u32, old: usize },
    /// A greedy single-character repeat that can give back characters down to `min_pos`
    Greedy { pc: u32, min_pos: usize, pos: usize, frame: u32 },
    /// A lazy repeat (the Rep at `pc`) that can take more characters
    Lazy { pc: u32, pos: usize, count: u32, frame: u32 },
    Verb { kind: VerbKind, pos: usize },
    Mark { old: u32 },
}

struct Frame {
    ret: u32,
    group: u32,
    parent: u32,
    pos: usize,
    depth: u32,
    caps: Box<[usize]>,
}

/// The result of a successful match: capture offsets (UNSET where a group did
/// not take part) and the last (*MARK) name passed
pub struct Captures {
    pub slots: Vec<usize>,
    pub mark: Option<Vec<u8>>,
}

impl Captures {
    pub fn get(&self, i: usize) -> Option<(usize, usize)> {
        let (s, e) = (*self.slots.get(2 * i)?, *self.slots.get(2 * i + 1)?);
        (s != UNSET && e != UNSET).then_some((s, e))
    }

    /// One more than the highest group that took part, as pcre2_match returns
    pub fn count(&self) -> usize {
        (0..self.slots.len() / 2).rev().find(|&i| self.get(i).is_some()).map_or(1, |i| i + 1)
    }
}

struct Matcher<'a> {
    re: &'a Regex,
    s: &'a [u8],
    start: usize,
    opts: MatchOptions,
    caps: Vec<usize>,
    regs: Vec<usize>,
    bt: Vec<Bt>,
    frames: Vec<Frame>,
    steps: u64,
    depth: usize,
    mark: u32,
}

impl Regex {
    /// Looks for a match starting at or after `start`. In UTF mode the
    /// subject must be valid UTF-8 and `start` a character boundary.
    pub fn exec(&self, s: &[u8], start: usize, opts: MatchOptions) -> Result<Option<Captures>, ExecError> {
        if start > s.len() {
            return Ok(None);
        }
        if let Some(b) = self.required
            && !s[start..].contains(&b)
        {
            return Ok(None);
        }
        let mut m = Matcher {
            re: self,
            s,
            start,
            opts,
            // Completed groups (start, end), then the start of each group while it is open
            caps: vec![UNSET; 3 * (self.capture_count + 1)],
            regs: vec![UNSET; self.regs],
            bt: SCRATCH.with(|sc| std::mem::take(&mut sc.borrow_mut().0)),
            frames: SCRATCH.with(|sc| std::mem::take(&mut sc.borrow_mut().1)),
            steps: 0,
            depth: 0,
            mark: u32::MAX,
        };
        if m.frames.is_empty() {
            m.frames.push(Frame { ret: 0, group: u32::MAX, parent: 0, pos: 0, depth: 0, caps: Box::new([]) });
        }
        let anchored = opts.anchored || matches!(self.anchor, Anchor::Start | Anchor::StartOffset);
        let mut pos = start;
        loop {
            m.caps.fill(UNSET);
            m.regs.fill(UNSET);
            m.bt.clear();
            m.frames.truncate(1);
            m.mark = u32::MAX;
            let mut next = None;
            match m.run(0, pos, 0, true) {
                Ok(Some(_)) => {
                    let mark = (m.mark != u32::MAX).then(|| self.marks[m.mark as usize].clone());
                    let mut slots = std::mem::take(&mut m.caps);
                    slots.truncate(2 * (self.capture_count + 1));
                    return Ok(Some(Captures { slots, mark }));
                }
                Ok(None) => {}
                Err(Stop::Error(e)) => return Err(e),
                Err(Stop::Verb(VerbKind::Commit, _)) => return Ok(None),
                Err(Stop::Verb(VerbKind::Skip, at)) if at > pos => next = Some(at),
                Err(Stop::Verb(_, _)) => {}
            }
            if anchored || pos >= s.len() {
                return Ok(None);
            }
            pos = match next {
                Some(p) => p,
                None => m.advance(pos),
            };
            // Skip ahead to a place where a match could start
            match self.anchor {
                Anchor::Line => {
                    while pos < s.len() && !m.newline_before(pos) {
                        pos += 1;
                    }
                    if pos >= s.len() && !m.newline_before(pos) {
                        return Ok(None);
                    }
                }
                _ => {
                    if let Some(first) = &self.first {
                        while pos < s.len() && !first[s[pos] as usize] {
                            pos += 1;
                        }
                        if pos >= s.len() {
                            return Ok(None);
                        }
                    }
                }
            }
        }
    }
}

thread_local! {
    /// The choice point stack and frames, kept between matches
    static SCRATCH: std::cell::RefCell<(Vec<Bt>, Vec<Frame>)> = const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

impl Drop for Matcher<'_> {
    fn drop(&mut self) {
        let mut bt = std::mem::take(&mut self.bt);
        let mut frames = std::mem::take(&mut self.frames);
        bt.clear();
        frames.truncate(1);
        // Do not hold on to the memory of a pathological match
        bt.shrink_to(1 << 16);
        frames.shrink_to(1 << 10);
        SCRATCH.with(|sc| *sc.borrow_mut() = (bt, frames));
    }
}

impl Matcher<'_> {
    /// The start of the next character
    #[inline]
    fn advance(&self, pos: usize) -> usize {
        if self.re.utf { pos + char_len(self.s, pos) } else { pos + 1 }
    }

    #[inline]
    fn char_at(&self, pos: usize) -> (u32, usize) {
        if self.re.utf { decode_at(self.s, pos) } else { (self.s[pos] as u32, 1) }
    }

    fn tick(&mut self) -> Result<(), Stop> {
        self.steps += 1;
        if self.steps > BACKTRACK_LIMIT {
            return Err(Stop::Error(ExecError::BacktrackLimit));
        }
        Ok(())
    }

    #[inline]
    fn push(&mut self, bt: Bt) -> Result<(), Stop> {
        if self.bt.len() >= STACK_LIMIT {
            return Err(Stop::Error(ExecError::StackLimit));
        }
        self.bt.push(bt);
        Ok(())
    }

    /// Where the start of open group `g` is kept
    #[inline]
    fn open_slot(&self, g: usize) -> usize {
        2 * (self.re.capture_count + 1) + g
    }

    fn set_cap(&mut self, slot: usize, pos: usize) -> Result<(), Stop> {
        let old = self.caps[slot];
        self.push(Bt::Cap { slot: slot as u32, old })?;
        self.caps[slot] = pos;
        Ok(())
    }

    /// The length of the newline at `pos`, if there is one
    fn newline_at(&self, pos: usize) -> Option<usize> {
        let s = self.s;
        let c = *s.get(pos)?;
        match self.re.newline {
            Newline::Lf => (c == b'\n').then_some(1),
            Newline::Cr => (c == b'\r').then_some(1),
            Newline::Nul => (c == 0).then_some(1),
            Newline::CrLf => (c == b'\r' && s.get(pos + 1) == Some(&b'\n')).then_some(2),
            Newline::AnyCrLf | Newline::Any => {
                if c == b'\r' {
                    return Some(if s.get(pos + 1) == Some(&b'\n') { 2 } else { 1 });
                }
                if c == b'\n' {
                    return Some(1);
                }
                if self.re.newline == Newline::AnyCrLf {
                    return None;
                }
                match c {
                    0x0b | 0x0c => Some(1),
                    0x85 if !self.re.utf => Some(1),
                    0xc2 if self.re.utf && s.get(pos + 1) == Some(&0x85) => Some(2),
                    0xe2 if self.re.utf && s.get(pos + 1) == Some(&0x80) && matches!(s.get(pos + 2), Some(0xa8 | 0xa9)) => {
                        Some(3)
                    }
                    _ => None,
                }
            }
        }
    }

    /// Whether a newline ends just before `pos`
    fn newline_before(&self, pos: usize) -> bool {
        if pos == 0 {
            return false;
        }
        let s = self.s;
        match self.re.newline {
            Newline::Lf => s[pos - 1] == b'\n',
            Newline::Cr => s[pos - 1] == b'\r',
            Newline::Nul => s[pos - 1] == 0,
            Newline::CrLf => pos >= 2 && &s[pos - 2..pos] == b"\r\n",
            Newline::AnyCrLf | Newline::Any => {
                let c = s[pos - 1];
                if c == b'\n' {
                    return true;
                }
                if c == b'\r' {
                    // Not between the halves of a CRLF
                    return s.get(pos) != Some(&b'\n');
                }
                if self.re.newline == Newline::AnyCrLf {
                    return false;
                }
                if self.re.utf {
                    let p = unicode::prev_char(s, pos, true);
                    self.newline_at(p) == Some(pos - p)
                } else {
                    matches!(c, 0x0b | 0x0c | 0x85)
                }
            }
        }
    }

    fn is_word_at(&self, pos: usize) -> bool {
        if pos >= self.s.len() {
            return false;
        }
        let (c, _) = self.char_at(pos);
        if self.re.ucp { unicode::is_word(c) } else { c < 128 && ((c as u8).is_ascii_alphanumeric() || c == b'_' as u32) }
    }

    fn is_word_before(&self, pos: usize) -> bool {
        pos > 0 && self.is_word_at(unicode::prev_char(self.s, pos, self.re.utf))
    }

    fn assert(&self, a: Assertion, pos: usize) -> bool {
        let len = self.s.len();
        match a {
            Assertion::Start { multiline: false } | Assertion::SubjectStart => pos == 0,
            Assertion::Start { multiline: true } => pos == 0 || pos < len && self.newline_before(pos),
            Assertion::End { multiline: false, endonly: true } | Assertion::SubjectEnd => pos == len,
            Assertion::End { multiline: false, endonly: false } | Assertion::SubjectEndNewline => {
                pos == len || self.newline_at(pos).is_some_and(|n| pos + n == len)
            }
            Assertion::End { multiline: true, .. } => pos == len || self.newline_at(pos).is_some(),
            Assertion::StartOffset => pos == self.start,
            Assertion::WordBoundary => self.is_word_before(pos) != self.is_word_at(pos),
            Assertion::NotWordBoundary => self.is_word_before(pos) == self.is_word_at(pos),
        }
    }

    /// Matches one character with `unit`, giving the position after it
    #[inline]
    fn unit_at(&self, unit: Unit, pos: usize) -> Option<usize> {
        let s = self.s;
        if pos >= s.len() {
            return None;
        }
        match unit {
            Unit::Byte(b) => (s[pos] == b).then_some(pos + 1),
            Unit::ByteFold(a, b) => (s[pos] == a || s[pos] == b).then_some(pos + 1),
            Unit::Char(bytes, n) => s[pos..].starts_with(&bytes[..n as usize]).then_some(pos + n as usize),
            Unit::CharFold(c) => {
                let (ch, n) = self.char_at(pos);
                unicode::caseless_eq(c, ch).then_some(pos + n)
            }
            Unit::Any => Some(self.advance(pos)),
            Unit::AnyNotNl => {
                if self.newline_at(pos).is_some() {
                    return None;
                }
                Some(self.advance(pos))
            }
            Unit::AnyByte => Some(pos + 1),
            Unit::Class(i) => {
                let (ch, n) = self.char_at(pos);
                self.re.classes[i as usize].matches(ch).then_some(pos + n)
            }
        }
    }

    /// Steps back one character in a repeat of `unit`
    fn back_one(&self, unit: Unit, pos: usize) -> usize {
        match unit {
            Unit::AnyByte => pos - 1,
            _ => unicode::prev_char(self.s, pos, self.re.utf),
        }
    }

    /// `n` characters back from `pos`
    fn back_n(&self, mut pos: usize, n: usize) -> Option<usize> {
        for _ in 0..n {
            if pos == 0 {
                return None;
            }
            pos = unicode::prev_char(self.s, pos, self.re.utf);
        }
        Some(pos)
    }

    /// Undoes everything above `base`, as when a nested match is discarded
    fn unwind(&mut self, base: usize) {
        while self.bt.len() > base {
            match self.bt.pop().unwrap() {
                Bt::Cap { slot, old } => self.caps[slot as usize] = old,
                Bt::Reg { reg, old } => self.regs[reg as usize] = old,
                Bt::Mark { old } => self.mark = old,
                _ => {}
            }
        }
    }

    /// After a nested match succeeds: drop its choice points but keep the undo
    /// records, so that backtracking past it still restores the captures
    fn commit_nested(&mut self, base: usize) {
        let mut w = base;
        for r in base..self.bt.len() {
            if matches!(self.bt[r], Bt::Cap { .. } | Bt::Reg { .. } | Bt::Mark { .. }) {
                self.bt.swap(w, r);
                w += 1;
            }
        }
        self.bt.truncate(w);
    }

    /// Backtracks to the newest choice point above `base`: Some((pc, pos, frame))
    fn backtrack(&mut self, base: usize) -> Result<Option<(usize, usize, u32)>, Stop> {
        while self.bt.len() > base {
            match self.bt.pop().unwrap() {
                Bt::Cap { slot, old } => self.caps[slot as usize] = old,
                Bt::Reg { reg, old } => self.regs[reg as usize] = old,
                Bt::Mark { old } => self.mark = old,
                Bt::Alt { pc, pos, frame } => {
                    self.tick()?;
                    return Ok(Some((pc as usize, pos, frame)));
                }
                Bt::Greedy { pc, min_pos, pos, frame } => {
                    self.tick()?;
                    let re = self.re;
                    let unit = match re.prog[pc as usize - 1] {
                        Inst::Rep { unit, .. } => unit,
                        _ => unreachable!(),
                    };
                    let mut p = self.back_one(unit, pos);
                    // When a literal byte follows, give back characters until it could match
                    let want = match &re.prog[pc as usize] {
                        Inst::Lit(l) => Some(l[0]),
                        Inst::Unit(Unit::Byte(b)) => Some(*b),
                        _ => None,
                    };
                    if let Some(b) = want {
                        while p > min_pos && self.s.get(p) != Some(&b) {
                            p = self.back_one(unit, p);
                        }
                    }
                    if p > min_pos {
                        self.push(Bt::Greedy { pc, min_pos, pos: p, frame })?;
                    }
                    return Ok(Some((pc as usize, p, frame)));
                }
                Bt::Lazy { pc, pos, count, frame } => {
                    let re = self.re;
                    let Inst::Rep { unit, max, .. } = re.prog[pc as usize] else { unreachable!() };
                    if count < max
                        && let Some(np) = self.unit_at(unit, pos)
                    {
                        self.tick()?;
                        if count + 1 < max {
                            self.push(Bt::Lazy { pc, pos: np, count: count + 1, frame })?;
                        }
                        return Ok(Some((pc as usize + 1, np, frame)));
                    }
                }
                Bt::Verb { kind, pos } => return Err(Stop::Verb(kind, pos)),
            }
        }
        Ok(None)
    }

    fn backref(&self, groups: &[usize], caseless: bool, pos: usize) -> Option<usize> {
        let g = groups.iter().find(|&&g| self.caps[2 * g] != UNSET && self.caps[2 * g + 1] != UNSET)?;
        let (a, b) = (self.caps[2 * g], self.caps[2 * g + 1]);
        let s = self.s;
        if a > b {
            return None;
        }
        let want = &s[a..b];
        if !caseless {
            return s[pos..].starts_with(want).then_some(pos + want.len());
        }
        if !self.re.utf {
            let have = s.get(pos..pos + want.len())?;
            return have.eq_ignore_ascii_case(want).then_some(pos + want.len());
        }
        let (mut i, mut p) = (0, pos);
        while i < want.len() {
            if p >= s.len() {
                return None;
            }
            let (c1, l1) = decode_at(want, i);
            let (c2, l2) = decode_at(s, p);
            if !unicode::caseless_eq(c1, c2) {
                return None;
            }
            i += l1;
            p += l2;
        }
        Some(p)
    }

    /// `\R`
    fn newline_seq(&self, pos: usize) -> Option<usize> {
        let s = self.s;
        let c = *s.get(pos)?;
        match c {
            b'\r' => Some(if s.get(pos + 1) == Some(&b'\n') { pos + 2 } else { pos + 1 }),
            b'\n' => Some(pos + 1),
            _ if self.re.bsr_anycrlf => None,
            0x0b | 0x0c => Some(pos + 1),
            0x85 if !self.re.utf => Some(pos + 1),
            _ if self.re.utf && c >= 0x80 => {
                let (ch, n) = decode_at(s, pos);
                matches!(ch, 0x85 | 0x2028 | 0x2029).then_some(pos + n)
            }
            _ => None,
        }
    }

    /// Runs from `pc` until a Match (at the top) or the SubEnd of the nested
    /// match this is: the position reached and the frame, or None
    fn run(&mut self, mut pc: usize, mut pos: usize, mut frame: u32, top: bool) -> Result<Option<(usize, u32)>, Stop> {
        let base = self.bt.len();
        let entry_frame = frame;
        let re = self.re;
        let s = self.s;
        macro_rules! fail {
            () => {
                match self.backtrack(base)? {
                    Some((p, q, f)) => {
                        pc = p;
                        pos = q;
                        frame = f;
                        continue;
                    }
                    None => return Ok(None),
                }
            };
        }
        loop {
            match &re.prog[pc] {
                Inst::Unit(u) => match self.unit_at(*u, pos) {
                    Some(np) => {
                        pos = np;
                        pc += 1;
                    }
                    None => fail!(),
                },
                Inst::Lit(l) => {
                    if s[pos..].starts_with(l) {
                        pos += l.len();
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::Rep { unit, min, max, kind } => {
                    let (unit, min, max, kind) = (*unit, *min, *max, *kind);
                    let mut n = 0;
                    let mut p = pos;
                    while n < min {
                        match self.unit_at(unit, p) {
                            Some(np) => p = np,
                            None => break,
                        }
                        n += 1;
                    }
                    if n < min {
                        fail!();
                    }
                    if kind == RepKind::Lazy {
                        if min < max {
                            self.push(Bt::Lazy { pc: pc as u32, pos: p, count: min, frame })?;
                        }
                        pos = p;
                        pc += 1;
                        continue;
                    }
                    let min_pos = p;
                    while n < max {
                        match self.unit_at(unit, p) {
                            Some(np) => p = np,
                            None => break,
                        }
                        n += 1;
                    }
                    if kind == RepKind::Greedy && p > min_pos {
                        self.push(Bt::Greedy { pc: pc as u32 + 1, min_pos, pos: p, frame })?;
                    }
                    pos = p;
                    pc += 1;
                }
                Inst::Split(a, b) => {
                    self.push(Bt::Alt { pc: *b as u32, pos, frame })?;
                    pc = *a;
                }
                Inst::Jmp(t) => pc = *t,
                Inst::Save(slot) => {
                    // A group's value changes only when it closes, so a back
                    // reference inside it still sees the previous iteration
                    let open = self.open_slot(*slot / 2);
                    if slot % 2 == 0 {
                        self.set_cap(open, pos)?;
                    } else {
                        let start = self.caps[open];
                        self.set_cap(slot - 1, start)?;
                        self.set_cap(*slot, pos)?;
                    }
                    pc += 1;
                }
                Inst::Assert(a) => {
                    if self.assert(*a, pos) {
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::Backref { groups, caseless } => match self.backref(groups, *caseless, pos) {
                    Some(np) => {
                        pos = np;
                        pc += 1;
                    }
                    None => fail!(),
                },
                Inst::LoopStart(reg) => {
                    let old = self.regs[*reg];
                    self.push(Bt::Reg { reg: *reg as u32, old })?;
                    self.regs[*reg] = pos;
                    pc += 1;
                }
                Inst::LoopCheck { reg, exit, back } => {
                    pc = if self.regs[*reg] == pos { *exit } else { *back };
                }
                Inst::Sub { kind, end } => {
                    let (kind, end) = (*kind, *end);
                    self.depth += 1;
                    if self.depth > NESTING_LIMIT {
                        return Err(Stop::Error(ExecError::RecursionLimit));
                    }
                    let sub_base = self.bt.len();
                    let r = self.run(pc + 1, pos, frame, false);
                    self.depth -= 1;
                    let r = match r {
                        Ok(r) => r,
                        // A verb backtracked to inside an assertion just fails it
                        Err(Stop::Verb(..)) if !matches!(kind, SubKind::Atomic) => {
                            self.unwind(sub_base);
                            None
                        }
                        Err(e) => return Err(e),
                    };
                    match kind {
                        SubKind::Atomic => match r {
                            Some((p, f)) => {
                                self.commit_nested(sub_base);
                                pos = p;
                                frame = f;
                                pc = end + 1;
                            }
                            None => fail!(),
                        },
                        SubKind::Ahead { neg } | SubKind::Cond { neg, .. } => {
                            let holds = r.is_some() != neg;
                            if r.is_some() {
                                if neg {
                                    self.unwind(sub_base);
                                } else {
                                    self.commit_nested(sub_base);
                                }
                            }
                            match (holds, kind) {
                                (true, _) => pc = end + 1,
                                (false, SubKind::Cond { no, .. }) => pc = no,
                                (false, _) => fail!(),
                            }
                        }
                    }
                }
                Inst::LookBehind { neg, branches, next, cond_no } => {
                    let (neg, next, cond_no) = (*neg, *next, *cond_no);
                    self.depth += 1;
                    if self.depth > NESTING_LIMIT {
                        return Err(Stop::Error(ExecError::RecursionLimit));
                    }
                    let sub_base = self.bt.len();
                    let mut found = false;
                    for &(len, start) in branches.iter() {
                        let Some(bpos) = self.back_n(pos, len) else { continue };
                        match self.run(start, bpos, frame, false) {
                            Ok(Some(_)) => {
                                found = true;
                                break;
                            }
                            Ok(None) => {}
                            Err(Stop::Verb(..)) => self.unwind(sub_base),
                            Err(e) => {
                                self.depth -= 1;
                                return Err(e);
                            }
                        }
                    }
                    self.depth -= 1;
                    if found {
                        if neg {
                            self.unwind(sub_base);
                        } else {
                            self.commit_nested(sub_base);
                        }
                    }
                    if found != neg {
                        pc = next;
                    } else if let Some(no) = cond_no {
                        pc = no;
                    } else {
                        fail!();
                    }
                }
                Inst::SubEnd => return Ok(Some((pos, frame))),
                Inst::CondGroup { groups, no } => {
                    let set = groups.iter().any(|&g| self.caps[2 * g + 1] != UNSET);
                    pc = if set { pc + 1 } else { *no };
                }
                Inst::CondRecursion { group, no } => {
                    let inside = frame != 0 && group.is_none_or(|g| self.frames[frame as usize].group as usize == g);
                    pc = if inside { pc + 1 } else { *no };
                }
                Inst::Call { target, group } => {
                    let (target, group) = (*target, *group);
                    // The same group again at the same place would never end.
                    // Calls further out were made at earlier positions, so only
                    // the innermost ones at this position need checking
                    let mut f = frame;
                    while f != 0 && self.frames[f as usize].pos == pos {
                        let fr = &self.frames[f as usize];
                        if fr.group as usize == group {
                            return Err(Stop::Error(ExecError::RecursionLoop));
                        }
                        f = fr.parent;
                    }
                    let depth = self.frames[frame as usize].depth + 1;
                    if depth as usize > CALL_LIMIT || self.frames.len() >= STACK_LIMIT / 4 {
                        return Err(Stop::Error(ExecError::RecursionLimit));
                    }
                    self.frames.push(Frame {
                        ret: pc as u32 + 1,
                        group: group as u32,
                        parent: frame,
                        pos,
                        depth,
                        caps: self.caps.clone().into_boxed_slice(),
                    });
                    frame = (self.frames.len() - 1) as u32;
                    pc = target;
                }
                Inst::GroupEnd(g) => {
                    if frame != 0 && self.frames[frame as usize].group as usize == *g {
                        self.return_from(&mut pc, &mut frame)?;
                    } else {
                        pc += 1;
                    }
                }
                Inst::Keep => {
                    let open = self.open_slot(0);
                    self.set_cap(open, pos)?;
                    pc += 1;
                }
                Inst::Newline => match self.newline_seq(pos) {
                    Some(np) => {
                        pos = np;
                        pc += 1;
                    }
                    None => fail!(),
                },
                Inst::ExtGrapheme => {
                    if pos >= s.len() {
                        fail!();
                    }
                    let (c, n) = self.char_at(pos);
                    pos = unicode::grapheme_end(s, pos + n, c, re.utf);
                    pc += 1;
                }
                Inst::Fail => fail!(),
                Inst::Accept => {
                    if frame != entry_frame && frame != 0 {
                        self.return_from(&mut pc, &mut frame)?;
                    } else if !top {
                        return Ok(Some((pos, frame)));
                    } else {
                        self.set_cap(1, pos)?;
                        pc = re.prog.len() - 1;
                    }
                }
                Inst::Verb(kind) => {
                    self.push(Bt::Verb { kind: *kind, pos })?;
                    pc += 1;
                }
                Inst::Mark(i) => {
                    let old = self.mark;
                    self.push(Bt::Mark { old })?;
                    self.mark = *i as u32;
                    pc += 1;
                }
                Inst::Match => {
                    let start = self.caps[self.open_slot(0)];
                    let empty = pos == start;
                    let at_start = start == self.start;
                    if empty && (re.notempty || (self.opts.notempty_atstart || re.notempty_atstart) && at_start) {
                        fail!();
                    }
                    self.caps[0] = start;
                    self.caps[1] = pos;
                    return Ok(Some((pos, frame)));
                }
            }
        }
    }

    /// Returns from a recursion: captures go back to what they were at the call
    fn return_from(&mut self, pc: &mut usize, frame: &mut u32) -> Result<(), Stop> {
        let f = *frame as usize;
        for slot in 0..self.caps.len() {
            let saved = self.frames[f].caps[slot];
            if self.caps[slot] != saved {
                self.set_cap(slot, saved)?;
            }
        }
        *pc = self.frames[f].ret as usize;
        *frame = self.frames[f].parent;
        Ok(())
    }
}

/// The length of the UTF-8 character at `pos`, tolerating bad input
#[inline]
fn char_len(s: &[u8], pos: usize) -> usize {
    let n = match s[pos] {
        0..=0xbf => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    n.min(s.len() - pos)
}

#[inline]
fn decode_at(s: &[u8], pos: usize) -> (u32, usize) {
    let want = match s[pos] {
        0..=0xbf => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    if want == 1 || s.len() - pos < want {
        return (s[pos] as u32, 1);
    }
    decode_utf8(&s[pos..pos + want])
}
