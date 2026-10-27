//! Streams and the file system: fopen() and friends, file_get_contents(),
//! file_put_contents(), file(), the directory functions, unlink(), mkdir(),
//! rename(), copy(), stat() and the STDIN/STDOUT/STDERR constants.

use std::fs::{self, OpenOptions};
use std::io;
use std::rc::Rc;

use super::{arg_bool, arg_int, arg_str, args, expect_array, list, Runtime};
use crate::array::{Key, PhpArray};
use crate::diag;
use crate::interp::{EResult, Fatal};
use crate::stream::{self, Backend, IoFail, Resource, Sink, Stream, CHUNK};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "fopen", "fclose", "fread", "fwrite", "fputs", "fgets", "fgetc", "feof", "fflush", "ftell", "fseek", "rewind",
    "ftruncate", "fstat", "flock", "fpassthru", "fprintf", "vfprintf", "fputcsv", "fsync", "fdatasync",
    "stream_get_contents", "stream_get_meta_data", "stream_is_local", "fgetcsv", "str_getcsv", "is_resource", "get_resource_type",
    "get_resource_id", "tmpfile", "tempnam", "sys_get_temp_dir", "file_get_contents", "file_put_contents", "file",
    "readfile", "unlink", "mkdir", "rmdir", "rename", "copy", "touch", "chmod", "filesize", "filemtime", "fileatime",
    "filectime", "fileperms", "fileinode", "fileowner", "filegroup", "filetype", "is_writable", "is_writeable",
    "is_executable", "is_link", "stat", "lstat", "clearstatcache", "opendir", "readdir", "closedir", "rewinddir",
    "scandir", "glob", "fnmatch", "umask",
];

pub fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "STDIN" => Value::Resource(std_stream(0)),
        "STDOUT" => Value::Resource(std_stream(1)),
        "STDERR" => Value::Resource(std_stream(2)),
        _ => {
            return Some(Value::Int(match name {
                "SEEK_SET" => 0,
                "SEEK_CUR" => 1,
                "SEEK_END" => 2,
                "LOCK_SH" => 1,
                "LOCK_EX" => 2,
                "LOCK_UN" => 3,
                "LOCK_NB" => 4,
                "FILE_USE_INCLUDE_PATH" => 1,
                "FILE_IGNORE_NEW_LINES" => 2,
                "FILE_SKIP_EMPTY_LINES" => 4,
                "FILE_APPEND" => 8,
                "FILE_NO_DEFAULT_CONTEXT" => 16,
                "SCANDIR_SORT_ASCENDING" => 0,
                "SCANDIR_SORT_DESCENDING" => 1,
                "SCANDIR_SORT_NONE" => 2,
                "GLOB_MARK" => 2,
                "GLOB_NOSORT" => 4,
                "GLOB_NOCHECK" => 16,
                "GLOB_NOESCAPE" => 64,
                "GLOB_ERR" => 1,
                "GLOB_BRACE" => 1024,
                "GLOB_ONLYDIR" => 1 << 13,
                "GLOB_AVAILABLE_FLAGS" => 1 | 2 | 4 | 16 | 64 | 1024 | (1 << 13),
                "FNM_NOESCAPE" => 2,
                "FNM_PATHNAME" => 1,
                "FNM_PERIOD" => 4,
                "FNM_CASEFOLD" => 16,
                _ => return None,
            }));
        }
    })
}

thread_local! {
    static STD: [Rc<Resource>; 3] = [
        Rc::new(Resource { id: 1, stream: std::cell::RefCell::new(Some(Stream::new(Backend::Stdin, "r", b"php://stdin"))) }),
        Rc::new(Resource { id: 2, stream: std::cell::RefCell::new(Some(Stream::new(Backend::Stdout, "w", b"php://stdout"))) }),
        Rc::new(Resource { id: 3, stream: std::cell::RefCell::new(Some(Stream::new(Backend::Stderr, "w", b"php://stderr"))) }),
    ];
}

fn std_stream(i: usize) -> Rc<Resource> {
    STD.with(|s| s[i].clone())
}

/// The interpreter's output, for writes to STDOUT and php://output
struct Out<'a>(&'a mut dyn Runtime);

impl Sink for Out<'_> {
    fn out(&mut self, bytes: &[u8]) {
        self.0.echo(bytes);
    }

    fn out_direct(&mut self, bytes: &[u8]) {
        self.0.echo_direct(bytes);
    }

    fn flush_out(&mut self) {
        self.0.flush_out();
    }
}

/// A stream argument: an open stream resource
fn arg_stream(fname: &str, n: usize, param: &str, v: &Value) -> EResult<Rc<Resource>> {
    match v {
        Value::Resource(r) if r.is_open() => Ok(r.clone()),
        Value::Resource(_) => Err(Fatal::type_error(format!("{fname}(): Argument #{n} (${param}) must be an open stream resource"))),
        other => Err(Fatal::type_error(format!(
            "{fname}(): Argument #{n} (${param}) must be of type resource, {} given",
            other.value_name()
        ))),
    }
}

/// Runs `f` on the stream of an open resource
fn with_stream<T>(r: &Resource, f: impl FnOnce(&mut Stream) -> T) -> T {
    let mut s = r.stream.borrow_mut();
    f(s.as_mut().expect("checked open"))
}

/// A path argument: no NUL bytes
fn arg_path(rt: &mut dyn Runtime, fname: &str, n: usize, param: &str, v: &Value) -> EResult<Vec<u8>> {
    let p = arg_str(rt, fname, n, param, v)?;
    if p.contains(&0) {
        return Err(Fatal::value_error(format!("{fname}(): Argument #{n} (${param}) must not contain any null bytes")));
    }
    Ok(p)
}

fn os_path(p: &[u8]) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    std::path::PathBuf::from(std::ffi::OsStr::from_bytes(p))
}

fn show(p: &[u8]) -> String {
    String::from_utf8_lossy(p).into_owned()
}

/// strerror's text for an I/O error
pub fn strerror(e: &io::Error) -> String {
    let s = e.to_string();
    s.split(" (os error").next().unwrap_or(&s).to_string()
}

/// A failed read: a notice and false, or the fatal error of running out of memory
fn read_notice(fname: &str, e: IoFail) -> EResult<Value> {
    if e.errno == stream::OUT_OF_MEMORY {
        return Err(Fatal::compile(format!(
            "Allowed memory size of {} bytes exhausted (tried to allocate {} bytes)",
            stream::READ_LIMIT,
            CHUNK
        )));
    }
    diag::notice(format!("{fname}(): Read of {CHUNK} bytes failed with errno={} {}", e.errno, e.msg));
    Ok(Value::Bool(false))
}

fn write_notice(fname: &str, n: usize, e: IoFail) -> Value {
    if e.errno == 0 {
        return Value::Bool(false);
    }
    diag::notice(format!("{fname}(): Write of {n} bytes failed with errno={} {}", e.errno, e.msg));
    Value::Bool(false)
}

