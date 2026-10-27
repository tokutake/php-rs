//! Non-fatal diagnostics (warnings, deprecations) and the current source line.
//!
//! Value conversions deep inside the runtime need to report warnings, but only the
//! interpreter owns the output stream. They are queued here and the interpreter
//! drains the queue before every write, so they appear in the same order as in PHP.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Warning,
    Notice,
    Deprecated,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Warning => "Warning",
            Level::Notice => "Notice",
            Level::Deprecated => "Deprecated",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub level: Level,
    pub msg: String,
    pub line: u32,
    pub file: Rc<str>,
}

thread_local! {
    static LINE: Cell<u32> = const { Cell::new(0) };
    /// The file of the code running, for the location of diagnostics
    static FILE: RefCell<Rc<str>> = RefCell::new(Rc::from(""));
    static PENDING: RefCell<Vec<Diag>> = const { RefCell::new(Vec::new()) };
    /// Nesting of `@` operators being evaluated
    static SILENCED: Cell<u32> = const { Cell::new(0) };
}

/// Runs `f` with diagnostics suppressed, as the `@` operator does
pub fn silenced<T>(f: impl FnOnce() -> T) -> T {
    SILENCED.with(|s| s.set(s.get() + 1));
    let out = f();
    SILENCED.with(|s| s.set(s.get() - 1));
    out
}

pub fn set_line(line: u32) {
    LINE.with(|l| l.set(line));
}

pub fn line() -> u32 {
    LINE.with(Cell::get)
}

pub fn set_file(file: Rc<str>) {
    FILE.with(|f| *f.borrow_mut() = file);
}

pub fn file() -> Rc<str> {
    FILE.with(|f| f.borrow().clone())
}

fn push(level: Level, msg: String) {
    if SILENCED.with(Cell::get) > 0 {
        return;
    }
    let line = line();
    let file = file();
    PENDING.with(|p| p.borrow_mut().push(Diag { level, msg, line, file }));
}

pub fn warning(msg: impl Into<String>) {
    push(Level::Warning, msg.into());
}

pub fn notice(msg: impl Into<String>) {
    push(Level::Notice, msg.into());
}

pub fn deprecated(msg: impl Into<String>) {
    push(Level::Deprecated, msg.into());
}

/// A deprecation found while compiling `file`, at a line of its own
pub fn compile_deprecated(file: &str, line: u32, msg: impl Into<String>) {
    PENDING.with(|p| p.borrow_mut().push(Diag { level: Level::Deprecated, msg: msg.into(), line, file: file.into() }));
}

pub fn take() -> Vec<Diag> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}
