//! Error handling and the runtime environment around it: error_reporting(),
//! set_error_handler(), trigger_error(), set_exception_handler(), shutdown
//! functions, assert(), ini settings, debug_backtrace() and the CLI's view of
//! HTTP headers

use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use super::{builtins, list, EResult, Fatal, Interp};
use crate::array::{Key, PhpArray};
use crate::datetime::{DateTime, Zone};
use crate::diag::{self, Diag, Level};
use crate::object::Object;
use crate::value::Value;

pub(super) const NAMES: &[&str] = &[
    "error_reporting", "set_error_handler", "restore_error_handler", "set_exception_handler",
    "restore_exception_handler", "trigger_error", "user_error", "error_get_last", "error_clear_last", "error_log",
    "register_shutdown_function", "assert", "func_get_arg", "debug_backtrace", "debug_print_backtrace", "ini_get",
    "ini_set", "ini_alter", "ini_restore", "set_time_limit", "ignore_user_abort", "headers_sent", "header",
    "header_remove", "headers_list", "http_response_code", "setcookie", "setrawcookie", "connection_status",
    "connection_aborted",
];

/// E_ALL in PHP 8.4 and later
pub(super) const E_ALL: i64 = 30719;
const E_USER_ERROR: i64 = 256;

/// The settings ini_get() knows, with php-cli's values when run without a php.ini
const INI_DEFAULTS: &[(&str, &str)] = &[
    ("allow_url_fopen", "1"),
    ("arg_separator.input", "&"),
    ("arg_separator.output", "&"),
    ("auto_append_file", ""),
    ("auto_prepend_file", ""),
    ("date.timezone", ""),
    ("default_charset", "UTF-8"),
    ("default_socket_timeout", "60"),
    ("disable_classes", ""),
    ("disable_functions", ""),
    ("display_errors", "1"),
    ("display_startup_errors", "1"),
    ("error_log", ""),
    ("error_reporting", ""),
    ("file_uploads", "1"),
    ("html_errors", "0"),
    ("implicit_flush", "1"),
    ("include_path", ".:/usr/share/php"),
    ("log_errors", "0"),
    ("max_execution_time", "0"),
    ("max_file_uploads", "20"),
    ("max_input_nesting_level", "64"),
    ("max_input_time", "-1"),
    ("max_input_vars", "1000"),
    ("memory_limit", "128M"),
    ("open_basedir", ""),
    ("output_buffering", "0"),
    ("output_handler", ""),
    ("pcre.backtrack_limit", "1000000"),
    ("pcre.jit", "1"),
    ("pcre.recursion_limit", "100000"),
    ("post_max_size", "8M"),
    ("precision", "14"),
    ("register_argc_argv", "1"),
    ("request_order", ""),
    ("serialize_precision", "-1"),
    ("short_open_tag", "1"),
    ("unserialize_max_depth", "4096"),
    ("upload_max_filesize", "2M"),
    ("upload_tmp_dir", ""),
    ("user_agent", ""),
    ("variables_order", "EGPCS"),
    ("zend.assertions", "1"),
    ("zend.enable_gc", "1"),
];

pub(super) struct ErrorState {
    pub(super) reporting: i64,
    /// set_error_handler() handlers, innermost last, with the levels they take
    handlers: Vec<(Value, i64)>,
    exception_handlers: Vec<Value>,
    pub(super) shutdown: Vec<(Value, Vec<Value>)>,
    /// ini_set() changes
    ini: HashMap<String, String>,
    /// Where output first went to the client, which makes headers "sent"
    pub(super) output_started: Option<(Rc<str>, u32)>,
    /// Header lines for the response, as "Name: value"
    pub(super) headers: Vec<String>,
    /// The response code, once header() or http_response_code() set one
    pub(super) status: Option<u16>,
}