/// The whole content of a file for md5_file() and the like: false (after a
/// warning) when it cannot be opened
pub fn read_file(rt: &mut dyn Runtime, fname: &str, n: usize, param: &str, v: &Value) -> EResult<Result<Vec<u8>, Value>> {
    let path = arg_path(rt, fname, n, param, v)?;
    if path.is_empty() {
        return Err(Fatal::value_error("Path must not be empty"));
    }
    stream::ensure_context();
    let Some(mut s) = open(fname, &path, "rb") else { return Ok(Err(Value::Bool(false))) };
    stream::alloc_id();
    match s.read_all() {
        Ok(b) => Ok(Ok(b)),
        Err(e) => read_notice(fname, e).map(Err),
    }
}

fn open_failed(fname: &str, path: &[u8], why: &str) -> Value {
    diag::warning(format!("{fname}({}): Failed to open stream: {why}", show(path)));
    Value::Bool(false)
}

/// Opens `path` the way php_stream_open_wrapper does, with its warnings.
/// None after warning
fn open(fname: &str, path: &[u8], mode: &str) -> Option<Stream> {
    let lower = path.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix(b"php://") {
        let backend = match rest {
            b"stdin" => Backend::Stdin,
            b"stdout" => Backend::Stdout,
            b"stderr" => Backend::Stderr,
            b"output" => Backend::Output,
            b"input" => Backend::Memory { data: stream::request_body(), pos: 0 },
            b"memory" => Backend::Memory { data: Vec::new(), pos: 0 },
            _ if rest.starts_with(b"temp") => {
                // A temp stream wraps a memory stream, which takes an id of its own
                stream::alloc_id();
                Backend::Memory { data: Vec::new(), pos: 0 }
            }
            _ => {
                diag::warning(format!("{fname}(): Invalid php:// URL specified"));
                open_failed(fname, path, "operation failed");
                return None;
            }
        };
        let mut s = Stream::new(backend, mode, path);
        match &s.backend {
            Backend::Memory { .. } => {
                // php_stream_mode_from_str: read-only unless the mode says otherwise
                s.append = mode.contains('a');
                s.readable = true;
                s.writable = rest != b"input" && (s.append || mode.contains(['w', '+']));
                s.mode = if s.append {
                    "a+b"
                } else if s.writable {
                    "w+b"
                } else {
                    "rb"
                }
                .into();
            }
            Backend::Stdin => {
                s.readable = true;
                s.writable = false;
            }
            _ => {
                s.readable = false;
                s.writable = true;
            }
        }
        return Some(s);
    }
    let file_path = path.strip_prefix(b"file://").unwrap_or(path);
    let Some((readable, writable, append)) = stream::mode_flags(mode) else {
        open_failed(fname, path, &format!("`{mode}' is not a valid mode for fopen"));
        return None;
    };
    let mut o = OpenOptions::new();
    o.read(readable || !writable).write(writable && !append).append(append);
    match mode.as_bytes()[0] {
        b'w' => {
            o.create(true).truncate(true);
        }
        b'a' | b'c' => {
            o.create(true);
        }
        b'x' => {
            o.create_new(true);
        }
        _ => {}
    }
    match o.open(os_path(file_path)) {
        Ok(f) => {
            let mut s = Stream::new(Backend::File(f), mode, path);
            s.readable = readable;
            s.writable = writable;
            Some(s)
        }
        Err(e) => {
            open_failed(fname, path, &strerror(&e));
            None
        }
    }
}

fn resource(s: Stream) -> Value {
    Value::Resource(stream::new_resource(s))
}

fn opt_int(fname: &str, n: usize, param: &str, v: Option<Value>, default: i64) -> EResult<i64> {
    match v {
        None => Ok(default),
        Some(v) => arg_int(fname, n, param, &v),
    }
}

