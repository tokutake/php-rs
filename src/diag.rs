//! Non-fatal diagnostics (warnings, deprecations) and the current source line.
//!
//! Value conversions deep inside the runtime need to report warnings, but only the
//! interpreter owns the output stream. They are queued here and the interpreter
//! drains the queue before every write, so they appear in the same order as in PHP.

use std::cell::{Cell, RefCell};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Warning,
    Deprecated,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Warning => "Warning",
            Level::Deprecated => "Deprecated",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub level: Level,
    pub msg: String,
    pub line: u32,
}

thread_local! {
    static LINE: Cell<u32> = const { Cell::new(0) };
    static PENDING: RefCell<Vec<Diag>> = const { RefCell::new(Vec::new()) };
}

pub fn set_line(line: u32) {
    LINE.with(|l| l.set(line));
}

pub fn line() -> u32 {
    LINE.with(Cell::get)
}

fn push(level: Level, msg: String) {
    let line = line();
    PENDING.with(|p| p.borrow_mut().push(Diag { level, msg, line }));
}

pub fn warning(msg: impl Into<String>) {
    push(Level::Warning, msg.into());
}

pub fn deprecated(msg: impl Into<String>) {
    push(Level::Deprecated, msg.into());
}

pub fn take() -> Vec<Diag> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}
