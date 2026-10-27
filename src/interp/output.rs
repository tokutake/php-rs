//! Output buffering: the ob_* functions. Everything the script prints (echo,
//! diagnostics, uncaught errors) goes to the innermost buffer if there is one;
//! writes to STDOUT bypass the buffers. Buffers left at the end of the script
//! are flushed, after the destructors have run.

use super::*;

/// Operation flags passed to handlers (PHP_OUTPUT_HANDLER_*)
const START: i64 = 1;
const CLEAN: i64 = 2;
const FLUSH: i64 = 4;
const FINAL: i64 = 8;
/// What ob_start()'s `$flags` allow
const CLEANABLE: i64 = 16;
const FLUSHABLE: i64 = 32;
const REMOVABLE: i64 = 64;
pub(super) const STDFLAGS: i64 = CLEANABLE | FLUSHABLE | REMOVABLE;
/// Status bits
const STARTED: i64 = 0x1000;
const DISABLED: i64 = 0x2000;

/// An output buffer started by ob_start()
pub(super) struct OutBuffer {
    data: Vec<u8>,
    /// The user handler, a callable
    handler: Option<Value>,
    /// As ob_list_handlers() shows it
    name: String,
    chunk_size: usize,
    /// CLEANABLE/FLUSHABLE/REMOVABLE plus the status bits
    flags: i64,
}

pub(super) fn output_constant(name: &str) -> Option<i64> {
    Some(match name {
        "PHP_OUTPUT_HANDLER_START" => START,
        "PHP_OUTPUT_HANDLER_WRITE" | "PHP_OUTPUT_HANDLER_CONT" => 0,
        "PHP_OUTPUT_HANDLER_FLUSH" => FLUSH,
        "PHP_OUTPUT_HANDLER_CLEAN" => CLEAN,
        "PHP_OUTPUT_HANDLER_FINAL" | "PHP_OUTPUT_HANDLER_END" => FINAL,
        "PHP_OUTPUT_HANDLER_CLEANABLE" => CLEANABLE,
        "PHP_OUTPUT_HANDLER_FLUSHABLE" => FLUSHABLE,
        "PHP_OUTPUT_HANDLER_REMOVABLE" => REMOVABLE,
        "PHP_OUTPUT_HANDLER_STDFLAGS" => STDFLAGS,
        "PHP_OUTPUT_HANDLER_STARTED" => STARTED,
        "PHP_OUTPUT_HANDLER_DISABLED" => DISABLED,
        "PHP_OUTPUT_HANDLER_PROCESSED" => 0x4000,
        _ => return None,
    })
}

impl<W: Write> Interp<W> {
    /// Script output: into the innermost buffer, which a handler with a chunk
    /// size then processes once it is full enough
    pub(super) fn emit(&mut self, bytes: &[u8]) -> EResult<()> {
        let full = match self.ob.last_mut() {
            // A handler's own output goes nowhere
            _ if self.ob_running => return Ok(()),
            Some(b) => {
                b.data.extend_from_slice(bytes);
                b.chunk_size > 0 && b.data.len() >= b.chunk_size
            }
            None => return self.raw_out(bytes),
        };
        if full {
            self.ob_flush_top(0)?;
        }
        Ok(())
    }

    /// Output that must not run handlers (diagnostics, uncaught errors)
    pub(super) fn emit_quiet(&mut self, bytes: &[u8]) {
        match self.ob.last_mut() {
            _ if self.ob_running => {}
            Some(b) => b.data.extend_from_slice(bytes),
            None => {
                let _ = self.raw_out(bytes);
            }
        }
    }

    /// Standard output itself
    pub(super) fn raw_out(&mut self, bytes: &[u8]) -> EResult<()> {
        if self.stdout_closed {
            return Ok(());
        }
        // From here on headers count as sent
        if self.errors.output_started.is_none() && !bytes.is_empty() {
            self.errors.output_started = Some((crate::diag::file(), crate::diag::line()));
        }
        self.out.write_all(bytes).map_err(|e| Fatal::error(format!("write failed: {e}")))
    }