fn opt_bool(fname: &str, n: usize, param: &str, v: Option<Value>) -> EResult<bool> {
    match v {
        None => Ok(false),
        Some(v) => arg_bool(fname, n, param, &v),
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "fopen" => {
            let [path, mode, ..] = args::<2, 4>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let mode = arg_str(rt, lname, 2, "mode", &mode.unwrap())?;
            if path.is_empty() {
                return Err(Fatal::value_error("Path must not be empty"));
            }
            stream::ensure_context();
            let mode = String::from_utf8_lossy(&mode).into_owned();
            Ok(open(lname, &path, &mode).map_or(Value::Bool(false), resource))
        }
        "tmpfile" => {
            args::<0, 0>(name, vals)?;
            let dir = std::env::temp_dir();
            for i in 0..100u32 {
                let p = dir.join(format!("php{:06x}{i}", std::process::id()));
                if let Ok(f) = OpenOptions::new().read(true).write(true).create_new(true).open(&p) {
                    // Gone once closed, like an unlinked temporary file
                    let _ = fs::remove_file(&p);
                    let mut s = Stream::new(Backend::File(f), "r+b", p.as_os_str().as_encoded_bytes());
                    s.readable = true;
                    s.writable = true;
                    return Ok(resource(s));
                }
            }
            Ok(Value::Bool(false))
        }
        "fclose" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            r.stream.borrow_mut().take();
            // Output goes nowhere once standard output is closed
            if r.id == 2 {
                rt.close_stdout();
            }
            Ok(Value::Bool(true))
        }
        "fread" => {
            let [f, len] = args::<2, 2>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let len = arg_int(lname, 2, "length", &len.unwrap())?;
            if len <= 0 {
                return Err(Fatal::value_error("fread(): Argument #2 ($length) must be greater than 0"));
            }
            Ok(match with_stream(&r, |s| s.read(len.min(1 << 31) as usize)) {
                Ok(b) => Value::str(b),
                Err(e) => read_notice(lname, e)?,
            })
        }
        "fwrite" | "fputs" => {
            let [f, data, len] = args::<2, 3>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let mut data = arg_str(rt, lname, 2, "data", &data.unwrap())?;
            if let Some(l) = len
                && !l.is_null()
            {
                let l = arg_int(lname, 3, "length", &l)?;
                data.truncate(l.max(0) as usize);
            }
            if data.is_empty() {
                return Ok(Value::Int(0));
            }
            let mut out = Out(rt);
            Ok(match with_stream(&r, |s| s.write(&mut out, &data)) {
                Ok(n) => Value::Int(n as i64),
                Err(e) => write_notice(lname, data.len(), e),
            })
        }
        "fgets" => {
            let [f, len] = args::<1, 2>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let max = match len {
                Some(l) if !l.is_null() => {
                    let l = arg_int(lname, 2, "length", &l)?;
                    if l <= 0 {
                        return Err(Fatal::value_error("fgets(): Argument #2 ($length) must be greater than 0"));
                    }
                    if l == 1 {
                        return Ok(Value::Bool(false));
                    }
                    Some((l - 1) as usize)
                }
                _ => None,
            };
            Ok(match with_stream(&r, |s| s.read_line(max)) {
                Ok(Some(l)) => Value::str(l),
                Ok(None) => Value::Bool(false),
                Err(e) => read_notice(lname, e)?,
            })
        }
        "fgetc" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            Ok(match with_stream(&r, |s| s.read(1)) {
                Ok(b) if !b.is_empty() => Value::str(b),
                Ok(_) => Value::Bool(false),
                Err(e) => read_notice(lname, e)?,
            })
        }
        "feof" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            Ok(Value::Bool(with_stream(&r, |s| s.eof)))
        }
        "fflush" | "fsync" | "fdatasync" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let mut out = Out(rt);
            let ok = with_stream(&r, |s| {
                if lname != "fflush" && s.file().is_none() {
                    diag::warning(format!("{lname}(): Can't fsync this stream!"));
                    return false;
                }
                s.flush(&mut out)
            });
            Ok(Value::Bool(ok))
        }
        "ftell" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            Ok(with_stream(&r, |s| s.tell()).map_or(Value::Bool(false), |p| Value::Int(p as i64)))
        }
        "fseek" => {
            let [f, offset, whence] = args::<2, 3>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let offset = arg_int(lname, 2, "offset", &offset.unwrap())?;
            let whence = opt_int(lname, 3, "whence", whence, 0)?;
            let ok = with_stream(&r, |s| {
                if !s.is_seekable() {
                    diag::warning(format!("{lname}(): Stream does not support seeking"));
                    return false;
                }
                s.seek(offset, whence)
            });
            Ok(Value::Int(if ok { 0 } else { -1 }))
        }
        "rewind" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            Ok(Value::Bool(with_stream(&r, |s| s.seek(0, 0))))
        }
        "ftruncate" => {
            let [f, size] = args::<2, 2>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let size = arg_int(lname, 2, "size", &size.unwrap())?;
            if size < 0 {
                return Err(Fatal::value_error("ftruncate(): Argument #2 ($size) must be greater than or equal to 0"));
            }
            Ok(Value::Bool(with_stream(&r, |s| {
                if !s.is_seekable() {
                    diag::warning("ftruncate(): Can't truncate this stream!");
                    return false;
                }
                s.truncate(size as u64)
            })))
        }
        "fstat" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            Ok(with_stream(&r, |s| match s.file() {
                Some(file) => file.metadata().ok().map(|m| stat_array(&m)),
                None => s.size().map(memory_stat),
            })
            .unwrap_or(Value::Bool(false)))
        }
        "flock" => {
            let [f, op, _] = args::<2, 3>(name, vals)?;
            arg_stream(lname, 1, "stream", &f.unwrap())?;
            let op = arg_int(lname, 2, "operation", &op.unwrap())?;
            if !(1..=3).contains(&(op & 3)) {
                return Err(Fatal::value_error("flock(): Argument #2 ($operation) must be one of LOCK_SH, LOCK_EX, or LOCK_UN"));
            }
            Ok(Value::Bool(true))
        }
        "fpassthru" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            Ok(match with_stream(&r, |s| s.read_all()) {
                Ok(b) => {
                    rt.echo(&b);
                    Value::Int(b.len() as i64)
                }
                Err(e) => read_notice(lname, e)?,
            })
        }
        "stream_get_contents" => {
            let [f, len, offset] = args::<1, 3>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let len = match len {
                Some(l) if !l.is_null() => Some(arg_int(lname, 2, "length", &l)?),
                _ => None,
            };
            if let Some(l) = len
                && l < -1
            {
                return Err(Fatal::value_error("stream_get_contents(): Argument #2 ($length) must be greater than or equal to -1"));
            }
            let offset = opt_int(lname, 3, "offset", offset, -1)?;
            if offset >= 0 && !with_stream(&r, |s| s.seek(offset, 0)) {
                diag::warning(format!("{lname}(): Failed to seek to position {offset} in the stream"));
                return Ok(Value::Bool(false));
            }
            Ok(match with_stream(&r, |s| match len {
                Some(l) if l >= 0 => s.read(l as usize),
                _ => s.read_all(),
            }) {
                Ok(b) => Value::str(b),
                Err(e) => read_notice(lname, e)?,
            })
        }
        "stream_get_meta_data" => {
            let [f] = args::<1, 1>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            Ok(with_stream(&r, |s| {
                let mut a = PhpArray::new();
                let mut put = |k: &str, v: Value| a.insert(Key::Str(k.as_bytes().into()), v);
                put("timed_out", Value::Bool(false));
                put("blocked", Value::Bool(true));
                put("eof", Value::Bool(s.eof));
                put("wrapper_type", Value::str(s.wrapper_type));
                put("stream_type", Value::str(s.stream_type));
                put("mode", Value::str(s.mode.clone()));
                put("unread_bytes", Value::Int(0));
                put("seekable", Value::Bool(s.is_seekable()));
                put("uri", Value::str(s.uri.clone()));
                Value::Array(Rc::new(a))
            }))
        }
        "stream_is_local" => {
            let [f] = args::<1, 1>(name, vals)?;
            Ok(Value::Bool(match f.unwrap() {
                Value::Resource(_) => true,
                v => !arg_str(rt, lname, 1, "stream", &v)?.windows(3).any(|w| w == b"://")
                    || v.to_bytes().starts_with(b"file://"),
            }))
        }
        "fprintf" | "vfprintf" => {
            let mut vals = vals;
            if vals.is_empty() {
                return Err(super::arg_count(name, 2, 0));
            }
            let f = vals.remove(0);
            let r = arg_stream(lname, 1, "stream", &f)?;
            if vals.is_empty() {
                return Err(super::arg_count(name, 2, 1));
            }
            let format = arg_str(rt, lname, 2, "format", &vals.remove(0))?;
            let text = if lname == "fprintf" {
                super::format::formatted_print(rt, lname, &format, &vals, Some(2))?
            } else {
                let [values] = args::<1, 1>(name, vals)?;
                let arr = expect_array(lname, 3, "values", &values.unwrap())?;
                let values: Vec<Value> = arr.values().map(Value::copy_out).collect();
                super::format::formatted_print(rt, lname, &format, &values, None)?
            };
            let mut out = Out(rt);
            Ok(match with_stream(&r, |s| s.write(&mut out, &text)) {
                Ok(n) => Value::Int(n as i64),
                Err(e) => write_notice(lname, text.len(), e),
            })
        }
        "fputcsv" => {
            let [f, fields, sep, enc, esc, eol] = args::<2, 6>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let fields = expect_array(lname, 2, "fields", &fields.unwrap())?;
            let sep = csv_char(rt, lname, 3, "separator", sep, b',')?;
            let enc = csv_char(rt, lname, 4, "enclosure", enc, b'"')?;
            let esc = csv_escape(rt, lname, 5, esc)?;
            let eol = match eol {
                Some(v) if !v.is_null() => arg_str(rt, lname, 6, "eol", &v)?,
                _ => b"\n".to_vec(),
            };
            let mut line = Vec::new();
            let count = fields.len();
            for (i, v) in fields.values().enumerate() {
                let field = rt.stringify(&v.copy_out())?;
                csv_field(&mut line, &field, sep, enc, esc);
                if i + 1 != count {
                    line.push(sep);
                }
            }
            line.extend_from_slice(&eol);
            let mut out = Out(rt);
            Ok(match with_stream(&r, |s| s.write(&mut out, &line)) {
                Ok(n) => Value::Int(n as i64),
                Err(e) => write_notice(lname, line.len(), e),
            })
        }
        "fgetcsv" => {
            let [f, len, sep, enc, esc] = args::<1, 5>(name, vals)?;
            let r = arg_stream(lname, 1, "stream", &f.unwrap())?;
            let len = match len {
                Some(l) if !l.is_null() => arg_int(lname, 2, "length", &l)?,
                _ => 0,
            };
            let sep = csv_char(rt, lname, 3, "separator", sep, b',')?;
            let enc = csv_char(rt, lname, 4, "enclosure", enc, b'"')?;
            let esc = csv_escape(rt, lname, 5, esc)?;
            if len < 0 || len == i64::MAX {
                return Err(Fatal::value_error(format!(
                    "fgetcsv(): Argument #2 ($length) must be between 0 and {}",
                    i64::MAX - 1
                )));
            }
            let max = (len > 0).then_some(len as usize);
            let line = match with_stream(&r, |s| s.read_line(max)) {
                Ok(Some(l)) => l,
                Ok(None) => return Ok(Value::Bool(false)),
                Err(e) => return read_notice(lname, e),
            };
            let mut next = || with_stream(&r, |s| s.read_line(None)).ok().flatten();
            Ok(csv_value(parse_csv(line, sep, enc, esc, &mut next)))
        }
        "str_getcsv" => {
            let [s, sep, enc, esc] = args::<1, 4>(name, vals)?;
            let s = arg_str(rt, lname, 1, "string", &s.unwrap())?;
            let sep = csv_char(rt, lname, 2, "separator", sep, b',')?;
            let enc = csv_char(rt, lname, 3, "enclosure", enc, b'"')?;
            let esc = csv_escape(rt, lname, 4, esc)?;
            Ok(csv_value(parse_csv(s, sep, enc, esc, &mut || None)))
        }
        "is_resource" => {
            let [v] = args::<1, 1>(name, vals)?;
            Ok(Value::Bool(matches!(v.unwrap(), Value::Resource(r) if r.is_open())))
        }
        "get_resource_type" | "get_resource_id" => {
            let [v] = args::<1, 1>(name, vals)?;
            match v.unwrap() {
                Value::Resource(r) => Ok(if lname == "get_resource_id" { Value::Int(r.id) } else { Value::str(r.type_name()) }),
                other => Err(Fatal::type_error(format!(
                    "{lname}(): Argument #1 ($resource) must be of type resource, {} given",
                    other.value_name()
                ))),
            }
        }
        "sys_get_temp_dir" => {
            args::<0, 0>(name, vals)?;
            let d = std::env::temp_dir();
            let s = d.to_string_lossy();
            Ok(Value::str(s.trim_end_matches('/').to_string()))
        }
        "tempnam" => {
            let [dir, prefix] = args::<2, 2>(name, vals)?;
            let dir = arg_path(rt, lname, 1, "directory", &dir.unwrap())?;
            let prefix = arg_path(rt, lname, 2, "prefix", &prefix.unwrap())?;
            let prefix = String::from_utf8_lossy(&crate::builtins::file::basename(&prefix, b"")).chars().take(63).collect::<String>();
            let mut base = os_path(&dir);
            if dir.is_empty() || !base.is_dir() {
                if !dir.is_empty() {
                    diag::notice(format!("{lname}(): file created in the system's temporary directory"));
                }
                base = std::env::temp_dir();
            }
            for i in 0..1000u32 {
                let n = ((std::process::id() as u64) << 20) ^ ((i as u64) * 7919) ^ nanos();
                let p = base.join(format!("{prefix}{:06}", n % 1_000_000));
                if OpenOptions::new().write(true).create_new(true).open(&p).is_ok() {
                    let p = fs::canonicalize(&p).unwrap_or(p);
                    return Ok(Value::str(p.to_string_lossy().into_owned()));
                }
            }
            Ok(Value::Bool(false))
        }
        "file_get_contents" => {
            let [path, _, _, offset, maxlen] = args::<1, 5>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let offset = opt_int(lname, 4, "offset", offset, 0)?;
            let maxlen = match maxlen {
                Some(m) if !m.is_null() => {
                    let m = arg_int(lname, 5, "length", &m)?;
                    if m < 0 {
                        return Err(Fatal::value_error("file_get_contents(): Argument #5 ($length) must be greater than or equal to 0"));
                    }
                    Some(m as usize)
                }
                _ => None,
            };
            if path.is_empty() {
                return Err(Fatal::value_error("Path must not be empty"));
            }
            stream::ensure_context();
            let Some(mut s) = open(lname, &path, "rb") else { return Ok(Value::Bool(false)) };
            stream::alloc_id();
            if offset != 0 && !s.seek(offset, if offset > 0 { 0 } else { 2 }) {
                diag::warning(format!("{lname}(): Failed to seek to position {offset} in the stream"));
                return Ok(Value::Bool(false));
            }
            Ok(match maxlen {
                Some(m) => s.read(m),
                None => s.read_all(),
            }
            .map_or_else(|e| read_notice(lname, e), |b| Ok(Value::str(b)))?)
        }
        "file_put_contents" => {
            let [path, data, flags, _] = args::<2, 4>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let data = data.unwrap();
            let flags = opt_int(lname, 3, "flags", flags, 0)?;
            if path.is_empty() {
                return Err(Fatal::value_error("Path must not be empty"));
            }
            let mode = if flags & 8 != 0 { "ab" } else if flags & 2 != 0 { "cb" } else { "wb" };
            if flags & 2 != 0 && flags & 8 == 0 && path.windows(3).any(|w| w == b"://") && !path.starts_with(b"file://") {
                diag::warning(format!("{lname}(): Exclusive locks may only be set for regular files"));
                return Ok(Value::Bool(false));
            }
            stream::ensure_context();
            let Some(mut s) = open(lname, &path, mode) else { return Ok(Value::Bool(false)) };
            stream::alloc_id();
            if mode == "cb" {
                s.truncate(0);
            }
            let bytes: Option<Vec<u8>> = match &data {
                Value::Resource(r) => {
                    let r = arg_stream(lname, 2, "data", &Value::Resource(r.clone()))?;
                    with_stream(&r, |src| src.read_all()).ok()
                }
                Value::Array(a) => {
                    let mut out = Vec::new();
                    for v in a.values() {
                        out.extend_from_slice(&rt.stringify(&v.copy_out())?);
                    }
                    Some(out)
                }
                Value::Object(o) if o.class.find_method("__tostring").is_none() => None,
                other => Some(rt.stringify(other)?),
            };
            let Some(bytes) = bytes else { return Ok(Value::Bool(false)) };
            if bytes.is_empty() {
                return Ok(Value::Int(0));
            }
            let mut out = Out(rt);
            Ok(match s.write(&mut out, &bytes) {
                Ok(n) => Value::Int(n as i64),
                Err(e) => {
                    diag::notice(format!("{lname}(): Write of {} bytes failed with errno={} {}", bytes.len(), e.errno, e.msg));
                    Value::Bool(false)
                }
            })
        }
        "file" => {
            let [path, flags, _] = args::<1, 3>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let flags = opt_int(lname, 2, "flags", flags, 0)?;
            if flags & !(1 | 2 | 4 | 16) != 0 {
                return Err(Fatal::value_error("file(): Argument #2 ($flags) must be a valid flag value"));
            }
            if path.is_empty() {
                return Err(Fatal::value_error("Path must not be empty"));
            }
            stream::ensure_context();
            let Some(mut s) = open(lname, &path, "rb") else { return Ok(Value::Bool(false)) };
            stream::alloc_id();
            let data = match s.read_all() {
                Ok(d) => d,
                Err(e) => return read_notice(lname, e),
            };
            Ok(list(file_lines(&data, flags & 2 == 0, flags & 4 != 0).into_iter().map(Value::str)))
        }
        "readfile" => {
            let [path, ..] = args::<1, 3>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            if path.is_empty() {
                return Err(Fatal::value_error("Path must not be empty"));
            }
            stream::ensure_context();
            let Some(mut s) = open(lname, &path, "rb") else { return Ok(Value::Bool(false)) };
            stream::alloc_id();
            Ok(match s.read_all() {
                Ok(b) => {
                    rt.echo(&b);
                    Value::Int(b.len() as i64)
                }
                Err(e) => read_notice(lname, e)?,
            })
        }
        "unlink" => {
            let [path, _] = args::<1, 2>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let p = os_path(&path);
            Ok(fs_result(fs::remove_file(&p), || format!("{lname}({})", show(&path))))
        }
        "mkdir" => {
            let [path, mode, recursive, _] = args::<1, 4>(name, vals)?;
            let path = arg_path(rt, lname, 1, "directory", &path.unwrap())?;
            let mode = opt_int(lname, 2, "permissions", mode, 0o777)?;
            let recursive = opt_bool(lname, 3, "recursive", recursive)?;
            let p = os_path(&path);
            let mut b = fs::DirBuilder::new();
            use std::os::unix::fs::DirBuilderExt;
            b.mode(mode as u32 & 0o7777).recursive(recursive);
            if recursive && p.exists() {
                diag::warning(format!("{lname}(): File exists"));
                return Ok(Value::Bool(false));
            }
            Ok(fs_result(b.create(&p), || format!("{lname}()")))
        }
        "rmdir" => {
            let [path, _] = args::<1, 2>(name, vals)?;
            let path = arg_path(rt, lname, 1, "directory", &path.unwrap())?;
            Ok(fs_result(fs::remove_dir(os_path(&path)), || format!("{lname}({})", show(&path))))
        }
        "rename" => {
            let [from, to, _] = args::<2, 3>(name, vals)?;
            let from = arg_path(rt, lname, 1, "from", &from.unwrap())?;
            let to = arg_path(rt, lname, 2, "to", &to.unwrap())?;
            Ok(fs_result(fs::rename(os_path(&from), os_path(&to)), || format!("{lname}({},{})", show(&from), show(&to))))
        }
        "copy" => {
            let [from, to, _] = args::<2, 3>(name, vals)?;
            let from = arg_path(rt, lname, 1, "from", &from.unwrap())?;
            let to = arg_path(rt, lname, 2, "to", &to.unwrap())?;
            stream::ensure_context();
            if os_path(&from).is_dir() {
                diag::warning(format!("{lname}(): The first argument to copy() function cannot be a directory"));
                return Ok(Value::Bool(false));
            }
            if os_path(&to).is_dir() {
                diag::warning(format!("{lname}(): The second argument to copy() function cannot be a directory"));
                return Ok(Value::Bool(false));
            }
            let Some(mut src) = open(lname, &from, "rb") else { return Ok(Value::Bool(false)) };
            stream::alloc_id();
            let Some(mut dst) = open(lname, &to, "wb") else { return Ok(Value::Bool(false)) };
            stream::alloc_id();
            let data = match src.read_all() {
                Ok(d) => d,
                Err(e) => return read_notice(lname, e),
            };
            let mut out = Out(rt);
            Ok(Value::Bool(data.is_empty() || dst.write(&mut out, &data).is_ok()))
        }
        "touch" => {
            let [path, mtime, atime] = args::<1, 3>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let mtime = match mtime {
                Some(v) if !v.is_null() => Some(arg_int(lname, 2, "mtime", &v)?),
                _ => None,
            };
            let atime = match atime {
                Some(v) if !v.is_null() => Some(arg_int(lname, 3, "atime", &v)?),
                _ => mtime,
            };
            let p = os_path(&path);
            let f = match OpenOptions::new().append(true).create(true).open(&p) {
                Ok(f) => f,
                Err(e) => {
                    diag::warning(format!("{lname}(): Unable to create file {} because {}", show(&path), strerror(&e)));
                    return Ok(Value::Bool(false));
                }
            };
            let to_time = |t: Option<i64>| match t {
                Some(t) => std::time::UNIX_EPOCH + std::time::Duration::from_secs(t.max(0) as u64),
                None => std::time::SystemTime::now(),
            };
            let times = fs::FileTimes::new().set_modified(to_time(mtime)).set_accessed(to_time(atime));
            Ok(match f.set_times(times) {
                Ok(()) => Value::Bool(true),
                Err(e) => {
                    diag::warning(format!("{lname}(): Utime failed: {}", strerror(&e)));
                    Value::Bool(false)
                }
            })
        }
        "chmod" => {
            let [path, mode] = args::<2, 2>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let mode = arg_int(lname, 2, "permissions", &mode.unwrap())?;
            use std::os::unix::fs::PermissionsExt;
            Ok(fs_result(fs::set_permissions(os_path(&path), fs::Permissions::from_mode(mode as u32 & 0o7777)), || {
                format!("{lname}()")
            }))
        }
        "umask" => {
            args::<0, 1>(name, vals)?;
            Ok(Value::Int(0o022))
        }
        "clearstatcache" => {
            args::<0, 2>(name, vals)?;
            Ok(Value::Null)
        }
        "filesize" | "filemtime" | "fileatime" | "filectime" | "fileperms" | "fileinode" | "fileowner" | "filegroup"
        | "filetype" | "stat" | "lstat" => {
            let [path] = args::<1, 1>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let p = os_path(&path);
            let m = if matches!(lname, "lstat" | "filetype") { fs::symlink_metadata(&p) } else { fs::metadata(&p) };
            let m = match m {
                Ok(m) if !path.is_empty() => m,
                _ => {
                    let l = if lname == "lstat" || lname == "filetype" { "L" } else { "" };
                    diag::warning(format!("{lname}(): {l}stat failed for {}", show(&path)));
                    return Ok(Value::Bool(false));
                }
            };
            use std::os::unix::fs::MetadataExt;
            Ok(match lname {
                "filesize" => Value::Int(m.size() as i64),
                "filemtime" => Value::Int(m.mtime()),
                "fileatime" => Value::Int(m.atime()),
                "filectime" => Value::Int(m.ctime()),
                "fileperms" => Value::Int(m.mode() as i64),
                "fileinode" => Value::Int(m.ino() as i64),
                "fileowner" => Value::Int(m.uid() as i64),
                "filegroup" => Value::Int(m.gid() as i64),
                "filetype" => {
                    let t = m.file_type();
                    use std::os::unix::fs::FileTypeExt;
                    Value::str(if t.is_symlink() {
                        "link"
                    } else if t.is_dir() {
                        "dir"
                    } else if t.is_file() {
                        "file"
                    } else if t.is_fifo() {
                        "fifo"
                    } else if t.is_char_device() {
                        "char"
                    } else if t.is_block_device() {
                        "block"
                    } else {
                        "socket"
                    })
                }
                _ => stat_array(&m),
            })
        }
        "is_writable" | "is_writeable" | "is_executable" | "is_link" => {
            let [path] = args::<1, 1>(name, vals)?;
            let path = arg_path(rt, lname, 1, "filename", &path.unwrap())?;
            let p = os_path(&path);
            if path.is_empty() {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(match lname {
                "is_link" => fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()),
                "is_executable" => access(&p, 1),
                _ => access(&p, 2),
            }))
        }
        "opendir" => {
            let [path, _] = args::<1, 2>(name, vals)?;
            let path = arg_path(rt, lname, 1, "directory", &path.unwrap())?;
            stream::ensure_context();
            Ok(match read_dir(&path) {
                Ok(entries) => {
                    let mut s = Stream::new(Backend::Dir { entries, pos: 0 }, "r", &path);
                    s.readable = true;
                    resource(s)
                }
                Err(e) => {
                    diag::warning(format!("{lname}({}): Failed to open directory: {}", show(&path), strerror(&e)));
                    Value::Bool(false)
                }
            })
        }
        "readdir" | "closedir" | "rewinddir" => {
            let [d] = args::<0, 1>(name, vals)?;
            let Some(d) = d else {
                return Err(Fatal::type_error(format!("{lname}(): No resource supplied")));
            };
            let r = match &d {
                Value::Resource(r) if r.is_open() && with_stream(r, |s| matches!(s.backend, Backend::Dir { .. })) => r.clone(),
                Value::Resource(_) => {
                    return Err(Fatal::type_error(format!("{lname}(): Argument #1 ($dir_handle) must be an open stream resource")));
                }
                other => {
                    return Err(Fatal::type_error(format!(
                        "{lname}(): Argument #1 ($dir_handle) must be of type resource or null, {} given",
                        other.value_name()
                    )));
                }
            };
            Ok(match lname {
                "readdir" => with_stream(&r, |s| s.next_entry()).map_or(Value::Bool(false), Value::str),
                "rewinddir" => {
                    with_stream(&r, |s| s.rewind_dir());
                    Value::Null
                }
                _ => {
                    r.stream.borrow_mut().take();
                    Value::Null
                }
            })
        }
        "scandir" => {
            let [path, order, _] = args::<1, 3>(name, vals)?;
            let path = arg_path(rt, lname, 1, "directory", &path.unwrap())?;
            let order = opt_int(lname, 2, "sorting_order", order, 0)?;
            if path.is_empty() {
                return Err(Fatal::value_error("scandir(): Argument #1 ($directory) must not be empty"));
            }
            stream::ensure_context();
            let mut entries = match read_dir(&path) {
                Ok(e) => e,
                Err(e) => {
                    diag::warning(format!("{lname}({}): Failed to open directory: {}", show(&path), strerror(&e)));
                    diag::warning(format!("{lname}(): (errno {}): {}", e.raw_os_error().unwrap_or(0), strerror(&e)));
                    return Ok(Value::Bool(false));
                }
            };
            stream::alloc_id();
            match order {
                0 => entries.sort(),
                2 => {}
                _ => {
                    entries.sort();
                    entries.reverse();
                }
            }
            Ok(list(entries.into_iter().map(Value::str)))
        }
        "glob" => {
            let [pattern, flags] = args::<1, 2>(name, vals)?;
            let pattern = arg_path(rt, lname, 1, "pattern", &pattern.unwrap())?;
            let flags = opt_int(lname, 2, "flags", flags, 0)?;
            Ok(list(glob(&pattern, flags).into_iter().map(Value::str)))
        }
        "fnmatch" => {
            let [pattern, s, flags] = args::<2, 3>(name, vals)?;
            let pattern = arg_path(rt, lname, 1, "pattern", &pattern.unwrap())?;
            let s = arg_path(rt, lname, 2, "filename", &s.unwrap())?;
            let flags = opt_int(lname, 3, "flags", flags, 0)?;
            Ok(Value::Bool(fnmatch(&pattern, &s, flags)))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}