impl Default for ErrorState {
    fn default() -> Self {
        ErrorState {
            reporting: E_ALL,
            handlers: Vec::new(),
            exception_handlers: Vec::new(),
            shutdown: Vec::new(),
            ini: HashMap::new(),
            output_started: None,
            headers: Vec::new(),
            status: None,
        }
    }
}

impl ErrorState {
    /// The name of a "Name: value" header line
    fn header_name(line: &str) -> Option<&str> {
        line.split_once(':').map(|(name, _)| name.trim())
    }

    /// Drops the header lines called `name`, ignoring case
    fn remove_header(&mut self, name: &[u8]) {
        self.headers.retain(|h| Self::header_name(h).is_none_or(|n| !n.as_bytes().eq_ignore_ascii_case(name)));
    }

    /// header(): a status line sets the response code, other lines are kept for
    /// the response, replacing earlier lines of the same name when asked to
    fn add_header(&mut self, line: String, replace: bool) {
        if line.starts_with("HTTP/") {
            if let Some(code) = line.split_whitespace().nth(1).and_then(|c| c.parse().ok()) {
                self.status = Some(code);
            }
            return;
        }
        if replace && let Some(name) = Self::header_name(&line) {
            self.remove_header(name.as_bytes());
        }
        self.headers.push(line);
    }

    pub(super) fn has_handler(&self) -> bool {
        !self.handlers.is_empty()
    }

    pub(super) fn display(&self) -> bool {
        let v = self.ini.get("display_errors").map_or("1", String::as_str);
        !matches!(v.to_ascii_lowercase().as_str(), "" | "0" | "off" | "false" | "no" | "stderr")
    }

    fn ini_get(&self, name: &str) -> Option<String> {
        if name == "error_reporting" {
            return Some(self.ini.get(name).cloned().unwrap_or_default());
        }
        self.ini.get(name).cloned().or_else(|| INI_DEFAULTS.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string()))
    }
}

/// An ini value as php-src parses a number with an optional K/M/G suffix
fn ini_int(s: &str) -> i64 {
    let s = s.trim();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().unwrap_or(0)
}

impl<W: Write> Interp<W> {
    /// Shows a diagnostic as php-cli does, unless error_reporting or
    /// display_errors hide it
    pub(super) fn show_diag(&mut self, d: &Diag) {
        if self.errors.reporting & d.level.code() == 0 || !self.errors.display() {
            return;
        }
        let msg = format!("\n{}: {} in {} on line {}\n", d.level.label(), d.msg, d.file, d.line);
        self.emit_quiet(msg.as_bytes());
    }

