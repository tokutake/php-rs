//! The process environment and the runtime: getenv(), putenv(), php_sapi_name(), ...

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::{arg_str, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::interp::{EResult, Fatal};
use crate::value::Value;

pub const NAMES: &[&str] = &["getenv", "putenv", "php_sapi_name", "phpversion", "php_uname", "getmypid"];

thread_local! {
    /// putenv() changes, over the process environment. Changing the real one
    /// is unsafe while other threads may read it, so the script gets its own view
    static OVERRIDES: RefCell<HashMap<Vec<u8>, Option<Vec<u8>>>> = RefCell::new(HashMap::new());
}

pub fn constant(name: &str) -> Option<Value> {
    match name {
        "PHP_SAPI" => Some(Value::str("cli")),
        _ => None,
    }
}

fn env_var(name: &[u8]) -> Option<Vec<u8>> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    if let Some(v) = OVERRIDES.with(|o| o.borrow().get(name).cloned()) {
        return v;
    }
    std::env::var_os(std::ffi::OsStr::from_bytes(name)).map(OsStringExt::into_vec)
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "getenv" => {
            let [n, _local] = args::<0, 2>(name, vals)?;
            match n {
                None | Some(Value::Null) => {
                    use std::os::unix::ffi::OsStrExt;
                    let mut arr = PhpArray::new();
                    for (k, v) in std::env::vars_os() {
                        arr.insert(Key::from_bytes(k.as_bytes()), Value::str(v.as_bytes().to_vec()));
                    }
                    OVERRIDES.with(|o| {
                        for (k, v) in o.borrow().iter() {
                            match v {
                                Some(v) => arr.insert(Key::from_bytes(k), Value::str(v.clone())),
                                None => {
                                    arr.remove(&Key::from_bytes(k));
                                }
                            }
                        }
                    });
                    Ok(Value::Array(Rc::new(arr)))
                }
                Some(n) => {
                    let n = arg_str(rt, lname, 1, "name", &n)?;
                    Ok(env_var(&n).map_or(Value::Bool(false), Value::str))
                }
            }
        }
        "putenv" => {
            let [a] = args::<1, 1>(name, vals)?;
            let a = arg_str(rt, lname, 1, "assignment", &a.unwrap())?;
            let (k, v) = match a.iter().position(|&c| c == b'=') {
                Some(i) => (a[..i].to_vec(), Some(a[i + 1..].to_vec())),
                None => (a.clone(), None),
            };
            if k.is_empty() {
                return Err(Fatal::value_error("putenv(): Argument #1 ($assignment) must have a valid syntax"));
            }
            OVERRIDES.with(|o| o.borrow_mut().insert(k, v));
            Ok(Value::Bool(true))
        }
        "php_sapi_name" => {
            args::<0, 0>(name, vals)?;
            Ok(Value::str("cli"))
        }
        "phpversion" => {
            let [ext] = args::<0, 1>(name, vals)?;
            Ok(match ext {
                None | Some(Value::Null) => Value::str("8.5.0"),
                Some(_) => Value::Bool(false),
            })
        }
        "getmypid" => {
            args::<0, 0>(name, vals)?;
            Ok(Value::Int(std::process::id() as i64))
        }
        "php_uname" => {
            let [mode] = args::<0, 1>(name, vals)?;
            let mode = match mode {
                Some(m) => arg_str(rt, lname, 1, "mode", &m)?,
                None => b"a".to_vec(),
            };
            let sys = std::env::consts::OS;
            let sys = format!("{}{}", sys[..1].to_ascii_uppercase(), &sys[1..]);
            let machine = std::env::consts::ARCH;
            Ok(Value::str(match mode.first() {
                Some(b's') => sys,
                Some(b'm') => machine.to_string(),
                Some(b'n') | Some(b'r') | Some(b'v') => String::new(),
                _ => format!("{sys} {machine}"),
            }))
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