fn nanos() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as u64)
}

/// true, or a warning with strerror and false
fn fs_result<T>(r: io::Result<T>, prefix: impl FnOnce() -> String) -> Value {
    match r {
        Ok(_) => Value::Bool(true),
        Err(e) => {
            diag::warning(format!("{}: {}", prefix(), strerror(&e)));
            Value::Bool(false)
        }
    }
}

/// access(2), by trying what it checks
fn access(p: &std::path::Path, what: u32) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(m) = fs::metadata(p) else { return false };
    if what == 2 {
        if m.is_dir() {
            return m.permissions().mode() & 0o222 != 0;
        }
        return OpenOptions::new().append(true).open(p).is_ok();
    }
    m.permissions().mode() & 0o111 != 0
}

/// The entries of a directory, with "." and ".." as readdir gives them
fn read_dir(path: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let mut out = vec![b".".to_vec(), b"..".to_vec()];
    for e in fs::read_dir(os_path(path))? {
        out.push(e?.file_name().as_encoded_bytes().to_vec());
    }
    Ok(out)
}

fn stat_array(m: &fs::Metadata) -> Value {
    use std::os::unix::fs::MetadataExt;
    let vals = [
        ("dev", m.dev() as i64),
        ("ino", m.ino() as i64),
        ("mode", m.mode() as i64),
        ("nlink", m.nlink() as i64),
        ("uid", m.uid() as i64),
        ("gid", m.gid() as i64),
        ("rdev", m.rdev() as i64),
        ("size", m.size() as i64),
        ("atime", m.atime()),
        ("mtime", m.mtime()),
        ("ctime", m.ctime()),
        ("blksize", m.blksize() as i64),
        ("blocks", m.blocks() as i64),
    ];
    stat_from(&vals)
}

