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
    /// trigger_error()'s levels
    UserWarning,
    UserNotice,
    UserDeprecated,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Warning | Level::UserWarning => "Warning",
            Level::Notice | Level::UserNotice => "Notice",
            Level::Deprecated | Level::UserDeprecated => "Deprecated",
        }
    }

    /// The E_* constant
    pub fn code(self) -> i64 {
        match self {
            Level::Warning => 2,
            Level::Notice => 8,
            Level::Deprecated => 8192,
            Level::UserWarning => 512,
            Level::UserNotice => 1024,
            Level::UserDeprecated => 16384,
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
    /// The last diagnostic, silenced ones included, for error_get_last()
    static LAST: RefCell<Option<(i64, String, Rc<str>, u32)>> = const { RefCell::new(None) };
}

/// The last error as error_get_last() reports it: type, message, file and line
pub fn last() -> Option<(i64, String, Rc<str>, u32)> {
    LAST.with(|l| l.borrow().clone())
}

pub fn set_last(last: Option<(i64, String, Rc<str>, u32)>) {
    LAST.with(|l| *l.borrow_mut() = last);
}

/// Whether diagnostics are waiting to be reported
pub fn pending() -> bool {
    PENDING.with(|p| !p.borrow().is_empty())
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

pub fn push(level: Level, msg: String) {
    let line = line();
    let file = file();
    LAST.with(|l| *l.borrow_mut() = Some((level.code(), msg.clone(), file.clone(), line)));
    if SILENCED.with(Cell::get) > 0 {
        return;
    }
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
    let msg = msg.into();
    LAST.with(|l| *l.borrow_mut() = Some((Level::Deprecated.code(), msg.clone(), file.into(), line)));
    PENDING.with(|p| p.borrow_mut().push(Diag { level: Level::Deprecated, msg: msg.into(), line, file: file.into() }));
}

pub fn take() -> Vec<Diag> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}