    /// Runs buffer `i`'s handler over `data` for the operation `op`. A handler
    /// that fails (returns false or throws) passes the output through as it
    /// is, and is not called again; its exception comes second
    fn ob_handle(&mut self, i: usize, data: Vec<u8>, op: i64) -> (Vec<u8>, EResult<()>) {
        let b = &mut self.ob[i];
        let first = b.flags & STARTED == 0;
        b.flags |= STARTED;
        let handler = match &b.handler {
            Some(h) if b.flags & DISABLED == 0 => h.clone(),
            _ => return (data, Ok(())),
        };
        let phase = if first { op | START } else { op };
        self.ob_running = true;
        let result = self.call_callback(&handler, vec![Value::str(data.clone()), Value::Int(phase)]);
        self.ob_running = false;
        let result = match result {
            Ok(Value::Bool(false)) => Ok(None),
            Ok(v) => self.stringify(&v).map(Some),
            Err(e) => Err(e),
        };
        match result {
            Ok(Some(out)) => (out, Ok(())),
            Ok(None) => {
                self.ob[i].flags |= DISABLED;
                (data, Ok(()))
            }
            Err(e) => {
                self.ob[i].flags |= DISABLED;
                (data, Err(e))
            }
        }
    }

    /// Hands processed output to the buffer below `i`, or to standard output
    fn ob_deliver(&mut self, i: usize, bytes: &[u8]) -> EResult<()> {
        match i.checked_sub(1) {
            Some(below) => {
                self.ob[below].data.extend_from_slice(bytes);
                Ok(())
            }
            None => self.raw_out(bytes),
        }
    }

    /// Processes the innermost buffer and passes its output on; `op` is FLUSH,
    /// or 0 for a buffer that reached its chunk size
    fn ob_flush_top(&mut self, op: i64) -> EResult<()> {
        let i = self.ob.len() - 1;
        let data = std::mem::take(&mut self.ob[i].data);
        let (out, result) = self.ob_handle(i, data, op);
        self.ob_deliver(i, &out)?;
        result
    }

    /// Removes the innermost buffer, passing its output on if `flush`
    fn ob_end_top(&mut self, flush: bool) -> EResult<()> {
        let i = self.ob.len() - 1;
        let data = std::mem::take(&mut self.ob[i].data);
        let op = if flush { FINAL } else { FINAL | CLEAN };
        let (out, result) = self.ob_handle(i, data, op);
        self.ob.truncate(i);
        if flush {
            self.ob_deliver(i, &out)?;
        }
        result
    }

    /// At the end of the script: flushes every buffer, whatever its flags
    pub(super) fn ob_end_all(&mut self) {
        while !self.ob.is_empty() {
            if let Err(e) = self.ob_end_top(true) {
                self.report(e);
            }
        }
    }

    /// The innermost buffer, if it allows the operation: else a notice naming it
    fn ob_allowed(&mut self, fname: &str, flag: i64, what: &str) -> bool {
        let b = self.ob.last().expect("checked by the caller");
        if b.flags & flag != 0 {
            return true;
        }
        diag::notice(format!("{fname}(): Failed to {what} buffer of {} ({})", b.name, self.ob.len() - 1));
        false
    }