/// fstat of a memory stream, as php-src's memory stream reports it
fn memory_stat(size: u64) -> Value {
    let vals = [
        ("dev", 12),
        ("ino", 0),
        ("mode", 0o100666),
        ("nlink", 1),
        ("uid", 0),
        ("gid", 0),
        ("rdev", -1),
        ("size", size as i64),
        ("atime", 0),
        ("mtime", 0),
        ("ctime", 0),
        ("blksize", -1),
        ("blocks", -1),
    ];
    stat_from(&vals)
}

fn stat_from(vals: &[(&str, i64)]) -> Value {
    let mut a = PhpArray::new();
    for (i, (_, v)) in vals.iter().enumerate() {
        a.insert(Key::Int(i as i64), Value::Int(*v));
    }
    for (k, v) in vals {
        a.insert(Key::Str(k.as_bytes().into()), Value::Int(*v));
    }
    Value::Array(Rc::new(a))
}

/// php_fputcsv's quoting of one field
fn csv_field(out: &mut Vec<u8>, field: &[u8], sep: u8, enc: u8, esc: Option<u8>) {
    let needs = field.iter().any(|&c| c == sep || c == enc || Some(c) == esc || matches!(c, b'\n' | b'\r' | b'\t' | b' '));
    if !needs {
        out.extend_from_slice(field);
        return;
    }
    out.push(enc);
    let mut escaped = false;
    for &c in field {
        if Some(c) == esc {
            escaped = true;
        } else if !escaped && c == enc {
            out.push(enc);
        } else {
            escaped = false;
        }
        out.push(c);
    }
    out.push(enc);
}

