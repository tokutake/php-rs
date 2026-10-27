//! Path functions, and the read-only file system functions that go with
//! include: file_exists(), is_file(), realpath(), file_get_contents(), ...

use std::rc::Rc;

use super::{arg_int, arg_str, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "basename", "dirname", "pathinfo", "file_exists", "is_file", "is_dir", "is_readable", "realpath", "getcwd", "chdir",
];

pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "PATHINFO_DIRNAME" => 1,
        "PATHINFO_BASENAME" => 2,
        "PATHINFO_EXTENSION" => 4,
        "PATHINFO_FILENAME" => 8,
        "PATHINFO_ALL" => 15,
        _ => return None,
    }))
}

/// php_basename: the last component, without trailing slashes or `suffix`
pub fn basename(path: &[u8], suffix: &[u8]) -> Vec<u8> {
    let end = path.iter().rposition(|&c| c != b'/').map_or(0, |i| i + 1);
    let trimmed = &path[..end];
    let start = trimmed.iter().rposition(|&c| c == b'/').map_or(0, |i| i + 1);
    let base = &trimmed[start..];
    if !suffix.is_empty() && base.len() > suffix.len() && base.ends_with(suffix) {
        return base[..base.len() - suffix.len()].to_vec();
    }
    base.to_vec()
}

/// zend_dirname
pub fn dirname(path: &[u8]) -> Vec<u8> {
    if path.is_empty() {
        return Vec::new();
    }
    let Some(end) = path.iter().rposition(|&c| c != b'/') else { return b"/".to_vec() };
    let Some(slash) = path[..=end].iter().rposition(|&c| c == b'/') else { return b".".to_vec() };
    match path[..slash].iter().rposition(|&c| c != b'/') {
        Some(e) => path[..=e].to_vec(),
        None => b"/".to_vec(),
    }
}

/// A file name argument, which cannot contain NUL bytes
fn path_arg(rt: &mut dyn Runtime, fname: &str, v: &Value) -> EResult<Vec<u8>> {
    let param = match fname { "realpath" => "path", "chdir" => "directory", _ => "filename" };
    let p = arg_str(rt, fname, 1, param, v)?;
    if p.contains(&0) {
        return Err(Fatal::value_error(format!("{fname}(): Argument #1 (${param}) must not contain any null bytes")));
    }
    Ok(p)
}

fn os_path(p: &[u8]) -> std::path::PathBuf {
    std::path::PathBuf::from(String::from_utf8_lossy(p).into_owned())
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "basename" => {
            let [p, suffix] = args::<1, 2>(name, vals)?;
            let p = arg_str(rt, lname, 1, "path", &p.unwrap())?;
            let suffix = match suffix {
                Some(s) => arg_str(rt, lname, 2, "suffix", &s)?,
                None => Vec::new(),
            };
            Ok(Value::str(basename(&p, &suffix)))
        }
        "dirname" => {
            let [p, levels] = args::<1, 2>(name, vals)?;
            let mut p = arg_str(rt, lname, 1, "path", &p.unwrap())?;
            let levels = match levels {
                Some(l) => arg_int(lname, 2, "levels", &l)?,
                None => 1,
            };
            if levels < 1 {
                return Err(Fatal::value_error("dirname(): Argument #2 ($levels) must be greater than or equal to 1"));
            }
            for _ in 0..levels {
                let next = dirname(&p);
                if next == p {
                    break;
                }
                p = next;
            }
            Ok(Value::str(p))
        }
        "pathinfo" => {
            let [p, flags] = args::<1, 2>(name, vals)?;
            let p = arg_str(rt, lname, 1, "path", &p.unwrap())?;
            let flags = match flags {
                Some(f) => arg_int(lname, 2, "flags", &f)?,
                None => 15,
            };
            let mut out = PhpArray::new();
            let put = |out: &mut PhpArray, k: &str, v: Vec<u8>| out.insert(Key::Str(k.as_bytes().into()), Value::str(v));
            if flags & 1 != 0 {
                let d = dirname(&p);
                if p.contains(&b'/') || !p.is_empty() {
                    put(&mut out, "dirname", d);
                }
            }
            let base = basename(&p, b"");
            if flags & 2 != 0 {
                put(&mut out, "basename", base.clone());
            }
            let dot = base.iter().rposition(|&c| c == b'.');
            if flags & 4 != 0
                && let Some(d) = dot
            {
                put(&mut out, "extension", base[d + 1..].to_vec());
            }
            if flags & 8 != 0 {
                put(&mut out, "filename", dot.map_or(base.clone(), |d| base[..d].to_vec()));
            }
            // A single element asked for comes as a string
            if flags != 15 {
                return Ok(out.values().next().cloned().unwrap_or(Value::str("")));
            }
            Ok(Value::Array(Rc::new(out)))
        }
        "file_exists" | "is_file" | "is_dir" | "is_readable" => {
            let [p] = args::<1, 1>(name, vals)?;
            let p = path_arg(rt, lname, &p.unwrap())?;
            let path = os_path(&p);
            Ok(Value::Bool(!p.is_empty() && match lname {
                "is_file" => path.is_file(),
                "is_dir" => path.is_dir(),
                "is_readable" => std::fs::File::open(&path).is_ok() || path.is_dir(),
                _ => path.exists(),
            }))
        }
        "realpath" => {
            let [p] = args::<1, 1>(name, vals)?;
            let p = path_arg(rt, lname, &p.unwrap())?;
            let p = if p.is_empty() { b".".to_vec() } else { p };
            Ok(match std::fs::canonicalize(os_path(&p)) {
                Ok(r) => Value::str(r.display().to_string()),
                Err(_) => Value::Bool(false),
            })
        }
        "getcwd" => {
            args::<0, 0>(name, vals)?;
            Ok(match std::env::current_dir() {
                Ok(d) => Value::str(d.display().to_string()),
                Err(_) => Value::Bool(false),
            })
        }
        "chdir" => {
            let [p] = args::<1, 1>(name, vals)?;
            let p = path_arg(rt, lname, &p.unwrap())?;
            Ok(match std::env::set_current_dir(os_path(&p)) {
                Ok(()) => Value::Bool(true),
                Err(e) => {
                    let errno = e.raw_os_error().unwrap_or(0);
                    crate::diag::warning(format!("chdir(): {} (errno {errno})", super::streams::strerror(&e)));
                    Value::Bool(false)
                }
            })
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