    /// Hands pending diagnostics to the error handler, showing those it
    /// declines. An exception from the handler propagates
    pub(super) fn dispatch_diags(&mut self) -> EResult<()> {
        let pending = diag::take();
        let mut rest = pending.into_iter();
        while let Some(d) = rest.next() {
            let handler = self.errors.handlers.last().filter(|(_, mask)| mask & d.level.code() != 0).map(|(h, _)| h.clone());
            let Some(handler) = handler else {
                self.show_diag(&d);
                continue;
            };
            // A handler's own errors go to the default handling
            let saved = std::mem::take(&mut self.errors.handlers);
            let args = vec![Value::Int(d.level.code()), Value::str(d.msg.clone()), Value::str(&*d.file), Value::Int(d.line as i64)];
            let result = self.callback("set_error_handler", 1, &handler).and_then(|f| self.call_value(&f, args, d.line));
            let inner = diag::take();
            self.errors.handlers = saved;
            for i in &inner {
                self.show_diag(i);
            }
            match result {
                Ok(Value::Bool(false)) => self.show_diag(&d),
                Ok(_) => {}
                Err(e) => {
                    // The diagnostics after this one were raised by the same statement
                    for d in rest {
                        self.show_diag(&d);
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// The uncaught exception, given to the exception handler if one is set:
    /// None when it handled it
    pub(super) fn handle_uncaught(&mut self, e: Fatal) -> Option<Fatal> {
        if e.uncatchable || e.exit.is_some() {
            return Some(e);
        }
        let Some(handler) = self.errors.exception_handlers.pop() else { return Some(e) };
        let ex = self.exception_object(e);
        let line = diag::line();
        self.callback("set_exception_handler", 1, &handler).and_then(|f| self.call_value(&f, vec![Value::Object(ex)], line)).err()
    }

    /// Runs the shutdown functions, including ones they register
    pub(super) fn run_shutdown_functions(&mut self) -> EResult<()> {
        let mut i = 0;
        while let Some((f, args)) = self.errors.shutdown.get(i).cloned() {
            let line = diag::line();
            let f = self.callback("register_shutdown_function", 1, &f)?;
            self.call_value(&f, args, line)?;
            self.flush_diags();
            i += 1;
        }
        self.errors.shutdown.clear();
        Ok(())
    }

    /// The call stack for debug_backtrace(), innermost first, with the object
    /// of each method frame
    fn backtrace(&self) -> Vec<(crate::object::TraceFrame, Option<Rc<Object>>)> {
        let frames = self.trace();
        // PHP frames appear in the trace in order, innermost first; built-in
        // calls sit between them
        let mut objects = vec![None; frames.len()];
        let mut j = 0;
        for i in (1..self.frames.len()).rev() {
            let name = &self.frames[i].ctx.trace_fn;
            while j < frames.len() && frames[j].function != *name {
                j += 1;
            }
            if j < frames.len() {
                if frames[j].class.as_ref().is_some_and(|(_, t)| *t == "->") {
                    objects[j] = self.frames[i].ctx.this.clone();
                }
                j += 1;
            }
        }
        frames.into_iter().zip(objects).collect()
    }

    pub(super) fn errors_builtin(&mut self, lname: &str, vals: Vec<Value>) -> EResult<Value> {
        match lname {
            "error_reporting" => {
                let [level] = builtins::args::<0, 1>(lname, vals)?;
                let old = self.errors.reporting;
                if let Some(level) = level
                    && !level.is_null()
                {
                    self.errors.reporting = builtins::arg_int(lname, 1, "error_level", &level)?;
                    self.errors.ini.insert("error_reporting".into(), self.errors.reporting.to_string());
                }
                Ok(Value::Int(old))
            }
            "set_error_handler" => {
                let [handler, levels] = builtins::args::<1, 2>(lname, vals)?;
                let handler = handler.unwrap().copy_out();
                if !handler.is_null() && self.callable(&handler)?.is_none() {
                    let detail = self.not_callable_reason(&handler)?;
                    return Err(Fatal::type_error(format!(
                        "set_error_handler(): Argument #1 ($callback) must be a valid callback or null, {detail}"
                    )));
                }
                let levels = match levels {
                    Some(l) => builtins::arg_int(lname, 2, "error_levels", &l)?,
                    None => E_ALL,
                };
                let old = self.errors.handlers.last().map_or(Value::Null, |(h, _)| h.clone());
                if handler.is_null() {
                    // null restores php's own handling, as a handler of its own
                    self.errors.handlers.push((Value::Null, 0));
                } else {
                    self.errors.handlers.push((handler, levels));
                }
                Ok(old)
            }
            "restore_error_handler" => {
                builtins::args::<0, 0>(lname, vals)?;
                self.errors.handlers.pop();
                Ok(Value::Bool(true))
            }
            "set_exception_handler" => {
                let [handler] = builtins::args::<1, 1>(lname, vals)?;
                let handler = handler.unwrap().copy_out();
                if !handler.is_null() && self.callable(&handler)?.is_none() {
                    let detail = self.not_callable_reason(&handler)?;
                    return Err(Fatal::type_error(format!(
                        "set_exception_handler(): Argument #1 ($callback) must be a valid callback or null, {detail}"
                    )));
                }
                let old = self.errors.exception_handlers.last().cloned().unwrap_or(Value::Null);
                if handler.is_null() {
                    self.errors.exception_handlers.clear();
                } else {
                    self.errors.exception_handlers.push(handler);
                }
                Ok(old)
            }
            "restore_exception_handler" => {
                builtins::args::<0, 0>(lname, vals)?;
                self.errors.exception_handlers.pop();
                Ok(Value::Bool(true))
            }
            "trigger_error" | "user_error" => {
                let [msg, level] = builtins::args::<1, 2>(lname, vals)?;
                let msg = String::from_utf8_lossy(&builtins::arg_str(self, lname, 1, "message", &msg.unwrap())?).into_owned();
                let level = match level {
                    Some(l) => builtins::arg_int(lname, 2, "error_level", &l)?,
                    None => 1024,
                };
                let level = match level {
                    E_USER_ERROR => {
                        diag::deprecated(
                            "Passing E_USER_ERROR to trigger_error() is deprecated since 8.4, throw an exception or call exit with a string message instead",
                        );
                        self.dispatch_diags()?;
                        return self.user_error(msg);
                    }
                    512 => Level::UserWarning,
                    1024 => Level::UserNotice,
                    16384 => Level::UserDeprecated,
                    _ => {
                        return Err(Fatal::value_error(format!(
                            "{lname}(): Argument #2 ($error_level) must be one of E_USER_ERROR, E_USER_WARNING, E_USER_NOTICE, or E_USER_DEPRECATED"
                        )));
                    }
                };
                diag::push(level, msg);
                if self.errors.has_handler() {
                    self.dispatch_diags()?;
                }
                Ok(Value::Bool(true))
            }
            "error_get_last" => {
                builtins::args::<0, 0>(lname, vals)?;
                Ok(match diag::last() {
                    None => Value::Null,
                    Some((ty, msg, file, line)) => {
                        let mut a = PhpArray::new();
                        a.insert(Key::Str((*b"type").into()), Value::Int(ty));
                        a.insert(Key::Str((*b"message").into()), Value::str(msg));
                        a.insert(Key::Str((*b"file").into()), Value::str(&*file));
                        a.insert(Key::Str((*b"line").into()), Value::Int(line as i64));
                        Value::Array(Rc::new(a))
                    }
                })
            }
            "error_clear_last" => {
                builtins::args::<0, 0>(lname, vals)?;
                diag::set_last(None);
                Ok(Value::Null)
            }
            "error_log" => {
                let [msg, ty, dest, _headers] = builtins::args::<1, 4>(lname, vals)?;
                let msg = builtins::arg_str(self, lname, 1, "message", &msg.unwrap())?;
                let ty = match ty {
                    Some(t) => builtins::arg_int(lname, 2, "message_type", &t)?,
                    None => 0,
                };
                if ty == 3 {
                    // Appends to the file named by the destination
                    let dest = builtins::arg_str(self, lname, 3, "destination", &dest.unwrap_or(Value::Null))?;
                    let path = std::path::PathBuf::from(String::from_utf8_lossy(&dest).into_owned());
                    let ok = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| f.write_all(&msg));
                    return Ok(Value::Bool(ok.is_ok()));
                }
                // Without an error_log setting the CLI logs to standard error
                let mut line = msg;
                line.push(b'\n');
                let _ = std::io::stderr().write_all(&line);
                Ok(Value::Bool(true))
            }
            "register_shutdown_function" => {
                let mut vals = vals.into_iter();
                let Some(f) = vals.next() else { return Err(builtins::arg_count(lname, 1, 0)) };
                if self.callable(&f)?.is_none() {
                    let detail = self.not_callable_reason(&f)?;
                    return Err(Fatal::type_error(format!(
                        "register_shutdown_function(): Argument #1 ($callback) must be a valid callback, {detail}"
                    )));
                }
                self.errors.shutdown.push((f, vals.collect()));
                Ok(Value::Null)
            }
            "assert" => {
                let [assertion, description] = builtins::args::<1, 2>(lname, vals)?;
                if assertion.unwrap().to_bool() {
                    return Ok(Value::Bool(true));
                }
                match description.map(|d| d.copy_out()) {
                    Some(Value::Object(o)) if o.class.is_throwable => Err(Fatal { object: Some(o), ..Fatal::new("", "") }),
                    Some(d) if !d.is_null() => {
                        let msg = String::from_utf8_lossy(&self.stringify(&d)?).into_owned();
                        Err(Fatal::new("AssertionError", msg))
                    }
                    _ => Err(Fatal::new("AssertionError", "assert(false)")),
                }
            }
            "func_get_arg" => {
                let [n] = builtins::args::<1, 1>(lname, vals)?;
                let n = builtins::arg_int(lname, 1, "position", &n.unwrap())?;
                if self.frames.len() < 2 {
                    return Err(Fatal::error("func_get_arg() cannot be called from the global scope"));
                }
                let args = &self.frame().args;
                if n < 0 {
                    return Err(Fatal::value_error("func_get_arg(): Argument #1 ($position) must be greater than or equal to 0"));
                }
                match args.get(n as usize) {
                    Some(v) => Ok(v.copy_out()),
                    None => Err(Fatal::value_error(
                        "func_get_arg(): Argument #1 ($position) must be less than the number of the arguments passed to the currently executed function",
                    )),
                }
            }
            "debug_backtrace" | "debug_print_backtrace" => {
                let [options, limit] = builtins::args::<0, 2>(lname, vals)?;
                let options = match options {
                    Some(o) => builtins::arg_int(lname, 1, "options", &o)?,
                    None => 1,
                };
                let limit = match limit {
                    Some(l) => builtins::arg_int(lname, 2, "limit", &l)?,
                    None => 0,
                };
                let mut frames = self.backtrace();
                // The innermost frame is this function's own call
                if frames.first().is_some_and(|(f, _)| f.function == lname) {
                    frames.remove(0);
                }
                if limit > 0 {
                    frames.truncate(limit as usize);
                }
                let with_args = options & 2 == 0;
                if lname == "debug_print_backtrace" {
                    let mut out = Vec::new();
                    for (i, (f, _)) in frames.iter().enumerate() {
                        let mut f = f.clone();
                        if !with_args {
                            f.args.clear();
                        }
                        let line = crate::object::trace_string(std::slice::from_ref(&f));
                        let line = line.lines().next().unwrap_or("").replacen("#0", &format!("#{i}"), 1);
                        out.extend_from_slice(line.as_bytes());
                        out.push(b'\n');
                    }
                    self.write(&out)?;
                    return Ok(Value::Null);
                }
                let mut list_out = Vec::new();
                for (f, object) in frames {
                    let mut a = PhpArray::new();
                    let mut put = |k: &str, v: Value| a.insert(Key::Str(k.as_bytes().into()), v);
                    if !f.file.is_empty() {
                        put("file", Value::str(f.file.as_str()));
                        put("line", Value::Int(f.line as i64));
                    }
                    put("function", Value::str(f.function.as_str()));
                    if let Some((class, ty)) = &f.class {
                        put("class", Value::str(class.as_str()));
                        if let Some(o) = object.filter(|_| options & 1 != 0) {
                            put("object", Value::Object(o));
                        }
                        put("type", Value::str(*ty));
                    }
                    if with_args {
                        put("args", list(f.args.clone()));
                    }
                    list_out.push(Value::Array(Rc::new(a)));
                }
                Ok(list(list_out))
            }
            "ini_get" => {
                let [name] = builtins::args::<1, 1>(lname, vals)?;
                let name = String::from_utf8_lossy(&builtins::arg_str(self, lname, 1, "option", &name.unwrap())?).into_owned();
                Ok(self.errors.ini_get(&name).map_or(Value::Bool(false), Value::str))
            }
            "ini_set" | "ini_alter" => {
                let [name, value] = builtins::args::<2, 2>(lname, vals)?;
                let name = String::from_utf8_lossy(&builtins::arg_str(self, lname, 1, "option", &name.unwrap())?).into_owned();
                let value = match value.unwrap().copy_out() {
                    Value::Bool(b) => if b { "1".to_string() } else { String::new() },
                    Value::Null => String::new(),
                    v => String::from_utf8_lossy(&builtins::arg_str(self, lname, 2, "value", &v)?).into_owned(),
                };
                let Some(old) = self.errors.ini_get(&name) else { return Ok(Value::Bool(false)) };
                if name == "error_reporting" {
                    self.errors.reporting = ini_int(&value);
                }
                self.errors.ini.insert(name, value);
                Ok(Value::str(old))
            }
            "ini_restore" => {
                let [name] = builtins::args::<1, 1>(lname, vals)?;
                let name = String::from_utf8_lossy(&builtins::arg_str(self, lname, 1, "option", &name.unwrap())?).into_owned();
                self.errors.ini.remove(&name);
                if name == "error_reporting" {
                    self.errors.reporting = E_ALL;
                }
                Ok(Value::Null)
            }
            "set_time_limit" => {
                builtins::args::<1, 1>(lname, vals)?;
                Ok(Value::Bool(true))
            }
            "ignore_user_abort" => {
                builtins::args::<0, 1>(lname, vals)?;
                Ok(Value::Int(0))
            }
            "connection_status" | "connection_aborted" => {
                builtins::args::<0, 0>(lname, vals)?;
                Ok(Value::Int(0))
            }
            "headers_sent" => {
                builtins::args::<0, 2>(lname, vals)?;
                Ok(Value::Bool(self.errors.output_started.is_some()))
            }
            "header" | "header_remove" | "setcookie" | "setrawcookie" => {
                if let Some((file, line)) = &self.errors.output_started {
                    diag::warning(format!(
                        "Cannot modify header information - headers already sent by (output started at {file}:{line})"
                    ));
                    return Ok(if lname.ends_with("cookie") { Value::Bool(false) } else { Value::Null });
                }
                match lname {
                    "header" => {
                        let [line, replace, code] = builtins::args::<1, 3>(lname, vals)?;
                        let line = builtins::arg_str(self, lname, 1, "header", &line.unwrap())?;
                        let replace = replace.is_none_or(|r| r.to_bool());
                        if let Some(code) = code.filter(|c| !c.is_null()) {
                            let code = builtins::arg_int(lname, 3, "response_code", &code)?;
                            if code > 0 {
                                self.errors.status = Some(code as u16);
                            }
                        }
                        self.errors.add_header(String::from_utf8_lossy(&line).into_owned(), replace);
                        Ok(Value::Null)
                    }
                    "header_remove" => {
                        let [name] = builtins::args::<0, 1>(lname, vals)?;
                        match name.filter(|n| !n.is_null()) {
                            Some(name) => {
                                let name = builtins::arg_str(self, lname, 1, "name", &name)?;
                                self.errors.remove_header(&name);
                            }
                            None => self.errors.headers.clear(),
                        }
                        Ok(Value::Null)
                    }
                    // setcookie() and setrawcookie(): the value is urlencoded only by setcookie()
                    _ => {
                        let [name, value, expires, path, domain, secure, httponly] = builtins::args::<1, 7>(lname, vals)?;
                        let name = builtins::arg_str(self, lname, 1, "name", &name.unwrap())?;
                        let value = value.map(|v| builtins::arg_str(self, lname, 2, "value", &v)).transpose()?.unwrap_or_default();
                        let value = if lname == "setcookie" { builtins::url::encode(&value, false) } else { value };
                        // Either the options array (PHP 7.3+) or the positional arguments
                        let (expires, path, domain, secure, httponly, samesite) = match expires {
                            Some(Value::Array(opts)) => {
                                let opt = |key: &[u8]| opts.get(&Key::Str(key.into())).cloned();
                                (
                                    opt(b"expires").map_or(0, |v| v.to_int()),
                                    opt(b"path").map_or_else(Vec::new, |v| v.to_bytes()),
                                    opt(b"domain").map_or_else(Vec::new, |v| v.to_bytes()),
                                    opt(b"secure").is_some_and(|v| v.to_bool()),
                                    opt(b"httponly").is_some_and(|v| v.to_bool()),
                                    opt(b"samesite").map_or_else(Vec::new, |v| v.to_bytes()),
                                )
                            }
                            expires => (
                                expires.map_or(0, |v| v.to_int()),
                                path.map(|p| builtins::arg_str(self, lname, 4, "path", &p)).transpose()?.unwrap_or_default(),
                                domain.map(|d| builtins::arg_str(self, lname, 5, "domain", &d)).transpose()?.unwrap_or_default(),
                                secure.is_some_and(|v| v.to_bool()),
                                httponly.is_some_and(|v| v.to_bool()),
                                Vec::new(),
                            ),
                        };
                        let mut attrs = vec![format!("{}={}", String::from_utf8_lossy(&name), String::from_utf8_lossy(&value))];
                        if expires > 0 {
                            let zone = Zone::load("UTC").unwrap();
                            let when = DateTime { seconds: expires, micros: 0, zone }.format_gmt(b"D, d-M-Y H:i:s T");
                            attrs.push(format!("expires={}", String::from_utf8_lossy(&when)));
                        }
                        if !path.is_empty() {
                            attrs.push(format!("path={}", String::from_utf8_lossy(&path)));
                        }
                        if !domain.is_empty() {
                            attrs.push(format!("domain={}", String::from_utf8_lossy(&domain)));
                        }
                        if secure {
                            attrs.push("secure".to_owned());
                        }
                        if httponly {
                            attrs.push("HttpOnly".to_owned());
                        }
                        if !samesite.is_empty() {
                            attrs.push(format!("SameSite={}", String::from_utf8_lossy(&samesite)));
                        }
                        self.errors.add_header(format!("Set-Cookie: {}", attrs.join("; ")), false);
                        Ok(Value::Bool(true))
                    }
                }
            }
            "headers_list" => {
                builtins::args::<0, 0>(lname, vals)?;
                Ok(list(self.errors.headers.iter().map(|h| Value::str(h.as_str())).collect()))
            }
            "http_response_code" => {
                let [code] = builtins::args::<0, 1>(lname, vals)?;
                match code.filter(|c| !c.is_null()) {
                    Some(code) => {
                        self.errors.status = Some(builtins::arg_int(lname, 1, "response_code", &code)? as u16);
                        Ok(Value::Bool(true))
                    }
                    None => Ok(self.errors.status.map_or(Value::Bool(false), |c| Value::Int(c.into()))),
                }
            }
            _ => Err(Fatal::error(format!("Call to undefined function {lname}()"))),
        }
    }

    /// trigger_error() with E_USER_ERROR: the handler's to deal with, or fatal
    fn user_error(&mut self, msg: String) -> EResult<Value> {
        if let Some((h, mask)) = self.errors.handlers.last().cloned()
            && mask & E_USER_ERROR != 0
            && !h.is_null()
        {
            let line = diag::line();
            let args = vec![Value::Int(E_USER_ERROR), Value::str(msg.clone()), Value::str(&*diag::file()), Value::Int(line as i64)];
            let f = self.callback("set_error_handler", 1, &h)?;
            let saved = std::mem::take(&mut self.errors.handlers);
            let r = self.call_value(&f, args, line);
            self.errors.handlers = saved;
            if !matches!(r?, Value::Bool(false)) {
                return Ok(Value::Bool(true));
            }
        }
        diag::set_last(Some((E_USER_ERROR, msg.clone(), diag::file(), diag::line())));
        Err(Fatal::compile(msg))
    }
}