/// file()'s split into lines
fn file_lines(data: &[u8], keep_newlines: bool, skip_empty: bool) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut s = 0;
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\n' {
            if keep_newlines {
                out.push(data[s..=i].to_vec());
            } else {
                let end = if i > s && data[i - 1] == b'\r' { i - 1 } else { i };
                // Skipping empty lines only applies without the newlines (as in php-src)
                if !(skip_empty && end == s) {
                    out.push(data[s..end].to_vec());
                }
            }
            s = i + 1;
        }
        i += 1;
    }
    if s < data.len() {
        out.push(data[s..].to_vec());
    }
    out
}

/// fnmatch(3) with FNM_NOESCAPE (2), FNM_PATHNAME (1), FNM_PERIOD (4), FNM_CASEFOLD (16)
pub fn fnmatch(p: &[u8], s: &[u8], flags: i64) -> bool {
    fn go(p: &[u8], s: &[u8], flags: i64, start: bool) -> bool {
        let pathname = flags & 1 != 0;
        let period = flags & 4 != 0;
        let fold = |c: u8| if flags & 16 != 0 { c.to_ascii_lowercase() } else { c };
        let leading_dot = |s: &[u8], start: bool| period && start && s.first() == Some(&b'.');
        let (mut pi, mut si) = (0, 0);
        let mut at_start = start;
        while pi < p.len() {
            match p[pi] {
                b'*' => {
                    if leading_dot(&s[si..], at_start) {
                        return false;
                    }
                    let rest = &p[pi + 1..];
                    for k in si..=s.len() {
                        if pathname && s[si..k].contains(&b'/') {
                            break;
                        }
                        if go(rest, &s[k..], flags, false) {
                            return true;
                        }
                    }
                    return false;
                }
                b'?' => {
                    if si >= s.len() || pathname && s[si] == b'/' || leading_dot(&s[si..], at_start) {
                        return false;
                    }
                    si += 1;
                    pi += 1;
                }
                b'[' => {
                    if si >= s.len() || pathname && s[si] == b'/' || leading_dot(&s[si..], at_start) {
                        return false;
                    }
                    let mut j = pi + 1;
                    let neg = matches!(p.get(j), Some(b'!' | b'^'));
                    if neg {
                        j += 1;
                    }
                    let mut hit = false;
                    let mut first = true;
                    while j < p.len() && (p[j] != b']' || first) {
                        first = false;
                        let lo = p[j];
                        if p.get(j + 1) == Some(&b'-') && p.get(j + 2).is_some_and(|&c| c != b']') {
                            let hi = p[j + 2];
                            if fold(lo) <= fold(s[si]) && fold(s[si]) <= fold(hi) {
                                hit = true;
                            }
                            j += 3;
                        } else {
                            if fold(lo) == fold(s[si]) {
                                hit = true;
                            }
                            j += 1;
                        }
                    }
                    if j >= p.len() {
                        // No closing bracket: a literal [
                        if s[si] != b'[' {
                            return false;
                        }
                        si += 1;
                        pi += 1;
                        continue;
                    }
                    if hit == neg {
                        return false;
                    }
                    si += 1;
                    pi = j + 1;
                }
                c => {
                    let (c, adv) = if c == b'\\' && flags & 2 == 0 && pi + 1 < p.len() { (p[pi + 1], 2) } else { (c, 1) };
                    if si >= s.len() || fold(s[si]) != fold(c) {
                        return false;
                    }
                    at_start = pathname && c == b'/';
                    si += 1;
                    pi += adv;
                    continue;
                }
            }
            at_start = false;
        }
        si == s.len()
    }
    go(p, s, flags, true)
}

