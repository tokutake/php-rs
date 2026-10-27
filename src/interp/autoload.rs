//! Class autoloading: spl_autoload_register() and friends, and the class
//! lookup that runs the registered loaders for an unknown class

use std::io::Write;
use std::rc::Rc;

use super::{builtins, Callable, EResult, Fatal, Interp};
use crate::array::PhpArray;
use crate::ast::IncludeKind;
use crate::diag;
use crate::object::ClassEntry;
use crate::value::Value;

pub(super) const NAMES: &[&str] = &[
    "spl_autoload_register", "spl_autoload_unregister", "spl_autoload_functions", "spl_autoload_call", "spl_autoload",
    "spl_autoload_extensions",
];

/// The share of the call depth limit one level of autoloading takes
const AUTOLOAD_WEIGHT: usize = 3;

/// What makes two registrations the same loader, as php-src compares them
#[derive(PartialEq)]
enum LoaderId {
    Closure(usize),
    Function(String),
    /// Lowercased class and method, and the object for an instance method
    Method(String, String, Option<usize>),
}

pub(super) struct Loader {
    callable: Value,
    id: LoaderId,
    /// As spl_autoload_functions() lists it
    shown: Value,
}

#[derive(Default)]
pub(super) struct Autoload {
    loaders: Vec<Loader>,
    /// Lowercased names of the classes being loaded: asking for one of them
    /// again while its loaders run finds nothing instead of recursing
    loading: Vec<String>,
    /// For spl_autoload(); None is the default ".inc,.php"
    extensions: Option<String>,
}

/// zend_is_valid_class_name
fn valid_class_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80)
}

impl<W: Write> Interp<W> {
    /// A class by name, running the autoloaders when it is not declared yet
    pub(super) fn find_class(&mut self, name: &str) -> EResult<Option<Rc<ClassEntry>>> {
        let name = name.strip_prefix('\\').unwrap_or(name);
        let lname = name.to_ascii_lowercase();
        if let Some(c) = self.classes.get(&lname) {
            return Ok(Some(c.clone()));
        }
        if self.autoload.loaders.is_empty() || !valid_class_name(name) || self.autoload.loading.contains(&lname) {
            return Ok(None);
        }
        self.autoload.loading.push(lname.clone());
        // Loading from a loader nests natively through the built-in that asked
        // for the class, so it weighs more towards the depth limit than a call
        self.nesting += AUTOLOAD_WEIGHT;
        let result = self.run_loaders(name, &lname);
        self.nesting -= AUTOLOAD_WEIGHT;
        self.autoload.loading.retain(|l| *l != lname);
        result?;
        Ok(self.classes.get(&lname).cloned())
    }

    /// Calls the loaders in order until one of them declares the class
    fn run_loaders(&mut self, name: &str, lname: &str) -> EResult<()> {
        let mut i = 0;
        while let Some(f) = self.autoload.loaders.get(i).map(|l| l.callable.clone()) {
            let f = self.callback("spl_autoload_call", 1, &f)?;
            self.call_value(&f, vec![Value::str(name)], diag::line())?;
            if self.classes.contains_key(lname) {
                break;
            }
            i += 1;
        }
        Ok(())
    }

    fn loader(&mut self, f: &Callable) -> (LoaderId, Value) {
        match f {
            Callable::Closure(o) => (LoaderId::Closure(Rc::as_ptr(o) as usize), Value::Object(o.clone())),
            Callable::User(f) => (LoaderId::Function(f.name.to_ascii_lowercase()), Value::str(f.name.as_str())),
            Callable::Builtin(n) => (LoaderId::Function(n.clone()), Value::str(n.as_str())),
            Callable::Method { this, method, class } => {
                let id = LoaderId::Method(
                    class.name.to_ascii_lowercase(),
                    method.name.to_ascii_lowercase(),
                    this.as_ref().filter(|_| !method.is_static).map(|o| Rc::as_ptr(o) as usize),
                );
                let target = match this {
                    Some(o) if !method.is_static => Value::Object(o.clone()),
                    _ => Value::str(class.name),
                };
                (id, super::list(vec![target, Value::str(method.name.as_str())]))
            }
        }
    }