    pub(super) fn output_builtin(&mut self, lname: &str, vals: Vec<Value>) -> EResult<Value> {
        // Diagnostics raised so far are output already, into the buffer
        self.flush_diags();
        let max = match lname {
            "ob_start" => 3,
            "ob_get_status" | "ob_implicit_flush" => 1,
            _ => 0,
        };
        if vals.len() > max {
            return Err(builtins::arg_count(lname, max, vals.len()));
        }
        if self.ob_running && lname == "ob_start" {
            let mut e = Fatal::error("ob_start(): Cannot use output buffering in output buffering display handlers");
            e.uncatchable = true;
            return Err(e);
        }
        let none = |msg: &str| {
            diag::notice(format!("{lname}(): {msg}"));
            Ok(Value::Bool(false))
        };
        let level = self.ob.len();
        Ok(match lname {
            "ob_start" => return self.ob_start(vals),
            "ob_get_contents" => self.ob.last().map_or(Value::Bool(false), |b| Value::str(b.data.clone())),
            "ob_get_length" => self.ob.last().map_or(Value::Bool(false), |b| Value::Int(b.data.len() as i64)),
            "ob_get_level" => Value::Int(level as i64),
            "ob_flush" | "ob_clean" | "ob_end_flush" | "ob_end_clean" | "ob_get_flush" if level == 0 => {
                return none(match lname {
                    "ob_flush" => "Failed to flush buffer. No buffer to flush",
                    "ob_clean" | "ob_end_clean" => "Failed to delete buffer. No buffer to delete",
                    _ => "Failed to delete and flush buffer. No buffer to delete or flush",
                });
            }
            "ob_get_clean" if level == 0 => Value::Bool(false),
            "ob_flush" => {
                if !self.ob_allowed(lname, FLUSHABLE, "flush") {
                    return Ok(Value::Bool(false));
                }
                self.ob_flush_top(FLUSH)?;
                Value::Bool(true)
            }
            "ob_clean" => {
                if !self.ob_allowed(lname, CLEANABLE, "delete") {
                    return Ok(Value::Bool(false));
                }
                let data = std::mem::take(&mut self.ob[level - 1].data);
                self.ob_handle(level - 1, data, CLEAN).1?;
                Value::Bool(true)
            }
            "ob_end_flush" | "ob_end_clean" => {
                let flush = lname == "ob_end_flush";
                if !self.ob_allowed(lname, REMOVABLE, if flush { "send" } else { "discard" }) {
                    return Ok(Value::Bool(false));
                }
                self.ob_end_top(flush)?;
                Value::Bool(true)
            }
            "ob_get_flush" | "ob_get_clean" => {
                let v = Value::str(self.ob[level - 1].data.clone());
                let flush = lname == "ob_get_flush";
                if self.ob_allowed(lname, REMOVABLE, if flush { "send" } else { "discard" }) {
                    self.ob_end_top(flush)?;
                } else {
                    let b = &self.ob[level - 1];
                    diag::notice(format!("{lname}(): Failed to delete buffer of {} ({})", b.name, level - 1));
                }
                v
            }
            "ob_list_handlers" => list(self.ob.iter().map(|b| Value::str(b.name.as_str())).collect()),
            "ob_get_status" => {
                let full = vals.first().is_some_and(Value::to_bool);
                let status = |(i, b): (usize, &OutBuffer)| {
                    let user = b.handler.is_some() as i64;
                    let buffer_size = if b.chunk_size > 1 { (b.chunk_size + 1).div_ceil(4096) * 4096 } else { 16384 };
                    let mut a = PhpArray::new();
                    a.insert(Key::from_bytes(b"name"), Value::str(b.name.as_str()));
                    a.insert(Key::from_bytes(b"type"), Value::Int(user));
                    a.insert(Key::from_bytes(b"flags"), Value::Int(b.flags | user));
                    a.insert(Key::from_bytes(b"level"), Value::Int(i as i64));
                    a.insert(Key::from_bytes(b"chunk_size"), Value::Int(b.chunk_size as i64));
                    a.insert(Key::from_bytes(b"buffer_size"), Value::Int(buffer_size as i64));
                    a.insert(Key::from_bytes(b"buffer_used"), Value::Int(b.data.len() as i64));
                    Value::Array(Rc::new(a))
                };
                if full {
                    list(self.ob.iter().enumerate().map(status).collect())
                } else {
                    match self.ob.last() {
                        Some(b) => status((level - 1, b)),
                        None => Value::Array(Rc::new(PhpArray::new())),
                    }
                }
            }
            "ob_implicit_flush" => Value::Null,
            "flush" => {
                let _ = self.out.flush();
                Value::Null
            }
            _ => return Err(Fatal::error(format!("Call to undefined function {lname}()"))),
        })
    }

    fn ob_start(&mut self, vals: Vec<Value>) -> EResult<Value> {
        let mut vals = vals.into_iter();
        let handler = vals.next().filter(|v| !v.is_null());
        let chunk_size = match vals.next() {
            Some(v) => builtins::arg_int("ob_start", 2, "chunk_size", &v)?.max(0) as usize,
            None => 0,
        };
        let flags = match vals.next() {
            Some(v) => builtins::arg_int("ob_start", 3, "flags", &v)? & STDFLAGS,
            None => STDFLAGS,
        };
        let name = match &handler {
            None => "default output handler".to_string(),
            Some(h) => {
                if self.callable(h)?.is_none() {
                    let reason = self.not_callable_reason(h)?;
                    diag::warning(format!("ob_start(): {reason}"));
                    diag::notice("ob_start(): Failed to create buffer");
                    return Ok(Value::Bool(false));
                }
                handler_name(h)
            }
        };
        self.ob.push(OutBuffer { data: Vec::new(), handler, name, chunk_size, flags });
        Ok(Value::Bool(true))
    }
}

/// A handler as ob_list_handlers() names it
fn handler_name(h: &Value) -> String {
    match h {
        Value::Str(s) => String::from_utf8_lossy(s).into_owned(),
        // A closure shows its own name, {closure:FILE:LINE}
        Value::Object(o) => match o.closure() {
            Some(c) if c.forward.is_none() => c.name.clone(),
            _ => format!("{}::__invoke", o.class.name),
        },
        Value::Array(a) => {
            let class = match a.get(&Key::Int(0)) {
                Some(Value::Object(o)) => o.class.name.to_string(),
                Some(v) => String::from_utf8_lossy(&v.to_bytes()).into_owned(),
                None => String::new(),
            };
            let method = a.get(&Key::Int(1)).map(|m| String::from_utf8_lossy(&m.to_bytes()).into_owned()).unwrap_or_default();
            format!("{class}::{method}")
        }
        _ => "???".into(),
    }
}