/// glob(3) for the flags PHP passes through
fn glob(pattern: &[u8], flags: i64) -> Vec<Vec<u8>> {
    let patterns = if flags & 1024 != 0 { expand_braces(pattern) } else { vec![pattern.to_vec()] };
    let mut out = Vec::new();
    for p in &patterns {
        let mut found = glob_one(p, flags);
        if flags & 4 == 0 {
            found.sort();
        }
        if found.is_empty() && flags & 16 != 0 {
            found.push(p.clone());
        }
        out.extend(found);
    }
    out
}

fn has_magic(s: &[u8]) -> bool {
    s.iter().any(|c| matches!(c, b'*' | b'?' | b'['))
}

fn glob_one(pattern: &[u8], flags: i64) -> Vec<Vec<u8>> {
    let absolute = pattern.first() == Some(&b'/');
    let parts: Vec<&[u8]> = pattern.split(|&c| c == b'/').filter(|p| !p.is_empty()).collect();
    let mut current: Vec<Vec<u8>> = vec![if absolute { b"/".to_vec() } else { Vec::new() }];
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let mut next = Vec::new();
        for base in &current {
            let join = |name: &[u8]| {
                let mut p = base.clone();
                if !p.is_empty() && !p.ends_with(b"/") {
                    p.push(b'/');
                }
                p.extend_from_slice(name);
                p
            };
            if !has_magic(part) {
                let p = join(&unescape(part, flags));
                if fs::symlink_metadata(os_path(&p)).is_ok() {
                    next.push(p);
                }
                continue;
            }
            let dir = if base.is_empty() { b".".to_vec() } else { base.clone() };
            let Ok(rd) = fs::read_dir(os_path(&dir)) else { continue };
            let mut names: Vec<Vec<u8>> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().as_encoded_bytes().to_vec()).collect();
            names.sort();
            for name in names {
                if name.first() == Some(&b'.') && part.first() != Some(&b'.') {
                    continue;
                }
                if fnmatch(part, &name, if flags & 64 != 0 { 2 } else { 0 }) {
                    next.push(join(&name));
                }
            }
        }
        if !last {
            next.retain(|p| os_path(p).is_dir());
        }
        current = next;
    }
    if parts.is_empty() {
        return Vec::new();
    }
    if flags & (1 << 13) != 0 {
        current.retain(|p| os_path(p).is_dir());
    }
    if flags & 2 != 0 {
        for p in &mut current {
            if os_path(p).is_dir() {
                p.push(b'/');
            }
        }
    }
    current
}