    pub(super) fn autoload_builtin(&mut self, lname: &str, vals: Vec<Value>) -> EResult<Value> {
        match lname {
            "spl_autoload_register" => {
                let [callback, throw, prepend] = builtins::args::<0, 3>(lname, vals)?;
                if let Some(t) = throw
                    && !t.to_bool()
                {
                    diag::notice(
                        "spl_autoload_register(): Argument #2 ($do_throw) has been ignored, spl_autoload_register() will always throw",
                    );
                }
                let callback = match callback.map(|c| c.copy_out()) {
                    None | Some(Value::Null) => Value::str("spl_autoload"),
                    Some(c) => c,
                };
                let Some(f) = self.callable(&callback)? else {
                    let detail = self.not_callable_reason(&callback)?;
                    return Err(Fatal::type_error(format!(
                        "spl_autoload_register(): Argument #1 ($callback) must be a valid callback or null, {detail}"
                    )));
                };
                let (id, shown) = self.loader(&f);
                if !self.autoload.loaders.iter().any(|l| l.id == id) {
                    let loader = Loader { callable: callback, id, shown };
                    if prepend.is_some_and(|p| p.to_bool()) {
                        self.autoload.loaders.insert(0, loader);
                    } else {
                        self.autoload.loaders.push(loader);
                    }
                }
                Ok(Value::Bool(true))
            }
            "spl_autoload_unregister" => {
                let [callback] = builtins::args::<1, 1>(lname, vals)?;
                let callback = callback.unwrap().copy_out();
                let Some(f) = self.callable(&callback)? else {
                    let detail = self.not_callable_reason(&callback)?;
                    return Err(Fatal::type_error(format!(
                        "spl_autoload_unregister(): Argument #1 ($callback) must be a valid callback, {detail}"
                    )));
                };
                let (id, _) = self.loader(&f);
                if id == LoaderId::Function("spl_autoload_call".into()) {
                    // Unregisters everything, as php-src does
                    self.autoload.loaders.clear();
                    return Ok(Value::Bool(true));
                }
                let before = self.autoload.loaders.len();
                self.autoload.loaders.retain(|l| l.id != id);
                Ok(Value::Bool(self.autoload.loaders.len() != before))
            }
            "spl_autoload_functions" => {
                builtins::args::<0, 0>(lname, vals)?;
                Ok(Value::Array(Rc::new(PhpArray::from_values(self.autoload.loaders.iter().map(|l| l.shown.clone())))))
            }
            "spl_autoload_call" => {
                let [name] = builtins::args::<1, 1>(lname, vals)?;
                let name = builtins::arg_str(self, lname, 1, "class", &name.unwrap())?;
                self.find_class(&String::from_utf8_lossy(&name))?;
                Ok(Value::Null)
            }
            "spl_autoload_extensions" => {
                let [ext] = builtins::args::<0, 1>(lname, vals)?;
                if let Some(ext) = ext {
                    let ext = builtins::arg_str(self, lname, 1, "file_extensions", &ext)?;
                    self.autoload.extensions = Some(String::from_utf8_lossy(&ext).into_owned());
                }
                Ok(Value::str(self.autoload.extensions.clone().unwrap_or_else(|| ".inc,.php".into())))
            }
            _ => {
                // spl_autoload: the class name, lowercased and with `\` as `/`,
                // plus each extension, from the include path
                let [class, ext] = builtins::args::<1, 2>(lname, vals)?;
                let class = builtins::arg_str(self, lname, 1, "class", &class.unwrap())?;
                let ext = match ext {
                    Some(Value::Null) | None => self.autoload.extensions.clone().unwrap_or_else(|| ".inc,.php".into()),
                    Some(e) => String::from_utf8_lossy(&builtins::arg_str(self, lname, 2, "file_extensions", &e)?).into_owned(),
                };
                let class = String::from_utf8_lossy(&class).to_ascii_lowercase();
                let base = class.replace('\\', "/");
                for e in ext.split(',') {
                    let path = format!("{base}{e}");
                    if self.resolve_include(&path).is_some() {
                        self.include(IncludeKind::RequireOnce, &path, diag::line())?;
                        if self.classes.contains_key(&class) {
                            break;
                        }
                    }
                }
                Ok(Value::Null)
            }
        }
    }
}