fn unescape(s: &[u8], flags: i64) -> Vec<u8> {
    if flags & 64 != 0 {
        return s.to_vec();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' && i + 1 < s.len() {
            i += 1;
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// GLOB_BRACE: `a{b,c}d` is `abd` and `acd`
fn expand_braces(p: &[u8]) -> Vec<Vec<u8>> {
    let Some(open) = p.iter().position(|&c| c == b'{') else { return vec![p.to_vec()] };
    let mut depth = 0;
    let mut close = None;
    let mut commas = Vec::new();
    for (i, &c) in p.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            b',' if depth == 1 => commas.push(i),
            _ => {}
        }
    }
    let Some(close) = close else { return vec![p.to_vec()] };
    let mut bounds = vec![open];
    bounds.extend(commas);
    bounds.push(close);
    let mut out = Vec::new();
    for w in bounds.windows(2) {
        let mut q = p[..open].to_vec();
        q.extend_from_slice(&p[w[0] + 1..w[1]]);
        q.extend_from_slice(&p[close + 1..]);
        out.extend(expand_braces(&q));
    }
    out
}

/// php_fgetcsv_lookup_trailing_spaces: where the line ends, before a final
/// "\n", "\r\n" or "\r"
fn csv_line_end(buf: &[u8]) -> usize {
    match buf {
        [.., b'\r', b'\n'] => buf.len() - 2,
        [.., b'\n' | b'\r'] => buf.len() - 1,
        _ => buf.len(),
    }
}

/// php_fgetcsv: the fields of one CSV record. An enclosure left open takes
/// more lines from `next_line` (None for str_getcsv). None for a blank line
pub fn parse_csv(
    mut buf: Vec<u8>,
    delim: u8,
    enc: u8,
    esc: Option<u8>,
    next_line: &mut dyn FnMut() -> Option<Vec<u8>>,
) -> Option<Vec<Vec<u8>>> {
    // C strings: reading at the end gives the terminating NUL
    let at = |buf: &[u8], i: usize| buf.get(i).copied().unwrap_or(0);
    let slice = |buf: &[u8], a: usize, b: usize| -> Vec<u8> {
        let b = b.min(buf.len());
        if a >= b { Vec::new() } else { buf[a..b].to_vec() }
    };
    let mut limit = csv_line_end(&buf);
    let mut line_end = limit;
    let mut line_end_bytes = buf[limit..].to_vec();
    let mut bptr = 0;
    let mut first_field = true;
    let mut values = Vec::new();
    loop {
        let mut temp = Vec::new();
        let mut inc = usize::from(bptr < limit);
        if inc == 1 {
            let mut tmp = bptr;
            while at(&buf, tmp) != delim && matches!(at(&buf, tmp), b' ' | b'\t'..=b'\r') {
                tmp += 1;
            }
            if at(&buf, tmp) == enc && tmp < limit {
                bptr = tmp;
            }
        }
        if first_field && bptr == line_end {
            return None;
        }
        first_field = false;
        let comp_end;
        if inc != 0 && at(&buf, bptr) == enc {
            let mut state = 0;
            bptr += 1;
            let mut hunk = bptr;
            loop {
                if inc == 0 {
                    if state == 2 {
                        temp.extend(slice(&buf, hunk, bptr.saturating_sub(1)));
                        hunk = bptr;
                        break;
                    }
                    if state == 1 {
                        temp.extend(slice(&buf, hunk, bptr));
                        hunk = bptr;
                    }
                    if hunk != line_end {
                        temp.extend(slice(&buf, hunk, bptr));
                        hunk = bptr;
                    }
                    // The line end is part of the field
                    temp.extend_from_slice(&line_end_bytes);
                    match next_line() {
                        None => {
                            if bptr > limit {
                                if hunk == bptr {
                                    hunk -= 1;
                                }
                                bptr -= 1;
                            }
                            break;
                        }
                        Some(nb) => {
                            buf = nb;
                            bptr = 0;
                            hunk = 0;
                            limit = csv_line_end(&buf);
                            line_end = limit;
                            line_end_bytes = buf[limit..].to_vec();
                            state = 0;
                        }
                    }
                } else {
                    match state {
                        1 => {
                            bptr += 1;
                            state = 0;
                        }
                        2 => {
                            if at(&buf, bptr) != enc {
                                temp.extend(slice(&buf, hunk, bptr - 1));
                                hunk = bptr;
                                break;
                            }
                            temp.extend(slice(&buf, hunk, bptr));
                            bptr += 1;
                            hunk = bptr;
                            state = 0;
                        }
                        _ => {
                            if at(&buf, bptr) == enc {
                                state = 2;
                            } else if Some(at(&buf, bptr)) == esc {
                                state = 1;
                            }
                            bptr += 1;
                        }
                    }
                }
                inc = usize::from(bptr < limit);
            }
            // Anything up to the delimiter belongs to the field too
            while inc != 0 && at(&buf, bptr) != delim {
                bptr += 1;
                inc = usize::from(bptr < limit);
            }
            temp.extend(slice(&buf, hunk, bptr));
            bptr += inc;
            comp_end = temp.len();
        } else {
            let hunk = bptr;
            while inc != 0 && at(&buf, bptr) != delim {
                bptr += 1;
                inc = usize::from(bptr < limit);
            }
            temp.extend(slice(&buf, hunk, bptr));
            comp_end = csv_line_end(&temp);
            if at(&buf, bptr) == delim {
                bptr += 1;
            }
        }
        temp.truncate(comp_end);
        values.push(temp);
        if inc == 0 {
            break;
        }
    }
    Some(values)
}

/// The `$escape` argument of the CSV functions: deprecated when left out (PHP 8.4)
pub fn csv_escape(rt: &mut dyn Runtime, fname: &str, n: usize, v: Option<Value>) -> EResult<Option<u8>> {
    match v {
        None => {
            diag::deprecated(format!("{fname}(): the $escape parameter must be provided as its default value will change"));
            Ok(Some(b'\\'))
        }
        Some(v) => {
            let s = arg_str(rt, fname, n, "escape", &v)?;
            if s.len() > 1 {
                return Err(Fatal::value_error(format!("{fname}(): Argument #{n} ($escape) must be empty or a single character")));
            }
            Ok(s.first().copied())
        }
    }
}

/// The separator or enclosure argument of the CSV functions
pub fn csv_char(rt: &mut dyn Runtime, fname: &str, n: usize, param: &str, v: Option<Value>, default: u8) -> EResult<u8> {
    let Some(v) = v else { return Ok(default) };
    let s = arg_str(rt, fname, n, param, &v)?;
    if s.len() != 1 {
        return Err(Fatal::value_error(format!("{fname}(): Argument #{n} (${param}) must be a single character")));
    }
    Ok(s[0])
}

pub fn csv_value(fields: Option<Vec<Vec<u8>>>) -> Value {
    match fields {
        Some(f) => list(f.into_iter().map(Value::str)),
        None => list([Value::Null]),
    }
}
