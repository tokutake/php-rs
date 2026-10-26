//! Tree-walking interpreter that executes the AST directly.
//! Class and object handling lives in the `oop` submodule.

pub(crate) mod generator;
mod closure;
mod oop;
mod output;

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::rc::Rc;

use crate::array::{Key, PhpArray};
use crate::ast::{Arg, ClassDecl, Expr, ExprKind, FuncDecl, FuncKind, ListItem, Magic, Name, Program, SourceFile, Stmt, TypeHint};
use crate::builtins::{self, Runtime};
use crate::diag;
use crate::object::{self, ClassEntry, Closure, Internal, Method, Native, Object, TraceFrame};

/// Built-ins that php-src compiles to dedicated opcodes when called directly
/// with this many arguments, so they get no frame in stack traces
fn compiled_inline(lname: &str, args: &[Arg]) -> bool {
    if args.iter().any(|a| a.spread) {
        return false;
    }
    match lname {
        "strlen" | "is_null" | "is_bool" | "is_long" | "is_int" | "is_integer" | "is_float" | "is_double"
        | "is_string" | "is_array" | "is_object" | "is_resource" | "is_scalar" | "boolval" | "intval" | "floatval"
        | "doubleval" | "strval" | "defined" | "chr" | "ord" | "count" | "sizeof" | "gettype" => args.len() == 1,
        "get_class" => args.len() <= 1,
        "get_called_class" | "func_num_args" | "func_get_args" => args.is_empty(),
        "array_key_exists" | "call_user_func_array" => args.len() == 2,
        "call_user_func" => !args.is_empty(),
        _ => false,
    }
}
use crate::value::{self, float_fits_int, float_to_int_implicit, loose_cmp, parse_numeric, ArithError, Num, Numeric, Value};

thread_local! {
    static STRICT_TYPES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the script declared strict_types=1: scalar arguments, return values
/// and typed properties are then not coerced (except int to float)
pub fn strict_types() -> bool {
    STRICT_TYPES.with(|s| s.get())
}

/// PHP code recursing deeper than this gets an Error instead of overflowing the
/// native stack. The interpreter thread is given a stack large enough for it
pub const MAX_CALL_DEPTH: usize = 10_000;

/// An uncaught PHP exception (Error hierarchy), or a fatal error that cannot be caught
#[derive(Debug)]
pub struct Fatal {
    pub class: &'static str,
    pub msg: String,
    /// Where it was thrown and the stack trace, filled in as it leaves the innermost call
    pub line: Option<u32>,
    pub file: Option<Rc<str>>,
    pub trace: Option<Vec<TraceFrame>>,
    /// A user-thrown exception object, which carries its own message, line and trace
    pub object: Option<Rc<Object>>,
    /// A compile-time style error ("Fatal error: Cannot redeclare ..."), not an exception
    pub uncatchable: bool,
    /// `exit`: the script ends with this status, without being an error
    pub exit: Option<u8>,
    /// The exception's code, for exceptions raised by built-ins
    pub code: i64,
}

impl Fatal {
    pub fn new(class: &'static str, msg: impl Into<String>) -> Self {
        Fatal { class, msg: msg.into(), line: None, file: None, trace: None, object: None, uncatchable: false, exit: None, code: 0 }
    }

    fn thrown(object: Rc<Object>) -> Self {
        Fatal { object: Some(object), ..Fatal::new("", "") }
    }

    /// A fatal error that try/catch does not see
    pub fn compile(msg: impl Into<String>) -> Self {
        Fatal { uncatchable: true, ..Fatal::new("", msg) }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Fatal::new("Error", msg)
    }

    pub fn type_error(msg: impl Into<String>) -> Self {
        Fatal::new("TypeError", msg)
    }

    pub fn value_error(msg: impl Into<String>) -> Self {
        Fatal::new("ValueError", msg)
    }
}

impl From<ArithError> for Fatal {
    fn from(e: ArithError) -> Self {
        match e {
            ArithError::DivisionByZero(msg) => Fatal::new("DivisionByZeroError", msg),
            ArithError::Arithmetic(msg) => Fatal::new("ArithmeticError", msg),
            ArithError::Type(msg) => Fatal::type_error(msg),
        }
    }
}

pub type EResult<T> = Result<T, Fatal>;

enum Flow {
    Normal,
    Break(u32),
    Continue(u32),
    Return(Value),
}

type RefCellValue = Rc<RefCell<Value>>;

/// Where a write lands before any array offsets are applied
#[derive(Clone)]
enum Base<'e> {
    /// A variable of the current frame, named in the AST
    Var(&'e str),
    /// A global variable, through `$GLOBALS['name']`
    Global(String),
    /// `$obj->name`
    Prop(Rc<Object>, String),
    /// A static property's storage, with its declared type and `Class::$name` for messages
    Cell(RefCellValue, Option<(TypeHint, String)>),
    /// `$obj[dim]` on an object, which goes through ArrayAccess. Always the
    /// last step of a path (see resolve_offsets)
    Offset(Rc<Object>, Option<Value>),
}

/// A write target: a base plus evaluated offsets (None for `[]`)
#[derive(Clone)]
struct Path<'e> {
    base: Base<'e>,
    dims: Vec<Option<Value>>,
}

impl<'e> Path<'e> {
    fn var(name: &'e str) -> Self {
        Path { base: Base::Var(name), dims: Vec::new() }
    }
}

/// An argument after evaluation: by-reference parameters receive the reference itself
enum ArgVal {
    Val(Value),
    Ref(RefCellValue),
    /// A parameter skipped by named arguments: its default applies
    Missing,
    /// A named argument without a parameter of its own, for the variadic one
    Named(String, Box<ArgVal>),
}

impl ArgVal {
    fn value(&self) -> Value {
        match self {
            ArgVal::Val(v) => v.clone(),
            ArgVal::Ref(r) => value::with_deref(r, Value::clone),
            ArgVal::Missing => Value::Null,
            ArgVal::Named(_, v) => v.value(),
        }
    }
}

/// What a call runs as: its name in messages and traces, and its class context
#[derive(Clone, Default)]
struct CallCtx {
    /// `foo`, `A::foo` or `{closure:FILE:LINE}`, as used in error messages
    name: String,
    /// The function part of a trace line
    trace_fn: String,
    /// Class and `->`/`::` of a method frame
    trace_class: Option<(String, &'static str)>,
    this: Option<Rc<Object>>,
    /// The class whose private and protected members are visible (`self`)
    scope: Option<Rc<ClassEntry>>,
    /// The late static binding class (`static`)
    static_class: Option<Rc<ClassEntry>>,
}

/// A frame's variables. Insertion-ordered, because dropping a frame destroys its
/// variables in that order (like php-src's compiled variables), and the order in
/// which objects die decides which handle numbers get reused
#[derive(Default)]
struct Vars {
    index: HashMap<String, usize>,
    slots: Vec<Option<(String, Value)>>,
}

impl Vars {
    fn get(&self, name: &str) -> Option<&Value> {
        let &i = self.index.get(name)?;
        self.slots[i].as_ref().map(|(_, v)| v)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        let &i = self.index.get(name)?;
        self.slots[i].as_mut().map(|(_, v)| v)
    }

    fn contains_key(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    fn insert(&mut self, name: String, v: Value) {
        match self.index.get(&name) {
            Some(&i) => self.slots[i].as_mut().unwrap().1 = v,
            None => {
                self.index.insert(name.clone(), self.slots.len());
                self.slots.push(Some((name, v)));
            }
        }
    }

    /// The variable's slot, created as null if missing
    fn entry(&mut self, name: &str) -> &mut Value {
        if !self.contains_key(name) {
            self.insert(name.to_string(), Value::Null);
        }
        self.get_mut(name).unwrap()
    }

    fn remove(&mut self, name: &str) {
        if let Some(i) = self.index.remove(name) {
            // The slot stays as a tombstone so the order of the others is kept
            self.slots[i] = None;
        }
    }
}

impl FromIterator<(String, Value)> for Vars {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        let mut vars = Vars::default();
        for (k, v) in iter {
            vars.insert(k, v);
        }
        vars
    }
}

/// Fields drop in declaration order, which mirrors php-src: the argument copies
/// kept for traces go first (freeing nothing while the variables still hold
/// them), then the variables in order, then $this
struct Frame {
    /// Arguments as passed, for func_get_args() and stack traces
    args: Vec<Value>,
    vars: Vars,
    /// None for the main script
    func: Option<Rc<FuncDecl>>,
    ctx: CallCtx,
    call_line: u32,
    /// Called back by a built-in function: its arguments come from internal
    /// code, which passes them in coercive mode even under strict_types
    by_builtin: bool,
    /// The file the call was made from
    call_file: Rc<str>,
}

pub struct Interp<W: Write> {
    out: W,
    /// Set by fclose(STDOUT)
    stdout_closed: bool,
    /// The file of the code running: its path as diagnostics show it, and its strict_types
    file: Rc<SourceFile>,
    frames: Vec<Frame>,
    functions: HashMap<String, Rc<FuncDecl>>,
    /// Constants from `const` and define()
    constants: HashMap<String, Value>,
    /// `static` variables, keyed by the declaring function (null for the main script) and the class
    /// scope, since each class using a trait gets its own copy of the method
    statics: HashMap<(*const FuncDecl, &'static str), HashMap<String, RefCellValue>>,
    classes: HashMap<String, Rc<ClassEntry>>,
    /// Class declarations already executed (hoisted ones are seen twice)
    declared: HashSet<*const ClassDecl>,
    /// Magic accessors running per (object id, property, kind), so that
    /// `$this->x` inside `__get('x')` reads the property instead of recursing
    magic_guard: HashSet<(u32, String, u8)>,
    /// Built-in calls in progress, with the number of PHP frames when each was
    /// made, so stack traces can show them between the PHP frames
    /// The bool marks an include, which is no built-in to the code it runs
    internal_calls: Vec<(usize, TraceFrame, bool)>,
    /// Destructors called at the end of the script show as called internally
    in_shutdown: bool,
    /// Destructor calls and includes in progress: they nest natively without
    /// adding to the frame count, so they count towards the depth limit separately
    nesting: usize,
    /// Generators being run, innermost last
    gen_runs: Vec<generator::GenRun>,
    /// Parsed defaults of built-in parameters, by source text
    builtin_defaults: HashMap<&'static str, Rc<Expr>>,
    /// Files included so far, for include_once and require_once
    included: HashSet<String>,
    /// Included files' code by path, parsed once like php-src's compiled file cache.
    /// Their functions and classes refer to it
    programs: HashMap<String, Rc<Program>>,
    /// Output buffers from ob_start(), innermost last
    ob: Vec<output::OutBuffer>,
    /// An output handler is running: its own output is dropped
    ob_running: bool,
    /// A class implementing ArrayAccess exists. Until then write paths need
    /// not look for objects, which only fail as arrays
    array_access_declared: bool,
}

fn undefined_constant(name: &str) -> Fatal {
    Fatal::error(format!("Undefined constant \"{name}\""))
}

/// Constants are case-sensitive, except for their namespace part
fn const_key(name: &str) -> std::borrow::Cow<'_, str> {
    match name.rfind('\\') {
        Some(i) => format!("{}{}", name[..i].to_ascii_lowercase(), &name[i..]).into(),
        None => name.into(),
    }
}

/// The process environment as an array, as $_ENV and $_SERVER start with it
fn env_array() -> PhpArray {
    use std::os::unix::ffi::OsStrExt;
    let mut arr = PhpArray::new();
    for (k, v) in std::env::vars_os() {
        arr.insert(Key::from_bytes(k.as_bytes()), Value::str(v.as_bytes().to_vec()));
    }
    arr
}

fn recursive_reference() -> Fatal {
    Fatal::error("Cannot modify a value through a reference to itself")
}

impl<W: Write> Runtime for Interp<W> {
    fn stringify(&mut self, v: &Value) -> EResult<Vec<u8>> {
        Interp::stringify(self, v)
    }

    fn call_method0(&mut self, obj: &Rc<Object>, name: &str) -> EResult<Value> {
        self.call_named_method(obj, name)
    }

    fn echo(&mut self, bytes: &[u8]) {
        let _ = self.write(bytes);
    }

    fn echo_direct(&mut self, bytes: &[u8]) {
        self.flush_diags();
        let _ = self.raw_out(bytes);
    }

    fn flush_out(&mut self) {
        self.flush_diags();
        let _ = self.out.flush();
    }

    fn close_stdout(&mut self) {
        self.flush_out();
        self.stdout_closed = true;
    }

    fn check_callback(&mut self, fname: &str, n: usize, cb: &Value) -> EResult<()> {
        self.callback(fname, n, cb).map(|_| ())
    }

    fn is_callable(&mut self, cb: &Value) -> bool {
        matches!(self.callable(cb), Ok(Some(_)))
    }

    fn call_callback(&mut self, cb: &Value, args: Vec<Value>) -> EResult<Value> {
        let f = self.callback("call_user_func", 1, cb)?;
        self.call_value(&f, args, diag::line())
    }

    fn new_object(&mut self, class: &str) -> Option<Rc<Object>> {
        let class = self.classes.get(&class.to_ascii_lowercase())?.clone();
        Some(Object::new(class, crate::object::Internal::None))
    }
}

impl<W: Write> Interp<W> {
    pub fn new(out: W, file: Rc<SourceFile>) -> Self {
        let main = Frame {
            vars: Vars::default(),
            func: None,
            ctx: CallCtx::default(),
            call_line: 0,
            args: Vec::new(),
            by_builtin: false,
            call_file: file.path.clone(),
        };
        Interp {
            out,
            stdout_closed: false,
            file,
            frames: vec![main],
            functions: HashMap::new(),
            constants: HashMap::new(),
            statics: HashMap::new(),
            classes: object::builtin_classes(),
            declared: HashSet::new(),
            magic_guard: HashSet::new(),
            internal_calls: Vec::new(),
            in_shutdown: false,
            nesting: 0,
            gen_runs: Vec::new(),
            builtin_defaults: HashMap::new(),
            included: HashSet::new(),
            programs: HashMap::new(),
            ob: Vec::new(),
            ob_running: false,
            array_access_declared: false,
        }
    }

    pub fn into_output(self) -> W {
        self.out
    }

    /// Switches to the code of another file, returning the one left
    fn enter_file(&mut self, file: &Rc<SourceFile>) -> Rc<SourceFile> {
        if Rc::ptr_eq(&self.file, file) {
            return file.clone();
        }
        STRICT_TYPES.with(|s| s.set(file.strict));
        diag::set_file(file.path.clone());
        std::mem::replace(&mut self.file, file.clone())
    }

    /// Runs a script, giving the exit status. An uncaught error is reported the
    /// way PHP does, with status 255
    pub fn run(&mut self, program: &Program) -> u8 {
        self.included.insert(program.file.path.to_string());
        self.run_main(program)
    }

    /// include / require: runs another file in the current scope. The result is
    /// the file's `return` value, else 1; false when an include cannot open it
    fn include(&mut self, kind: crate::ast::IncludeKind, path: &str, line: u32) -> EResult<Value> {
        use crate::ast::IncludeKind;
        const INCLUDE_PATH: &str = ".:/usr/share/php";
        let fname = kind.name();
        let require = matches!(kind, IncludeKind::Require | IncludeKind::RequireOnce);
        let Some(found) = self.resolve_include(path) else {
            diag::warning(format!("{fname}({path}): Failed to open stream: No such file or directory"));
            if require {
                return Err(Fatal::error(format!("Failed opening required '{path}' (include_path='{INCLUDE_PATH}')")));
            }
            diag::warning(format!("{fname}(): Failed opening '{path}' for inclusion (include_path='{INCLUDE_PATH}')"));
            return Ok(Value::Bool(false));
        };
        let shown = found.display().to_string();
        if matches!(kind, IncludeKind::IncludeOnce | IncludeKind::RequireOnce) && self.included.contains(&shown) {
            return Ok(Value::Bool(true));
        }
        self.included.insert(shown.clone());
        let program = match self.programs.get(&shown) {
            Some(p) => p.clone(),
            None => {
                let p = self.load(fname, path, &found, &shown)?;
                let Some(p) = p else { return Ok(Value::Bool(false)) };
                self.programs.insert(shown.clone(), p.clone());
                p
            }
        };
        // The include shows in stack traces like a call.
        // Its argument (the file) only shows when the trace continues inside it
        let args = vec![Value::str(shown.clone())];
        let frame = TraceFrame { file: self.file.path.to_string(), line, function: fname.into(), class: None, args };
        self.internal_calls.push((self.frames.len(), frame, true));
        if self.frames.len() + self.nesting > MAX_CALL_DEPTH {
            self.internal_calls.pop();
            return Err(Fatal::error(
                "Maximum call stack size of 8355840 bytes (zend.max_allowed_stack_size - zend.reserved_stack_size) reached. Infinite recursion?",
            ));
        }
        let prev = self.enter_file(&program.file);
        self.nesting += 1;
        let result = self.hoist(&program.stmts).and_then(|_| self.block(&program.stmts));
        self.nesting -= 1;
        // Errors leaving the file are located while it is current
        let result = result.map_err(|mut e| {
            if e.object.is_none() && e.exit.is_none() {
                e.line = Some(e.line.unwrap_or_else(diag::line));
                e.file.get_or_insert_with(diag::file);
                if e.trace.is_none() {
                    e.trace = Some(self.trace());
                }
                if !e.uncatchable {
                    return Fatal::thrown(self.exception_object(e));
                }
            }
            e
        });
        self.enter_file(&prev);
        self.internal_calls.pop();
        Ok(match result? {
            Flow::Return(v) => v,
            _ => Value::Int(1),
        })
    }

    /// Reads and parses an included file; None (after a warning) if it cannot be read
    fn load(&mut self, fname: &str, path: &str, found: &std::path::Path, shown: &str) -> EResult<Option<Rc<Program>>> {
        let src = match std::fs::read(found) {
            Ok(s) => s,
            Err(e) => {
                diag::warning(format!("{fname}({path}): Failed to open stream: {e}"));
                return Ok(None);
            }
        };
        let parsed = crate::lexer::lex(&src)
            .map_err(|e| ("Parse error", e.msg, e.line))
            .and_then(|tokens| crate::parser::parse(tokens, shown).map_err(|e| (e.label(), e.msg, e.line)));
        match parsed {
            Ok(p) => Ok(Some(Rc::new(p))),
            Err((label, msg, at)) => {
                // A syntax error is a ParseError thrown by the include; other compile errors are fatal
                let mut e = if label == "Parse error" { Fatal::new("ParseError", msg) } else { Fatal::compile(msg) };
                e.line = Some(at);
                e.file = Some(shown.into());
                Err(e)
            }
        }
    }

    /// Finds an included file: absolute and ./ ../ paths as they are, others
    /// in the include path (the working directory) and then the directory of
    /// the including script, as php-src does
    fn resolve_include(&self, path: &str) -> Option<std::path::PathBuf> {
        use std::path::Path;
        let p = Path::new(path);
        let is_file = |p: &Path| p.is_file().then(|| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
        if p.is_absolute() || path.starts_with("./") || path.starts_with("../") {
            return is_file(p);
        }
        if let Some(found) = is_file(p) {
            return Some(found);
        }
        let dir = Path::new(&*self.file.path).parent()?;
        is_file(&dir.join(p))
    }

    fn run_main(&mut self, program: &Program) -> u8 {
        let stmts = &program.stmts;
        STRICT_TYPES.with(|s| s.set(program.file.strict));
        diag::set_file(program.file.path.clone());
        self.file = program.file.clone();
        let result = self.hoist(stmts).and_then(|_| self.block(stmts));
        let mut status = 0;
        match result {
            Ok(_) => {}
            // exit ends the script normally, destructors included
            Err(Fatal { exit: Some(s), .. }) => status = s,
            Err(e) => {
                status = 255;
                let fatal = e.uncatchable;
                self.report(e);
                // A fatal error ends the script without destructors, an uncaught exception does not
                if fatal {
                    object::disable_destructors();
                    self.ob_end_all();
                    self.flush_diags();
                    return status;
                }
            }
        }
        match self.shutdown() {
            Ok(()) => {}
            Err(Fatal { exit: Some(s), .. }) => status = s,
            Err(e) => {
                status = 255;
                self.report(e);
            }
        }
        object::disable_destructors();
        self.flush_diags();
        self.ob_end_all();
        self.flush_diags();
        status
    }

    /// Destroys what is left at the end of the script like php-src: global
    /// variables holding the only handle to an object, last first (repeated while
    /// that frees any), then all remaining objects in handle order
    fn shutdown(&mut self) -> EResult<()> {
        self.in_shutdown = true;
        self.run_destructors()?;
        loop {
            let live = |vars: &Vars| vars.slots.iter().flatten().count();
            let before = live(&self.frames[0].vars);
            for i in (0..self.frames[0].vars.slots.len()).rev() {
                let name = match &self.frames[0].vars.slots[i] {
                    Some((name, Value::Object(o))) if Rc::strong_count(o) == 1 => name.clone(),
                    _ => continue,
                };
                self.frames[0].vars.remove(&name);
                self.run_destructors()?;
            }
            if live(&self.frames[0].vars) == before {
                break;
            }
        }
        for obj in object::destructible_objects() {
            if !obj.destructed.replace(true) {
                self.destruct(obj)?;
                self.run_destructors()?;
            }
        }
        Ok(())
    }

    /// Prints an uncaught error the way PHP does
    fn report(&mut self, e: Fatal) {
        self.flush_diags();
        let msg = if e.uncatchable {
            let line = e.line.unwrap_or_else(diag::line);
            let trace = object::trace_string(&e.trace.unwrap_or_default());
            let file = e.file.unwrap_or_else(|| self.file.path.clone());
            format!("\nFatal error: {} in {file} on line {line}\nStack trace:\n{trace}\n", e.msg)
        } else {
            let ex = self.exception_object(e);
            // An uncaught ParseError (from include) reads like the parse error it is
            if ex.class.name == "ParseError" {
                let file = String::from_utf8_lossy(&ex.get("\0*\0file").to_bytes()).into_owned();
                let msg = String::from_utf8_lossy(&ex.get("\0*\0message").to_bytes()).into_owned();
                let line = ex.get("\0*\0line").to_int();
                self.emit_quiet(format!("\nParse error: {msg} in {file} on line {line}\n").as_bytes());
                return;
            }
            // Uncaught output goes through __toString, which a subclass may override
            let text = match self.stringify(&Value::Object(ex.clone())) {
                Ok(s) => s,
                Err(_) => object::throwable_to_string(&ex),
            };
            self.flush_diags();
            let file = String::from_utf8_lossy(&ex.get("\0*\0file").to_bytes()).into_owned();
            let line = ex.get("\0*\0line").to_int();
            format!("\nFatal error: Uncaught {}\n  thrown in {file} on line {line}\n", String::from_utf8_lossy(&text))
        };
        self.emit_quiet(msg.as_bytes());
    }

    /// Calls the destructors of the objects released since the last call
    fn run_destructors(&mut self) -> EResult<()> {
        // Depth first, and without native recursion: long chains of objects are
        // released one link at a time
        let mut stack = Vec::new();
        loop {
            match object::take_pending_destruct() {
                Some((obj, rest)) => {
                    stack.push(rest);
                    if let Err(e) = self.destruct(obj) {
                        while let Some(rest) = stack.pop() {
                            object::restore_pending(rest);
                        }
                        return Err(e);
                    }
                }
                None => match stack.pop() {
                    Some(rest) => object::restore_pending(rest),
                    None => return Ok(()),
                },
            }
        }
    }

    fn destruct(&mut self, obj: Rc<Object>) -> EResult<()> {
        if matches!(obj.internal, Internal::Generator(_)) {
            return self.gen_destroy(&obj);
        }
        let Some(m) = obj.class.find_method("__destruct").cloned() else { return Ok(()) };
        let line = diag::line();
        self.nesting += 1;
        let result = self.call_method(Some(obj.clone()), &m, Vec::new(), line, obj.class.clone());
        self.nesting -= 1;
        diag::set_line(line);
        result.map(drop)
    }

    /// Runs the destructors due once `result` is in. One that throws while an
    /// exception is on its way out gets that exception as its previous one
    fn with_destructors<T>(&mut self, result: EResult<T>) -> EResult<T> {
        let pending = match result {
            Ok(v) => return self.run_destructors().map(|_| v),
            Err(e) if e.uncatchable => return Err(e),
            Err(e) => e,
        };
        match self.run_destructors() {
            Ok(()) => Err(pending),
            Err(e) if e.uncatchable => Err(e),
            Err(e) => {
                let ex = self.exception_object(e);
                let previous = self.exception_object(pending);
                object::chain_previous(&ex, previous);
                Err(Fatal::thrown(ex))
            }
        }
    }

    /// Unconditional top-level functions and classes exist before any code runs.
    /// A class is hoisted once its parent and interfaces are known
    fn hoist(&mut self, stmts: &[Stmt]) -> EResult<()> {
        for s in stmts {
            if let Stmt::Function(f) = s {
                self.declare(f)?;
            }
        }
        let mut pending: Vec<&Rc<ClassDecl>> =
            stmts.iter().filter_map(|s| if let Stmt::Class(c) = s { Some(c) } else { None }).collect();
        loop {
            let before = pending.len();
            let mut rest = Vec::new();
            for c in pending {
                let lname = c.name.to_ascii_lowercase();
                let deps_known =
                    c.dependencies().iter().all(|d| self.classes.contains_key(&d.to_ascii_lowercase()));
                if deps_known && !self.classes.contains_key(&lname) {
                    if self.declare_class(c).is_ok() {
                        continue;
                    }
                    // A declaration that fails is left for runtime, where its error belongs
                    // (php-src likewise only early-binds classes that link cleanly)
                    self.classes.remove(&lname);
                    self.declared.remove(&Rc::as_ptr(c));
                    diag::take();
                }
                rest.push(c);
            }
            pending = rest;
            if pending.is_empty() || pending.len() == before {
                return Ok(());
            }
        }
    }

    fn declare(&mut self, f: &Rc<FuncDecl>) -> EResult<()> {
        let lname = f.name.to_ascii_lowercase();
        if let Some(existing) = self.functions.get(&lname) {
            // Hoisted functions are seen again when their statement executes
            if Rc::ptr_eq(existing, f) {
                return Ok(());
            }
            diag::set_line(f.line);
            return Err(Fatal::compile(format!(
                "Cannot redeclare function {}() (previously declared in {}:{})",
                f.name, existing.file.path, existing.line
            )));
        }
        if builtins::exists(&lname) {
            diag::set_line(f.line);
            return Err(Fatal::compile(format!("Cannot redeclare function {}()", f.name)));
        }
        self.functions.insert(lname, f.clone());
        Ok(())
    }

    fn flush_diags(&mut self) {
        for d in diag::take() {
            let msg = format!("\n{}: {} in {} on line {}\n", d.level.label(), d.msg, d.file, d.line);
            self.emit_quiet(msg.as_bytes());
        }
    }

    fn write(&mut self, bytes: &[u8]) -> EResult<()> {
        self.flush_diags();
        self.emit(bytes)
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    fn ctx(&self) -> &CallCtx {
        &self.frames.last().unwrap().ctx
    }

    fn block(&mut self, stmts: &[Stmt]) -> EResult<Flow> {
        for s in stmts {
            let flow = self.stmt(s)?;
            self.run_destructors()?;
            if !matches!(flow, Flow::Normal) {
                return Ok(flow);
            }
        }
        Ok(Flow::Normal)
    }

    fn stmt(&mut self, s: &Stmt) -> EResult<Flow> {
        match s {
            Stmt::Echo(exprs) => {
                for e in exprs {
                    let v = self.expr(e)?;
                    // Temporaries die before the output is written
                    self.run_destructors()?;
                    let bytes = self.stringify(&v)?;
                    self.write(&bytes)?;
                }
            }
            Stmt::Expr(e) => {
                self.expr(e)?;
            }
            Stmt::Block(body) => return self.block(body),
            Stmt::If(branches, else_body) => {
                for (cond, body) in branches {
                    if self.expr(cond)?.to_bool() {
                        return self.block(body);
                    }
                }
                if let Some(body) = else_body {
                    return self.block(body);
                }
            }
            Stmt::While(cond, body) => {
                while self.expr(cond)?.to_bool() {
                    if let Some(flow) = loop_flow(self.block(body)?) {
                        return Ok(flow);
                    }
                }
            }
            Stmt::DoWhile(body, cond) => loop {
                if let Some(flow) = loop_flow(self.block(body)?) {
                    return Ok(flow);
                }
                if !self.expr(cond)?.to_bool() {
                    break;
                }
            },
            Stmt::For(init, cond, step, body) => {
                for e in init {
                    self.expr(e)?;
                }
                loop {
                    // With multiple condition expressions, the last one decides
                    let mut ok = true;
                    for e in cond {
                        ok = self.expr(e)?.to_bool();
                    }
                    if !ok {
                        break;
                    }
                    if let Some(flow) = loop_flow(self.block(body)?) {
                        return Ok(flow);
                    }
                    for e in step {
                        self.expr(e)?;
                    }
                }
            }
            Stmt::Foreach { subject, key, value, by_ref, body } if !by_ref || !subject.is_lvalue() => {
                let v = self.expr(subject)?;
                if *by_ref && let Value::Object(o) = &v && matches!(o.internal, Internal::Period(_) | Internal::PeriodIterator(_)) {
                    return Err(Fatal::error("An iterator cannot be used with foreach by reference"));
                }
                // By reference over a temporary: nothing to write back to
                if *by_ref && let Value::Object(o) = &v && matches!(o.internal, Internal::Generator(_)) {
                    return Err(self.generator_by_ref(o));
                }
                let entries: Vec<(Value, Value)> = match v {
                    // Iterating our own Rc handle means writes to the variable inside the
                    // loop copy-on-write, so the loop sees the array as it was (PHP semantics)
                    Value::Array(arr) => {
                        return self.foreach_array(&arr, key.as_ref(), value, body);
                    }
                    Value::Object(o) if o.class.is_subclass_of("Traversable") => {
                        let Some(it) = self.iter_start(Value::Object(o), false)? else { return Ok(Flow::Normal) };
                        return self.foreach_iter(it, key.as_ref(), value, body);
                    }
                    Value::Object(o) => self.visible_props(&o),
                    other => {
                        foreach_type_warning(&other);
                        return Ok(Flow::Normal);
                    }
                };
                for (k, v) in entries {
                    if let Some(key_target) = key {
                        self.assign(key_target, k)?;
                    }
                    self.assign(value, v)?;
                    if let Some(flow) = loop_flow(self.block(body)?) {
                        return Ok(flow);
                    }
                }
            }
            Stmt::Foreach { subject, key, value, body, .. } => {
                let path = self.path(subject)?;
                let path = self.fetch_offset(path)?;
                let keys = self.with_place(&path, false, true, |slot| {
                    Ok(match slot {
                        Value::Array(arr) => Ok(arr.keys().cloned().collect::<Vec<_>>()),
                        Value::Null => {
                            // Iterating null by reference turns it into an (empty) array
                            *slot = Value::Array(Rc::new(PhpArray::new()));
                            Ok(Vec::new())
                        }
                        other => Err(other.copy_out()),
                    })
                })?;
                let keys = match keys {
                    Ok(keys) => keys,
                    Err(Value::Object(o)) if matches!(o.internal, Internal::Generator(_)) => {
                        return Err(self.generator_by_ref(&o));
                    }
                    Err(Value::Object(o)) if matches!(o.internal, Internal::Period(_) | Internal::PeriodIterator(_)) => {
                        return Err(Fatal::error("An iterator cannot be used with foreach by reference"));
                    }
                    Err(Value::Object(_)) => {
                        return Err(Fatal::error("Iterating objects by reference is not supported yet"));
                    }
                    Err(other) => {
                        foreach_type_warning(&other);
                        return Ok(Flow::Normal);
                    }
                };
                for k in keys {
                    let mut elem = path.clone();
                    elem.dims.push(Some(k.to_value()));
                    // Elements unset by the loop body are skipped
                    let exists = self.with_place(&path, false, true, |slot| {
                        Ok(matches!(slot, Value::Array(arr) if arr.contains_key(&k)))
                    })?;
                    if !exists {
                        continue;
                    }
                    if let Some(key_target) = key {
                        self.assign(key_target, k.to_value())?;
                    }
                    let r = self.make_ref(&elem)?;
                    self.bind_ref(value, r)?;
                    if let Some(flow) = loop_flow(self.block(body)?) {
                        return Ok(flow);
                    }
                }
            }
            Stmt::Switch(subject, cases) => {
                let v = self.expr(subject)?;
                let mut start = None;
                for (i, (cond, _)) in cases.iter().enumerate() {
                    if let Some(c) = cond {
                        let cv = self.expr(c)?;
                        if binary("==", &v, &cv)?.to_bool() {
                            start = Some(i);
                            break;
                        }
                    }
                }
                let Some(start) = start.or_else(|| cases.iter().position(|(c, _)| c.is_none())) else {
                    return Ok(Flow::Normal);
                };
                // Execution falls through the following cases until a break.
                // A switch counts as a loop level for break and continue
                for (_, body) in &cases[start..] {
                    match self.block(body)? {
                        Flow::Normal => {}
                        Flow::Break(1) | Flow::Continue(1) => break,
                        Flow::Break(n) => return Ok(Flow::Break(n - 1)),
                        Flow::Continue(n) => return Ok(Flow::Continue(n - 1)),
                        ret @ Flow::Return(_) => return Ok(ret),
                    }
                }
            }
            Stmt::StrictTypes => {}
            Stmt::Plain(inner) => return self.stmt(inner),
            Stmt::Yield { .. } | Stmt::YieldFrom { .. } => {
                return Err(Fatal::error("yield outside the generator executor"));
            }
            Stmt::Const(consts) => {
                for (name, e) in consts {
                    let v = self.expr(e)?;
                    self.define_const(name, v);
                }
            }
            Stmt::Break(n) => return Ok(Flow::Break(*n)),
            Stmt::Continue(n) => return Ok(Flow::Continue(*n)),
            Stmt::Unset(targets) => {
                for t in targets {
                    self.unset(t)?;
                }
            }
            Stmt::Function(f) => self.declare(f)?,
            Stmt::Class(c) => {
                if !self.declared.contains(&Rc::as_ptr(c)) {
                    self.declare_class(c)?;
                }
            }
            Stmt::Try { body, catches, finally } => {
                let mut result = self.block(body);
                if let Err(e) = result {
                    if e.uncatchable {
                        return Err(e);
                    }
                    let ex = self.exception_object(e);
                    result = Err(Fatal::thrown(ex.clone()));
                    let caught = catches.iter().find(|c| c.types.iter().any(|t| self.is_instance_of(&ex.class, t)));
                    if let Some(c) = caught {
                        result = match &c.var {
                            Some(var) => self.with_place(&Path::var(var), false, true, |slot| {
                                *slot = Value::Object(ex.clone());
                                Ok(())
                            }),
                            None => Ok(()),
                        }
                        .and_then(|_| self.block(&c.body));
                    }
                }
                if let Some(fin) = finally {
                    // A return, break or new exception in finally replaces the pending outcome
                    match self.block(fin) {
                        Ok(Flow::Normal) => {}
                        Ok(flow) => return Ok(flow),
                        Err(new) if new.uncatchable => return Err(new),
                        Err(new) => {
                            let Err(pending) = result else { return Err(new) };
                            if pending.uncatchable {
                                return Err(pending);
                            }
                            // The exception being replaced is kept as the new one's previous
                            let new = self.exception_object(new);
                            let pending = self.exception_object(pending);
                            object::chain_previous(&new, pending);
                            return Err(Fatal::thrown(new));
                        }
                    }
                }
                return result;
            }
            Stmt::Return(value) => {
                let v = match value {
                    Some(e) => self.expr(e)?,
                    None => Value::Null,
                };
                return Ok(Flow::Return(v));
            }
            Stmt::Global(names) => {
                for name in names {
                    let r = make_ref_slot(self.frames[0].vars.entry(name));
                    self.frame().vars.insert(name.clone(), Value::Ref(r));
                }
            }
            Stmt::StaticVar(vars) => {
                let func = self.frame().func.as_ref().map_or(std::ptr::null(), Rc::as_ptr);
                let key = (func, self.ctx().scope.as_ref().map_or("", |c| c.name));
                for (name, init) in vars {
                    let existing = self.statics.get(&key).and_then(|m| m.get(name)).cloned();
                    let r = match existing {
                        Some(r) => r,
                        None => {
                            let v = match init {
                                Some(e) => self.expr(e)?,
                                None => Value::Null,
                            };
                            let r = Rc::new(RefCell::new(v));
                            self.statics.entry(key).or_default().insert(name.clone(), r.clone());
                            r
                        }
                    };
                    self.frame().vars.insert(name.clone(), Value::Ref(r));
                }
            }
        }
        Ok(Flow::Normal)
    }

    fn foreach_array(&mut self, arr: &Rc<PhpArray>, key: Option<&Expr>, value: &Expr, body: &[Stmt]) -> EResult<Flow> {
        for (k, v) in arr.iter() {
            if let Some(key_target) = key {
                self.assign(key_target, k.to_value())?;
            }
            self.assign(value, v.copy_out())?;
            if let Some(flow) = loop_flow(self.block(body)?) {
                return Ok(flow);
            }
        }
        Ok(Flow::Normal)
    }

    fn lookup(&mut self, name: &str) -> Value {
        if name == "this"
            && let Some(this) = &self.ctx().this {
                return Value::Object(this.clone());
            }
        if Self::is_superglobal(name) {
            let name = self.superglobal(name);
            return self.frames[0].vars.get(&name).map_or(Value::Null, Value::copy_out);
        }
        match self.frame().vars.get(name) {
            Some(v) => v.copy_out(),
            None if name == "GLOBALS" => self.globals_array(),
            None => {
                diag::warning(format!("Undefined variable ${name}"));
                Value::Null
            }
        }
    }

    /// Variables that every scope sees as the global ones
    fn is_superglobal(name: &str) -> bool {
        name.starts_with('_')
            && matches!(name, "_SERVER" | "_GET" | "_POST" | "_COOKIE" | "_FILES" | "_ENV" | "_REQUEST")
    }

    /// A superglobal's slot name, creating $_ENV and $_REQUEST on first use as
    /// php-src does (they are "just in time" globals)
    fn superglobal(&mut self, name: &str) -> String {
        if !self.frames[0].vars.contains_key(name) {
            let v = match name {
                "_ENV" => Value::Array(Rc::new(env_array())),
                _ => Value::Array(Rc::new(PhpArray::new())),
            };
            self.frames[0].vars.insert(name.to_string(), v);
        }
        name.to_string()
    }

    /// The globals php-cli starts a script with: $_GET and friends (empty),
    /// $argv / $argc, and $_SERVER with the environment and script details
    pub fn set_globals(&mut self, argv: &[String]) {
        let empty = || Value::Array(Rc::new(PhpArray::new()));
        let args = Value::Array(Rc::new(PhpArray::from_values(argv.iter().map(|a| Value::str(a.as_str())))));
        let argc = Value::Int(argv.len() as i64);
        let script = Value::str(argv.first().cloned().unwrap_or_default());
        let mut server = env_array();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let mut put = |k: &str, v: Value| server.insert(Key::Str(k.as_bytes().into()), v);
        for k in ["PHP_SELF", "SCRIPT_NAME", "SCRIPT_FILENAME", "PATH_TRANSLATED"] {
            put(k, script.clone());
        }
        put("DOCUMENT_ROOT", Value::str(""));
        put("REQUEST_TIME_FLOAT", Value::Float(now.as_secs_f64()));
        put("REQUEST_TIME", Value::Int(now.as_secs() as i64));
        put("argv", args.clone());
        put("argc", argc.clone());
        let vars = &mut self.frames[0].vars;
        for k in ["_GET", "_POST", "_COOKIE", "_FILES"] {
            vars.insert(k.to_string(), empty());
        }
        vars.insert("argv".into(), args);
        vars.insert("argc".into(), argc);
        vars.insert("_SERVER".into(), Value::Array(Rc::new(server)));
    }

    /// `$GLOBALS` read as a whole: a copy of the global variables
    fn globals_array(&self) -> Value {
        let mut arr = PhpArray::new();
        for (name, v) in self.frames[0].vars.slots.iter().flatten() {
            arr.insert(Key::from_bytes(name.as_bytes()), v.copy_out());
        }
        Value::Array(Rc::new(arr))
    }

    /// `$GLOBALS[...]`, unless a local variable is called $GLOBALS
    fn is_globals(&self, e: &Expr) -> bool {
        matches!(&e.kind, ExprKind::Var(n) if n == "GLOBALS") && !self.frames.last().unwrap().vars.contains_key("GLOBALS")
    }

    fn expr(&mut self, e: &Expr) -> EResult<Value> {
        // Diagnostics report the line of the most recently entered node, which
        // matches the line php-src stamps on the opcode raising them
        diag::set_line(e.line);
        Ok(match &e.kind {
            ExprKind::Null => Value::Null,
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Int(i) => Value::Int(*i),
            ExprKind::Float(f) => Value::Float(*f),
            ExprKind::Str(s) => Value::Str(s.clone()),
            ExprKind::Interp(parts) => {
                let mut s = Vec::new();
                for part in parts {
                    let v = self.expr(part)?;
                    s.extend_from_slice(&self.stringify(&v)?);
                }
                Value::str(s)
            }
            ExprKind::Var(name) => self.lookup(name),
            ExprKind::Const(name) => self.constant(name.trim_start_matches('\\'))?,
            ExprKind::NsConst(name, global) => match self.lookup_const(name) {
                Some(v) => v,
                None => self.lookup_const(global).ok_or_else(|| undefined_constant(name))?,
            },
            ExprKind::Yield { .. } | ExprKind::YieldFrom(_) => {
                return Err(Fatal::error("yield outside the generator executor"));
            }
            ExprKind::Magic(Magic::File) => Value::str(self.file.path.as_bytes()),
            ExprKind::Magic(Magic::Dir) => {
                let dir = std::path::Path::new(&*self.file.path).parent().map_or(".".into(), |d| d.display().to_string());
                Value::str(dir.into_bytes())
            }
            ExprKind::Magic(Magic::ClosureName) => Value::str(self.ctx().name.as_bytes()),
            ExprKind::Magic(Magic::Class) => Value::str(self.ctx().scope.as_ref().map_or("", |c| c.name)),
            ExprKind::Hole => return Err(Fatal::compile("Cannot use empty array elements in arrays")),
            ExprKind::List(_) => return Err(Fatal::compile("Cannot use list() outside of an assignment")),
            ExprKind::Match(subject, arms) => {
                let v = self.expr(subject)?;
                for arm in arms {
                    for c in arm.conds.iter().flatten() {
                        if v.strict_eq(&self.expr(c)?) {
                            return self.expr(&arm.body);
                        }
                    }
                }
                match arms.iter().find(|a| a.conds.is_none()) {
                    Some(arm) => self.expr(&arm.body)?,
                    None => {
                        diag::set_line(e.line);
                        return Err(Fatal::new("UnhandledMatchError", format!("Unhandled match case {}", match_case(&v))));
                    }
                }
            }
            ExprKind::Array(items) => {
                let mut arr = PhpArray::new();
                for item in items {
                    let key = match &item.key {
                        Some(k) => Some(to_key(&self.expr(k)?, "array")?),
                        None => None,
                    };
                    let v = if item.by_ref {
                        let path = self.path(&item.value)?;
                        Value::Ref(self.make_ref(&path)?)
                    } else {
                        self.expr(&item.value)?
                    };
                    match key {
                        Some(k) => arr.insert(k, v),
                        None => {
                            arr.push(v).map_err(|_| next_element_occupied())?;
                        }
                    }
                }
                Value::Array(Rc::new(arr))
            }
            ExprKind::Index(_, None) => return Err(Fatal::error("Cannot use [] for reading")),
            ExprKind::Index(base, Some(index)) if self.is_globals(base) => {
                let name = self.expr(index)?;
                let name = String::from_utf8_lossy(&self.stringify(&name)?).into_owned();
                match self.frames[0].vars.get(&name) {
                    Some(v) => v.copy_out(),
                    None => {
                        diag::warning(format!("Undefined global variable ${name}"));
                        Value::Null
                    }
                }
            }
            ExprKind::Index(base, Some(index)) => {
                let b = self.expr(base)?;
                let i = self.expr(index)?;
                match &b {
                    Value::Object(o) => self.offset_get(o, Some(&i))?,
                    _ => read_dim(&b, &i)?,
                }
            }
            ExprKind::Assign(target, rhs) => match &target.kind {
                ExprKind::List(items) if has_ref(items) => {
                    if !rhs.is_lvalue() {
                        return Err(Fatal::compile("Cannot assign reference to non referenceable value"));
                    }
                    let path = self.path(rhs)?;
                    self.destructure_ref(items, &path)?;
                    self.expr(rhs)?
                }
                _ => {
                    let v = self.expr(rhs)?;
                    self.assign(target, v.clone())?;
                    v
                }
            },
            ExprKind::AssignRef(target, source) => {
                let path = self.path(source)?;
                let r = self.make_ref(&path)?;
                let v = value::with_deref(&r, Value::clone);
                self.bind_ref(target, r)?;
                v
            }
            ExprKind::CompoundAssign("??", target, rhs) => match self.quiet(target)? {
                Some(v) if !v.is_null() => v,
                _ => {
                    let v = self.expr(rhs)?;
                    self.assign(target, v.clone())?;
                    v
                }
            },
            ExprKind::CompoundAssign(op, target, rhs) => {
                let path = self.path(target)?;
                let r = self.expr(rhs)?;
                // A whole property is read and written back, so its type, readonly
                // state and __get/__set all apply
                if let Some((obj, name)) = path.whole_prop() {
                    let (obj, name) = (obj.clone(), name.to_string());
                    let cur = self.read_prop(&obj, &name)?;
                    let new = if *op == "." {
                        let mut s = self.stringify(&cur)?;
                        s.extend_from_slice(&self.stringify(&r)?);
                        Value::str(s)
                    } else {
                        binary(op, &cur, &r)?
                    };
                    self.write_prop(&obj, &name, new.clone())?;
                    return Ok(new);
                }
                // Likewise an ArrayAccess element is read with offsetGet and written with offsetSet
                if let Base::Offset(obj, dim) = &path.base {
                    let cur = self.offset_get(obj, dim.as_ref())?;
                    let new = if *op == "." {
                        let mut s = self.stringify(&cur)?;
                        s.extend_from_slice(&self.stringify(&r)?);
                        Value::str(s)
                    } else {
                        binary(op, &cur, &r)?
                    };
                    self.offset_set(obj, dim.as_ref(), new.clone())?;
                    return Ok(new);
                }
                // String conversion may run __toString, so it happens before the write
                let r_str = if *op == "." { Some(self.stringify(&r)?) } else { None };
                self.with_place(&path, true, true, |place| {
                    match (r_str, &mut *place) {
                        // Append in place so `$s .= ...` in a loop stays linear
                        (Some(s), Value::Str(cur)) => Rc::make_mut(cur).extend_from_slice(&s),
                        (Some(s), cur) => {
                            let mut joined = to_str(cur)?;
                            joined.extend_from_slice(&s);
                            *place = Value::str(joined);
                        }
                        (None, _) => *place = binary(op, place, &r)?,
                    }
                    Ok(place.clone())
                })?
            }
            ExprKind::IncDec { target, inc, prefix } => {
                let path = self.path(target)?;
                if let Some((obj, name)) = path.whole_prop() {
                    let (obj, name) = (obj.clone(), name.to_string());
                    let old = self.read_prop(&obj, &name)?;
                    let new = if *inc { increment(&old)? } else { decrement(&old)? };
                    self.write_prop(&obj, &name, new.clone())?;
                    return Ok(if *prefix { new } else { old });
                }
                self.with_place(&path, true, true, |place| {
                    let old = place.clone();
                    *place = if *inc { increment(&old)? } else { decrement(&old)? };
                    Ok(if *prefix { place.clone() } else { old })
                })?
            }
            ExprKind::Unary(op, operand) => {
                let v = self.expr(operand)?;
                match *op {
                    "!" => Value::Bool(!v.to_bool()),
                    "-" => value::neg(&v)?,
                    "+" => value::plus(&v)?,
                    "~" => match &v {
                        Value::Str(s) => Value::str(s.iter().map(|b| !b).collect::<Vec<u8>>()),
                        Value::Int(i) => Value::Int(!i),
                        Value::Float(f) => Value::Int(!float_to_int_implicit(*f, &v)),
                        _ => return Err(Fatal::type_error(format!("Cannot perform bitwise not on {}", v.type_name()))),
                    },
                    "(int)" => Value::Int(v.cast_int()),
                    "(float)" => Value::Float(v.to_float()),
                    "(string)" => Value::str(self.stringify(&v)?),
                    "(bool)" => Value::Bool(v.to_bool()),
                    "(array)" => cast_array(v),
                    "(object)" => self.cast_object(v),
                    _ => unreachable!("unknown unary operator {op}"),
                }
            }
            ExprKind::Binary("&&", l, r) => Value::Bool(self.expr(l)?.to_bool() && self.expr(r)?.to_bool()),
            ExprKind::Binary("||", l, r) => Value::Bool(self.expr(l)?.to_bool() || self.expr(r)?.to_bool()),
            ExprKind::Binary("??", l, r) => match self.quiet(l)? {
                Some(v) if !v.is_null() => v,
                _ => self.expr(r)?,
            },
            ExprKind::Binary(".", l, r) => {
                let lv = self.expr(l)?;
                let rv = self.expr(r)?;
                let mut s = self.stringify(&lv)?;
                s.extend_from_slice(&self.stringify(&rv)?);
                Value::str(s)
            }
            ExprKind::Binary(op, l, r) => {
                let lv = self.expr(l)?;
                let rv = self.expr(r)?;
                let result = binary(op, &lv, &rv)?;
                // Operands are released left to right like php-src (Rust would drop
                // them in reverse), which decides the order object handles are reused
                drop(lv);
                drop(rv);
                result
            }
            ExprKind::Ternary(cond, then, otherwise) => {
                let c = self.expr(cond)?;
                if c.to_bool() {
                    match then {
                        Some(t) => self.expr(t)?,
                        None => c,
                    }
                } else {
                    self.expr(otherwise)?
                }
            }
            ExprKind::Isset(targets) => {
                for t in targets {
                    if self.quiet_inner(t, false)?.is_none_or(|v| v.is_null()) {
                        return Ok(Value::Bool(false));
                    }
                }
                Value::Bool(true)
            }
            ExprKind::Empty(target) => Value::Bool(!self.quiet(target)?.is_some_and(|v| v.to_bool())),
            ExprKind::Call(name, args) => {
                let v = self.call(name.trim_start_matches('\\'), args, e.line, true)?;
                // Whatever the call did, diagnostics from here on belong to the call site
                diag::set_line(e.line);
                v
            }
            ExprKind::NsCall(name, global, args) => {
                let lglobal = global.to_ascii_lowercase();
                let use_global = !self.functions.contains_key(&name.to_ascii_lowercase())
                    && (self.functions.contains_key(&lglobal) || builtins::exists(&lglobal));
                // Resolved at runtime, so php-src does not compile it to a dedicated opcode
                let v = if use_global { self.call(global, args, e.line, false)? } else { self.call(name, args, e.line, false)? };
                // Whatever the call did, diagnostics from here on belong to the call site
                diag::set_line(e.line);
                v
            }
            ExprKind::CallValue(callee, args) => {
                let f = self.expr(callee)?;
                let v = self.call_value_expr(f, args, e.line)?;
                diag::set_line(e.line);
                v
            }
            ExprKind::Closure(f) => self.make_closure(f)?,
            ExprKind::CallableFrom(call) => self.callable_from(call)?,
            ExprKind::AnonymousClass(decl, args) => {
                let name = decl.name.to_ascii_lowercase();
                if !self.classes.contains_key(&name) { self.declare_class(decl)?; }
                let class = self.classes[&name].clone();
                let value = self.instantiate(&class, args, e.line)?;
                diag::set_line(e.line);
                value
            }
            ExprKind::New(class, args) => {
                let class = self.resolve_class(class)?;
                let v = self.instantiate(&class, args, e.line)?;
                diag::set_line(e.line);
                v
            }
            ExprKind::MethodCall { object, name, args, nullsafe } => {
                let o = self.expr(object)?;
                if *nullsafe && o.is_null() {
                    // `?->` short-circuits: the arguments are not evaluated
                    return Ok(Value::Null);
                }
                let name = self.member_name(name)?;
                let v = self.method_call(o, &name, args, e.line)?;
                diag::set_line(e.line);
                v
            }
            ExprKind::StaticCall { class, name, args } => {
                let v = self.static_call(class, name, args, e.line)?;
                diag::set_line(e.line);
                v
            }
            ExprKind::Prop { object, name, nullsafe } => {
                let o = self.expr(object)?;
                if *nullsafe && o.is_null() {
                    return Ok(Value::Null);
                }
                let name = self.member_name(name)?;
                match o {
                    Value::Object(obj) => self.read_prop(&obj, &name)?,
                    Value::Null if *nullsafe => Value::Null,
                    other => {
                        diag::warning(format!("Attempt to read property \"{name}\" on {}", other.type_name()));
                        Value::Null
                    }
                }
            }
            ExprKind::StaticProp { class, name } => {
                let (cell, _) = self.static_prop_cell(class, name)?;
                value::with_deref(&cell, Value::clone)
            }
            ExprKind::ClassConst { class, name } => self.class_const(class, name)?,
            ExprKind::InstanceOf(x, class) => {
                let v = self.expr(x)?;
                let Value::Object(o) = &v else { return Ok(Value::Bool(false)) };
                let name = match class {
                    crate::ast::ClassRef::Named(n) => n.clone(),
                    other => match self.resolve_class_quiet(other)? {
                        Some(c) => c.name.to_string(),
                        None => return Ok(Value::Bool(false)),
                    },
                };
                Value::Bool(self.is_instance_of(&o.class, &name))
            }
            ExprKind::Silence(x) => diag::silenced(|| self.expr(x))?,
            ExprKind::Include(kind, path) => {
                let path = self.expr(path)?;
                let path = String::from_utf8_lossy(&self.stringify(&path)?).into_owned();
                let v = self.include(*kind, &path, e.line)?;
                diag::set_line(e.line);
                v
            }
            ExprKind::Exit(arg) => {
                let status = match arg {
                    None => 0,
                    Some(a) => match self.expr(a)? {
                        // A message is printed, and the status is 0
                        Value::Str(s) => {
                            self.write(&s)?;
                            0
                        }
                        v => v.to_int() as u8,
                    },
                };
                return Err(Fatal { uncatchable: true, exit: Some(status), ..Fatal::new("", "") });
            }
            ExprKind::Clone(x) => {
                let v = self.expr(x)?;
                self.clone_value(v)?
            }
            ExprKind::Throw(x) => {
                return match self.expr(x)? {
                    Value::Object(o) if o.class.is_throwable => Err(Fatal::thrown(o)),
                    _ => Err(Fatal::error("Can only throw objects")),
                };
            }
        })
    }

    /// Evaluates without "undefined" warnings, as isset/empty/?? do.
    /// None means the variable or a key along the way does not exist
    fn quiet(&mut self, e: &Expr) -> EResult<Option<Value>> {
        self.quiet_inner(e, true)
    }

    /// `need_value` is false for the outermost operand of isset(), which only asks
    /// whether something exists: then a magic property is checked with __isset alone
    fn quiet_inner(&mut self, e: &Expr, need_value: bool) -> EResult<Option<Value>> {
        match &e.kind {
            ExprKind::Var(name) if name == "this" => Ok(self.ctx().this.clone().map(Value::Object)),
            ExprKind::Var(name) if name == "GLOBALS" && !self.frame().vars.contains_key(name) => Ok(Some(self.globals_array())),
            ExprKind::Var(name) if Self::is_superglobal(name) => {
                let name = self.superglobal(name);
                Ok(self.frames[0].vars.get(&name).map(Value::copy_out))
            }
            ExprKind::Var(name) => Ok(self.frame().vars.get(name).map(Value::copy_out)),
            ExprKind::Index(base, Some(index)) if self.is_globals(base) => {
                let name = self.expr(index)?;
                let name = String::from_utf8_lossy(&self.stringify(&name)?).into_owned();
                Ok(self.frames[0].vars.get(&name).map(Value::copy_out))
            }
            ExprKind::Index(base, Some(index)) => {
                let Some(b) = self.quiet(base)? else { return Ok(None) };
                let i = self.expr(index)?;
                Ok(match &b {
                    Value::Array(arr) => arr.get(&to_key(&i, "array")?).map(Value::copy_out),
                    // isset() itself only asks offsetExists; anything else reads the value too
                    Value::Object(o) => match self.offset_exists(o, &i)? {
                        false => None,
                        true if need_value => Some(self.offset_get(o, Some(&i))?),
                        true => Some(Value::Bool(true)),
                    },
                    Value::Str(s) => {
                        let off = match &i {
                            Value::Str(t) => match parse_numeric(t) {
                                (Num::Int(n), Numeric::Full) => Some(n),
                                _ => None,
                            },
                            Value::Float(f) => Some(float_to_int_implicit(*f, &i)),
                            Value::Array(_) | Value::Object(_) => None,
                            other => Some(other.to_int()),
                        };
                        off.and_then(|o| byte_at(s, o)).map(|b| Value::str(vec![b]))
                    }
                    _ => None,
                })
            }
            ExprKind::Prop { object, name, .. } => match self.quiet(object)? {
                Some(Value::Object(o)) => {
                    let name = self.member_name(name)?;
                    self.isset_prop(&o, &name, need_value)
                }
                _ => Ok(None),
            },
            ExprKind::StaticProp { class, name } => {
                let Some(c) = self.resolve_class_quiet(class)? else { return Ok(None) };
                Ok(c.find_static(name).map(|p| value::with_deref(&p.cell, Value::clone)))
            }
            _ => self.expr(e).map(Some),
        }
    }

    /// Resolves a write target. Everything before the last property or static
    /// property step is evaluated as a value: objects are handles, so writing
    /// through them needs no write access to the variable holding them
    fn path<'e>(&mut self, target: &'e Expr) -> EResult<Path<'e>> {
        let path = self.raw_path(target)?;
        self.resolve_offsets(path)
    }

    /// Rewrites a path whose offsets run into an object, so that they go through
    /// ArrayAccess. The object's offset becomes the last step. Offsets below it
    /// are reached with offsetGet: unless that gives an object, the write lands in
    /// a copy that is thrown away, as in PHP
    fn resolve_offsets<'e>(&mut self, mut path: Path<'e>) -> EResult<Path<'e>> {
        if !self.array_access_declared {
            return Ok(path);
        }
        while let Some((k, obj)) = self.find_object(&path) {
            let mut rest = path.dims.split_off(k);
            let dim = rest.remove(0);
            if rest.is_empty() {
                return Ok(Path { base: Base::Offset(obj, dim), dims: Vec::new() });
            }
            let v = self.offset_get_indirect(&obj, dim.as_ref())?;
            path = Path { base: Base::Cell(Rc::new(RefCell::new(v)), None), dims: rest };
        }
        Ok(path)
    }

    /// For a path used more than once: an ArrayAccess element is fetched once
    /// into a copy, which the uses then share
    fn fetch_offset<'e>(&mut self, path: Path<'e>) -> EResult<Path<'e>> {
        match &path.base {
            Base::Offset(obj, dim) => {
                let v = self.offset_get_indirect(obj, dim.as_ref())?;
                Ok(Path { base: Base::Cell(Rc::new(RefCell::new(v)), None), dims: path.dims })
            }
            _ => Ok(path),
        }
    }

    /// The first object that an offset of the path is applied to, with the
    /// position of that offset. Only looks, without creating anything
    fn find_object(&mut self, path: &Path<'_>) -> Option<(usize, Rc<Object>)> {
        if path.dims.is_empty() {
            return None;
        }
        let dims = &path.dims;
        match &path.base {
            Base::Var(name) => self.frames.last().unwrap().vars.get(name).and_then(|v| probe_object(v, dims, 0)),
            Base::Global(name) => self.frames[0].vars.get(name).and_then(|v| probe_object(v, dims, 0)),
            Base::Prop(obj, name) => {
                let key = match self.find_prop(&obj.class, name) {
                    oop::PropLookup::Declared(p) => p.key,
                    oop::PropLookup::Dynamic => Key::Str(name.as_bytes().into()),
                    oop::PropLookup::Inaccessible(_) => return None,
                };
                let props = obj.props.try_borrow().ok()?;
                probe_object(props.get(&key)?, dims, 0)
            }
            Base::Cell(cell, _) => probe_object(&*cell.try_borrow().ok()?, dims, 0),
            Base::Offset(..) => None,
        }
    }

    fn raw_path<'e>(&mut self, target: &'e Expr) -> EResult<Path<'e>> {
        diag::set_line(target.line);
        match &target.kind {
            ExprKind::Var(name) if Self::is_superglobal(name) => {
                Ok(Path { base: Base::Global(self.superglobal(name)), dims: Vec::new() })
            }
            ExprKind::Var(name) => Ok(Path::var(name)),
            ExprKind::Index(base, Some(index)) if self.is_globals(base) => {
                let name = self.expr(index)?;
                let name = String::from_utf8_lossy(&self.stringify(&name)?).into_owned();
                Ok(Path { base: Base::Global(name), dims: Vec::new() })
            }
            ExprKind::Index(base, index) => {
                let mut path = self.raw_path(base)?;
                path.dims.push(match index {
                    Some(i) => Some(self.expr(i)?),
                    None => None,
                });
                Ok(path)
            }
            ExprKind::Prop { object, name, .. } => {
                let o = self.expr(object)?;
                let name = self.member_name(name)?;
                match o {
                    Value::Object(o) => Ok(Path { base: Base::Prop(o, name), dims: Vec::new() }),
                    other => Err(Fatal::error(format!("Attempt to assign property \"{name}\" on {}", other.type_name()))),
                }
            }
            ExprKind::StaticProp { class, name } => {
                let (cell, ty) = self.static_prop_cell(class, name)?;
                Ok(Path { base: Base::Cell(cell, ty), dims: Vec::new() })
            }
            _ => Err(Fatal::error("Cannot assign to this expression")),
        }
    }

    /// Runs `f` on the storage for a path, creating arrays and elements along the
    /// way. With `read`, the old value is about to be used (`.=`, `++`), so a
    /// missing variable or final key is reported like a read. With `deref_last`,
    /// `f` sees the value behind a final reference rather than the reference slot
    fn with_place<T>(
        &mut self,
        path: &Path<'_>,
        read: bool,
        deref_last: bool,
        f: impl FnOnce(&mut Value) -> EResult<T>,
    ) -> EResult<T> {
        match &path.base {
            Base::Var(name) => {
                if *name == "this" {
                    return Err(Fatal::error("Cannot re-assign $this"));
                }
                let vars = &mut self.frames.last_mut().unwrap().vars;
                if read && path.dims.is_empty() && !vars.contains_key(name) {
                    diag::warning(format!("Undefined variable ${name}"));
                }
                let slot = vars.entry(name);
                descend(slot, &path.dims, read, deref_last, f)
            }
            Base::Global(name) => {
                let vars = &mut self.frames[0].vars;
                if read && path.dims.is_empty() && !vars.contains_key(name) {
                    diag::warning(format!("Undefined global variable ${name}"));
                }
                descend(vars.entry(name), &path.dims, read, deref_last, f)
            }
            Base::Prop(obj, name) => {
                let key = self.prop_key_for_update(obj, name, read && path.dims.is_empty())?;
                let mut props = obj.props.try_borrow_mut().map_err(|_| recursive_reference())?;
                let slot = props.entry(key);
                descend(slot, &path.dims, read, deref_last, f)
            }
            Base::Cell(cell, _) => {
                let mut slot = cell.try_borrow_mut().map_err(|_| recursive_reference())?;
                descend(&mut slot, &path.dims, read, deref_last, f)
            }
            Base::Offset(obj, dim) => {
                let mut copy = self.offset_get_indirect(obj, dim.as_ref())?;
                descend(&mut copy, &path.dims, read, deref_last, f)
            }
        }
    }

    /// Turns the slot at `path` into a reference (if it is not one yet) and returns it
    fn make_ref(&mut self, path: &Path<'_>) -> EResult<RefCellValue> {
        if let (Base::Cell(cell, _), true) = (&path.base, path.dims.is_empty()) {
            return Ok(cell.clone());
        }
        self.with_place(path, false, false, |slot| Ok(make_ref_slot(slot)))
    }

    /// Makes `target` an alias of `r`, as `$target = &...` does
    fn bind_ref(&mut self, target: &Expr, r: RefCellValue) -> EResult<()> {
        let path = self.path(target)?;
        if let (Base::Cell(..), true) = (&path.base, path.dims.is_empty()) {
            return Err(Fatal::error("Binding static properties by reference is not supported yet"));
        }
        if let Base::Offset(obj, dim) = &path.base {
            self.offset_get_indirect(obj, dim.as_ref())?;
            return Err(Fatal::error("Cannot assign by reference to an array dimension of an object"));
        }
        self.with_place(&path, false, false, |slot| {
            *slot = Value::Ref(r);
            Ok(())
        })
    }

    fn assign(&mut self, target: &Expr, v: Value) -> EResult<()> {
        if let ExprKind::List(items) = &target.kind {
            return self.destructure(items, v);
        }
        let mut path = self.path(target)?;
        if let (Base::Prop(obj, name), true) = (&path.base, path.dims.is_empty()) {
            return self.write_prop(obj, name, v);
        }
        if let Base::Offset(obj, dim) = &path.base {
            return self.offset_set(obj, dim.as_ref(), v);
        }
        if let (Base::Cell(cell, ty), true) = (&path.base, path.dims.is_empty()) {
            let v = match ty {
                Some((ty, label)) => {
                    let err = |v: &Value, _: &str| {
                        Fatal::type_error(format!("Cannot assign {} to property {label} of type {}", v.value_name(), ty.display))
                    };
                    self.check_param(v, Some(ty), &err)?
                }
                None => v,
            };
            let mut slot = cell.try_borrow_mut().map_err(|_| recursive_reference())?;
            return set_slot(&mut slot, v);
        }
        // `$s[i] = "x"` on a string replaces a byte rather than creating an element
        if let Some(Some(index)) = path.dims.last() {
            let index = index.clone();
            path.dims.pop();
            // An object written to a string offset is converted with __toString,
            // which cannot run in the borrowed write path: the write is retried
            // with the converted value
            let obj = matches!(v, Value::Object(_)).then(|| v.clone());
            let needs_str = self.with_place(&path, false, true, |parent| {
                if let Value::Str(s) = parent {
                    if obj.is_some() {
                        return Ok(true);
                    }
                    assign_string_offset(Rc::make_mut(s), &index, &v)?;
                    return Ok(false);
                }
                set_slot(write_dim(parent, Some(&index), false)?, v)?;
                Ok(false)
            })?;
            if let (true, Some(obj)) = (needs_str, obj) {
                let v = Value::str(self.stringify(&obj)?);
                return self.with_place(&path, false, true, |parent| match parent {
                    Value::Str(s) => assign_string_offset(Rc::make_mut(s), &index, &v),
                    _ => set_slot(write_dim(parent, Some(&index), false)?, v),
                });
            }
            return Ok(());
        }
        self.with_place(&path, false, true, |slot| {
            *slot = v;
            Ok(())
        })
    }

    /// `[$a, 'k' => $b] = $v`. Elements missing from an array warn like reads;
    /// a non-array assigns null to every target
    fn destructure(&mut self, items: &[Option<ListItem>], v: Value) -> EResult<()> {
        let mut next = 0;
        for item in items {
            let Some(item) = item else {
                next += 1;
                continue;
            };
            let key = self.list_key(item, &mut next)?;
            let elem = match &v {
                Value::Array(_) => read_dim(&v, &key)?,
                Value::Object(o) => self.offset_get(o, Some(&key))?,
                _ => Value::Null,
            };
            self.assign(&item.target, elem)?;
        }
        Ok(())
    }

    /// Destructuring with `&` targets, which bind to the elements of the source variable
    fn destructure_ref(&mut self, items: &[Option<ListItem>], source: &Path<'_>) -> EResult<()> {
        let mut next = 0;
        for item in items {
            let Some(item) = item else {
                next += 1;
                continue;
            };
            let key = self.list_key(item, &mut next)?;
            let mut elem = source.clone();
            elem.dims.push(Some(key.clone()));
            match &item.target.kind {
                _ if item.by_ref => {
                    let r = self.make_ref(&elem)?;
                    self.bind_ref(&item.target, r)?;
                }
                ExprKind::List(inner) if has_ref(inner) => self.destructure_ref(inner, &elem)?,
                _ => {
                    let container = self.with_place(source, false, true, |slot| Ok(slot.clone()))?;
                    let v = match &container {
                        Value::Array(_) => read_dim(&container, &key)?,
                        Value::Object(o) => self.offset_get(o, Some(&key))?,
                        _ => Value::Null,
                    };
                    self.assign(&item.target, v)?;
                }
            }
        }
        Ok(())
    }

    /// The key a list item reads: its own, or the next position
    fn list_key(&mut self, item: &ListItem, next: &mut i64) -> EResult<Value> {
        Ok(match &item.key {
            Some(k) => self.expr(k)?,
            None => {
                *next += 1;
                Value::Int(*next - 1)
            }
        })
    }

    fn constant(&mut self, name: &str) -> EResult<Value> {
        self.lookup_const(name).ok_or_else(|| undefined_constant(name))
    }

    fn lookup_const(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.constants.get(&*const_key(name)) {
            return Some(v.clone());
        }
        constant(name)
    }

    /// Defines a global constant; false (with a warning) if it already exists
    fn define_const(&mut self, name: &str, v: Value) -> bool {
        let key = const_key(name);
        if self.constants.contains_key(&*key) || constant(name).is_some() {
            diag::warning(format!("Constant {name} already defined"));
            return false;
        }
        self.constants.insert(key.into_owned(), v);
        true
    }

    fn unset(&mut self, target: &Expr) -> EResult<()> {
        let mut path = self.path(target)?;
        let Some(last) = path.dims.pop() else {
            return match &path.base {
                // Unsetting a variable only removes this name; a reference survives elsewhere
                Base::Var(name) => {
                    self.frame().vars.remove(name);
                    Ok(())
                }
                Base::Global(name) => {
                    self.frames[0].vars.remove(name);
                    Ok(())
                }
                Base::Prop(obj, name) => self.unset_prop(obj, name),
                Base::Cell(..) => Err(Fatal::error("Attempt to unset static property")),
                Base::Offset(obj, dim) => self.offset_unset(obj, dim.as_ref()),
            };
        };
        match &path.base {
            Base::Var(name) => match self.frame().vars.get_mut(name) {
                Some(slot) => unset_in(slot, &path.dims, last),
                None => {
                    diag::warning(format!("Undefined variable ${name}"));
                    Ok(())
                }
            },
            Base::Global(name) => match self.frames[0].vars.get_mut(name) {
                Some(slot) => unset_in(slot, &path.dims, last),
                None => Ok(()),
            },
            Base::Prop(obj, name) => {
                let key = self.prop_key_for_update(obj, name, false)?;
                let mut props = obj.props.try_borrow_mut().map_err(|_| recursive_reference())?;
                match props.get_mut(&key) {
                    Some(slot) => unset_in(slot, &path.dims, last),
                    None => Ok(()),
                }
            }
            Base::Cell(cell, _) => {
                let mut slot = cell.try_borrow_mut().map_err(|_| recursive_reference())?;
                unset_in(&mut slot, &path.dims, last)
            }
            Base::Offset(obj, dim) => {
                let mut copy = self.offset_get_indirect(obj, dim.as_ref())?;
                unset_in(&mut copy, &path.dims, last)
            }
        }
    }

    fn member_name(&mut self, name: &Name) -> EResult<String> {
        match name {
            Name::Lit(n) => Ok(n.clone()),
            Name::Dyn(e) => {
                let v = self.expr(e)?;
                Ok(String::from_utf8_lossy(&self.stringify(&v)?).into_owned())
            }
        }
    }

    /// String conversion as done by echo, `.` and interpolation: arrays warn,
    /// objects use __toString
    pub fn stringify(&mut self, v: &Value) -> EResult<Vec<u8>> {
        let Value::Object(o) = v else { return to_str(v) };
        let Some(m) = o.class.find_method("__tostring").cloned() else {
            return Err(Fatal::error(format!("Object of class {} could not be converted to string", o.class.name)));
        };
        let line = diag::line();
        match self.call_method(Some(o.clone()), &m, Vec::new(), line, o.class.clone())? {
            Value::Str(s) => Ok(s.to_vec()),
            other => Err(Fatal::type_error(format!(
                "{}::__toString(): Return value must be of type string, {} returned",
                m.class,
                other.value_name()
            ))),
        }
    }

    /// Calls a function by name. `direct` is false for calls resolved at runtime,
    /// which php-src never compiles to dedicated opcodes
    fn call(&mut self, name: &str, args: &[Arg], line: u32, direct: bool) -> EResult<Value> {
        let lname = name.to_ascii_lowercase();
        if let Some(f) = self.functions.get(&lname).cloned() {
            let ctx = CallCtx { name: f.name.clone(), trace_fn: f.name.clone(), ..CallCtx::default() };
            let bound = self.bind_args(&f, &ctx.name, args)?;
            return self.invoke_with(&f, ctx, Vec::new(), bound, line);
        }
        // call_user_func() hands its named arguments on to the callback
        if lname == "call_user_func" && args.iter().any(|a| a.name.is_some()) && !args.iter().any(|a| a.spread) {
            let mut positional = Vec::new();
            let mut named = Vec::new();
            for a in args {
                let v = self.expr(&a.value)?;
                match &a.name {
                    Some(n) => named.push((n.clone(), v)),
                    None => positional.push(v),
                }
            }
            if positional.is_empty() {
                return Err(Fatal::new("ArgumentCountError", "call_user_func(): Argument #1 ($callback) not passed"));
            }
            let cb = positional.remove(0);
            let f = self.callback(&lname, 1, &cb)?;
            let shown = std::iter::once(cb).chain(positional.iter().cloned()).collect();
            return self.internal_call(lname.clone(), None, shown, line, |this| {
                this.call_value_named(&f, positional, named, line)
            });
        }
        let reordered = if builtins::exists(&lname) { self.order_builtin_args(&lname, args)? } else { None };
        let args = reordered.as_deref().unwrap_or(args);

        // Built-ins that take their first argument by reference
        if builtins::is_by_ref(&lname) || matches!(lname.as_str(), "usort" | "uasort" | "uksort" | "array_walk") {
            let Some((first, rest)) = args.split_first() else {
                return Err(builtins::arg_count(name, 1, 0));
            };
            if !first.value.is_lvalue() {
                return Err(Fatal::error(format!("{lname}(): Argument #1 ($array) could not be passed by reference")));
            }
            let path = self.path(&first.value)?;
            let path = self.fetch_offset(path)?;
            let vals = self.eval_args(rest)?;
            let mut shown = vec![self.with_place(&path, false, true, |slot| Ok(slot.clone()))?];
            shown.extend(vals.iter().cloned());
            return self.internal_call(lname.clone(), None, shown, line, |this| {
                if lname.starts_with('u') {
                    return this.user_sort(&lname, &path, vals, line);
                }
                if lname == "array_walk" {
                    return this.array_walk(&path, vals, line);
                }
                this.with_place(&path, false, true, |place| builtins::call_by_ref(&lname, place, vals))
            });
        }

        // Built-ins with an output parameter after the first
        if let Some((pos, param)) = builtins::out_param(&lname)
            && args.len() > pos
            && !args.iter().any(|a| a.spread)
        {
            if !args[pos].value.is_lvalue() {
                return Err(Fatal::error(format!(
                    "{lname}(): Argument #{} (${param}) could not be passed by reference",
                    pos + 1
                )));
            }
            let mut vals = Vec::with_capacity(args.len());
            for (i, a) in args.iter().enumerate() {
                vals.push(if i == pos { Value::Null } else { self.expr(&a.value)? });
            }
            let path = self.path(&args[pos].value)?;
            let shown = vals.clone();
            return self.internal_call(lname.clone(), None, shown, line, |this| {
                let (v, out) = this.call_with_out(&lname, vals)?;
                this.with_place(&path, false, true, |slot| {
                    match out {
                        Some(out) => set_slot(slot, out),
                        None => Ok(()),
                    }
                })?;
                Ok(v)
            });
        }

        let vals = self.eval_args(args)?;
        if (direct && compiled_inline(&lname, args)) || lname == "print" || !builtins::exists(&lname) {
            return self.call_builtin(name, &lname, vals, line);
        }
        self.internal_call(lname.clone(), None, vals.clone(), line, |this| this.call_builtin(name, &lname, vals, line))
    }

    /// A built-in with an output parameter: its result and the value for the parameter
    fn call_with_out(&mut self, lname: &str, vals: Vec<Value>) -> EResult<(Value, Option<Value>)> {
        if lname.starts_with("preg_") {
            return builtins::call_out(self, lname, vals);
        }
        let mut vals = vals.into_iter();
        let mut next = || vals.next().unwrap_or(Value::Null);
        match lname {
            "str_replace" | "str_ireplace" => {
                let (search, replace, subject) = (next(), next(), next());
                let (v, count) = builtins::str_replace(self, lname, search, replace, subject)?;
                Ok((v, Some(Value::Int(count))))
            }
            _ => {
                let (a, b) = (next(), next());
                let a = builtins::arg_str(self, lname, 1, "string1", &a)?;
                let b = builtins::arg_str(self, lname, 2, "string2", &b)?;
                let sim = builtins::similar_text(&a, &b);
                let total = a.len() + b.len();
                let percent = if total == 0 { 0.0 } else { sim as f64 * 2.0 * 100.0 / total as f64 };
                Ok((Value::Int(sim as i64), Some(Value::Float(percent))))
            }
        }
    }

    /// Runs a built-in as php-src's internal call frame: an error the built-in
    /// raises itself gets a stack trace starting with that frame
    fn internal_call<T>(
        &mut self,
        function: String,
        class: Option<(String, &'static str)>,
        args: Vec<Value>,
        line: u32,
        f: impl FnOnce(&mut Self) -> EResult<T>,
    ) -> EResult<T> {
        let frame = TraceFrame { file: self.file.path.to_string(), line, function, class, args };
        self.internal_calls.push((self.frames.len(), frame, false));
        let result = f(self).map_err(|mut e| {
            if e.object.is_none() && e.trace.is_none() && !e.uncatchable {
                // The error belongs to the line of the call
                e.line = Some(e.line.unwrap_or(line));
                e.trace = Some(self.trace());
            }
            e
        });
        self.internal_calls.pop();
        result
    }

    /// Built-ins called with evaluated arguments
    fn call_builtin(&mut self, name: &str, lname: &str, vals: Vec<Value>, line: u32) -> EResult<Value> {
        match lname {
            "print" => {
                let s = self.stringify(&vals[0])?;
                self.write(&s)?;
                Ok(Value::Int(1))
            }
            "var_dump" => {
                for v in &vals {
                    let mut s = Vec::new();
                    builtins::var_dump(&mut s, v, 1);
                    self.write(&s)?;
                }
                Ok(Value::Null)
            }
            "printf" | "vprintf" => {
                let s = if lname == "printf" {
                    builtins::sprintf(self, lname, vals)?
                } else {
                    builtins::vsprintf(self, name, lname, vals)?
                };
                self.write(&s)?;
                Ok(Value::Int(s.len() as i64))
            }
            "var_export" => {
                let [v, ret] = <[Value; 2]>::try_from({
                    let mut v = vals;
                    if v.is_empty() || v.len() > 2 {
                        return Err(builtins::arg_count(lname, 1, v.len()));
                    }
                    v.resize(2, Value::Bool(false));
                    v
                })
                .unwrap();
                let mut s = Vec::new();
                builtins::var_export(&mut s, &v, 1);
                if ret.to_bool() {
                    return Ok(Value::str(s));
                }
                self.write(&s)?;
                Ok(Value::Null)
            }
            "print_r" => {
                let mut s = Vec::new();
                builtins::print_r(&mut s, vals.first().unwrap_or(&Value::Null), 0);
                if vals.get(1).is_some_and(Value::to_bool) {
                    return Ok(Value::str(s));
                }
                self.write(&s)?;
                Ok(Value::Bool(true))
            }
            "func_get_args" => Ok(list(self.frame().args.clone())),
            "func_num_args" => Ok(Value::Int(self.frame().args.len() as i64)),
            "function_exists" => {
                let n = vals.first().map(|v| v.to_bytes()).unwrap_or_default();
                let n = String::from_utf8_lossy(&n).trim_start_matches('\\').to_ascii_lowercase();
                Ok(Value::Bool(self.functions.contains_key(&n) || builtins::exists(&n)))
            }
            "is_callable" => {
                let callable = match vals.first() {
                    Some(v) => self.callable(v)?.is_some(),
                    None => false,
                };
                Ok(Value::Bool(callable))
            }
            "call_user_func" => {
                let Some((cb, rest)) = vals.split_first() else {
                    return Err(builtins::arg_count(lname, 1, 0));
                };
                let f = self.callback(lname, 1, cb)?;
                self.call_value(&f, rest.to_vec(), line)
            }
            "call_user_func_array" => {
                let [cb, args] = <[Value; 2]>::try_from(vals).map_err(|v| builtins::arg_count(lname, 2, v.len()))?;
                let f = self.callback(lname, 1, &cb)?;
                let Value::Array(arr) = args else {
                    return Err(Fatal::type_error(format!(
                        "call_user_func_array(): Argument #2 ($args) must be of type array, {} given",
                        args.value_name()
                    )));
                };
                let mut positional = Vec::new();
                let mut named = Vec::new();
                for (k, v) in arr.iter() {
                    match k {
                        Key::Int(_) if !named.is_empty() => {
                            return Err(Fatal::error("Cannot use positional argument after named argument during unpacking"));
                        }
                        Key::Int(_) => positional.push(v.copy_out()),
                        Key::Str(s) => named.push((String::from_utf8_lossy(s).into_owned(), v.copy_out())),
                    }
                }
                self.call_value_named(&f, positional, named, line)
            }
            "define" => {
                let [name, value] = <[Value; 2]>::try_from(vals).map_err(|v| builtins::arg_count(lname, 2, v.len()))?;
                let name = String::from_utf8_lossy(&self.stringify(&name)?).into_owned();
                if name.contains("::") {
                    return Err(Fatal::value_error("define(): Argument #1 ($constant_name) cannot be a class constant"));
                }
                Ok(Value::Bool(self.define_const(&name, value)))
            }
            "defined" | "constant" => {
                let [name] = <[Value; 1]>::try_from(vals).map_err(|v| builtins::arg_count(lname, 1, v.len()))?;
                let name = String::from_utf8_lossy(&self.stringify(&name)?).trim_start_matches('\\').to_string();
                let found = match name.split_once("::") {
                    Some((class, cname)) => {
                        let class = crate::ast::ClassRef::from_name(class);
                        if lname == "defined" {
                            let c = self.resolve_class_quiet(&class)?;
                            return Ok(Value::Bool(c.is_some_and(|c| c.consts.contains_key(cname))));
                        }
                        self.class_const(&class, cname)
                    }
                    None => self.constant(&name),
                };
                match (lname, found) {
                    ("defined", r) => Ok(Value::Bool(r.is_ok())),
                    (_, r) => r,
                }
            }
            "array_map" => self.array_map(vals, line),
            "iterator_to_array" | "iterator_count" => self.iterator_builtin(lname, vals),
            "array_reduce" => {
                if vals.len() < 2 || vals.len() > 3 {
                    return Err(builtins::arg_count(lname, 2, vals.len()));
                }
                let mut vals = vals.into_iter();
                let arr = vals.next().unwrap();
                let Value::Array(arr) = arr else {
                    return Err(Fatal::type_error(format!(
                        "array_reduce(): Argument #1 ($array) must be of type array, {} given",
                        arr.value_name()
                    )));
                };
                let cb = self.callback(lname, 2, &vals.next().unwrap())?;
                let mut carry = vals.next().unwrap_or(Value::Null);
                for v in arr.values() {
                    carry = self.call_value(&cb, vec![carry, v.copy_out()], line)?;
                }
                Ok(carry)
            }
            "compact" => {
                let mut out = PhpArray::new();
                let mut names = vals;
                let mut depth = 0;
                while !names.is_empty() && depth < 64 {
                    let mut nested = Vec::new();
                    for n in names {
                        match n {
                            Value::Array(a) => nested.extend(a.values().map(Value::copy_out)),
                            Value::Str(s) => {
                                let name = String::from_utf8_lossy(&s).into_owned();
                                let found = self.frame().vars.get(&name).map(Value::copy_out);
                                let this = self.ctx().this.clone().filter(|_| name == "this");
                                match found.or(this.map(Value::Object)) {
                                    Some(v) => out.insert(Key::Str(s.to_vec().into()), v),
                                    None => diag::warning(format!("compact(): Undefined variable ${name}")),
                                }
                            }
                            other => diag::warning(format!(
                                "compact(): Argument #{} must be string or array of strings, {} given",
                                1,
                                other.type_name()
                            )),
                        }
                    }
                    names = nested;
                    depth += 1;
                }
                Ok(Value::Array(Rc::new(out)))
            }
            "extract" => {
                if vals.is_empty() || vals.len() > 3 {
                    return Err(builtins::arg_count(lname, 1, vals.len()));
                }
                let mut vals = vals.into_iter();
                let arr = vals.next().unwrap();
                let Value::Array(arr) = arr else {
                    return Err(Fatal::type_error(format!(
                        "extract(): Argument #1 ($array) must be of type array, {} given",
                        arr.value_name()
                    )));
                };
                let flags = vals.next().map_or(0, |f| f.to_int());
                let prefix = vals.next().map(|p| String::from_utf8_lossy(&p.to_bytes()).into_owned()).unwrap_or_default();
                let valid = |n: &str| {
                    let mut chars = n.chars();
                    chars.next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic() || !c.is_ascii())
                        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric() || !c.is_ascii())
                };
                let mut count = 0;
                for (k, v) in arr.iter() {
                    let Key::Str(s) = k else { continue };
                    let mut name = String::from_utf8_lossy(s).into_owned();
                    let exists = self.frame().vars.contains_key(&name);
                    match flags & 0xff {
                        1 if exists => continue,
                        2 if exists => name = format!("{prefix}_{name}"),
                        3 => name = format!("{prefix}_{name}"),
                        _ => {}
                    }
                    if !valid(&name) || name == "this" {
                        continue;
                    }
                    self.frame().vars.insert(name, v.copy_out());
                    count += 1;
                }
                Ok(Value::Int(count))
            }
            "is_iterable" => {
                let [v] = <[Value; 1]>::try_from(vals).map_err(|v| builtins::arg_count(lname, 1, v.len()))?;
                Ok(Value::Bool(self.is_traversable(&v)))
            }
            "array_filter" => self.array_filter(vals, line),
            _ if lname.starts_with("ob_") || lname == "flush" => self.output_builtin(lname, vals),
            _ => {
                if let Some(v) = self.object_builtin(lname, &vals)? {
                    return Ok(v);
                }
                builtins::call(self, name, lname, vals)
            }
        }
    }

    fn eval_args(&mut self, args: &[Arg]) -> EResult<Vec<Value>> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            let v = self.expr(&a.value)?;
            if a.spread {
                vals.extend(spread(v)?);
            } else {
                vals.push(v);
            }
        }
        Ok(vals)
    }

    /// Evaluates call arguments for a user function, taking references for by-ref parameters
    fn bind_args(&mut self, f: &FuncDecl, fname: &str, args: &[Arg]) -> EResult<Vec<ArgVal>> {
        let mut bound = Vec::with_capacity(args.len());
        let mut named = Vec::new();
        for a in args {
            if a.spread {
                let v = self.expr(&a.value)?;
                let mut named_here = false;
                for (k, v) in spread_entries(v)? {
                    match k {
                        // Only within one unpacked array do string keys rule out int keys
                        Key::Int(_) if named_here => {
                            return Err(Fatal::error("Cannot use positional argument after named argument during unpacking"));
                        }
                        Key::Int(_) => bound.push(ArgVal::Val(v)),
                        Key::Str(s) => {
                            named_here = true;
                            let name = String::from_utf8_lossy(&s).into_owned();
                            place_named(f, &mut bound, &mut named, name, ArgVal::Val(v))?;
                        }
                    }
                }
                continue;
            }
            if let Some(name) = &a.name {
                let param = f.params.iter().position(|p| &p.name == name && !p.variadic);
                let by_ref = match param {
                    Some(i) => f.params[i].by_ref,
                    None => f.params.last().is_some_and(|p| p.variadic && p.by_ref),
                };
                let val = if by_ref {
                    if !a.value.is_lvalue() {
                        let n = param.map_or(f.params.len(), |i| i + 1);
                        return Err(Fatal::error(format!(
                            "{fname}(): Argument #{n} (${name}) could not be passed by reference"
                        )));
                    }
                    let path = self.path(&a.value)?;
                    ArgVal::Ref(self.make_ref(&path)?)
                } else {
                    ArgVal::Val(self.expr(&a.value)?)
                };
                place_named(f, &mut bound, &mut named, name.clone(), val)?;
                continue;
            }
            let i = bound.len();
            let by_ref = match f.params.get(i) {
                Some(p) => p.by_ref,
                None => f.params.last().is_some_and(|p| p.variadic && p.by_ref),
            };
            if by_ref {
                if !a.value.is_lvalue() {
                    let pname = &f.params[i.min(f.params.len() - 1)].name;
                    return Err(Fatal::error(format!(
                        "{fname}(): Argument #{} (${pname}) could not be passed by reference",
                        i + 1
                    )));
                }
                let path = self.path(&a.value)?;
                bound.push(ArgVal::Ref(self.make_ref(&path)?));
            } else {
                bound.push(ArgVal::Val(self.expr(&a.value)?));
            }
        }
        bound.extend(named.into_iter().map(|(n, v)| ArgVal::Named(n, Box::new(v))));
        Ok(bound)
    }

    /// Arguments for a user function from evaluated values, some of them named
    fn bind_values(f: &FuncDecl, positional: Vec<Value>, named: Vec<(String, Value)>) -> EResult<Vec<ArgVal>> {
        let mut bound: Vec<ArgVal> = positional.into_iter().map(ArgVal::Val).collect();
        let mut extra = Vec::new();
        for (name, v) in named {
            place_named(f, &mut bound, &mut extra, name, ArgVal::Val(v))?;
        }
        bound.extend(extra.into_iter().map(|(n, v)| ArgVal::Named(n, Box::new(v))));
        Ok(bound)
    }

    /// Calls a callable with named arguments as well (call_user_func_array with string keys)
    fn call_value_named(&mut self, f: &Callable, positional: Vec<Value>, named: Vec<(String, Value)>, line: u32) -> EResult<Value> {
        if named.is_empty() {
            return self.call_value(f, positional, line);
        }
        let decl = match f {
            Callable::User(d) => Some(d.clone()),
            Callable::Closure(o) => o.closure().map(|c| c.func.clone()),
            Callable::Method { method, .. } => match &method.body {
                object::MethodBody::User(d) => Some(d.clone()),
                object::MethodBody::Native(_) => None,
            },
            Callable::Builtin(_) => None,
        };
        if let Some(decl) = decl {
            let args = Self::bind_values(&decl, positional, named)?;
            return self.call_value_args(f, args, line);
        }
        let display = match f {
            Callable::Builtin(name) => name.clone(),
            Callable::Method { method, .. } => format!("{}::{}", method.class, method.name),
            _ => unreachable!("user callables handled above"),
        };
        let vals = self.order_builtin_values(&display, positional, named)?;
        self.call_value(f, vals, line)
    }

    /// Puts named arguments of a built-in into their positions, with the
    /// defaults of skipped parameters from its signature
    pub(super) fn order_builtin_values(
        &mut self,
        fname: &str,
        positional: Vec<Value>,
        named: Vec<(String, Value)>,
    ) -> EResult<Vec<Value>> {
        let Some(sig) = builtins::signature(&fname.to_ascii_lowercase()) else {
            let (name, _) = &named[0];
            return Err(Fatal::error(format!("Unknown named parameter ${name}")));
        };
        let mut slots: Vec<Option<Value>> = positional.into_iter().map(Some).collect();
        for (name, v) in named {
            match sig.iter().position(|p| p.name == name && !p.variadic) {
                Some(i) => {
                    if slots.len() <= i {
                        slots.resize(i + 1, None);
                    }
                    if slots[i].is_some() {
                        return Err(Fatal::error(format!("Named parameter ${name} overwrites previous argument")));
                    }
                    slots[i] = Some(v);
                }
                None if sig.last().is_some_and(|p| p.variadic) => {
                    return Err(Fatal::new(
                        "ArgumentCountError",
                        format!("{fname}() does not accept unknown named parameters"),
                    ));
                }
                None => return Err(Fatal::error(format!("Unknown named parameter ${name}"))),
            }
        }
        let mut out = Vec::with_capacity(slots.len());
        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(v) => out.push(v),
                None => {
                    let d = self.builtin_default(fname, i, &sig[i])?;
                    out.push(self.expr(&d)?);
                }
            }
        }
        Ok(out)
    }

    /// The default of a skipped built-in parameter, parsed from its signature
    fn builtin_default(&mut self, fname: &str, i: usize, p: &builtins::Param) -> EResult<Rc<Expr>> {
        let not_passed = |what: &str| {
            Fatal::new("ArgumentCountError", format!("{fname}(): Argument #{} (${}) {what}", i + 1, p.name))
        };
        let Some(src) = p.default else { return Err(not_passed("not passed")) };
        if src == "UNKNOWN" {
            return Err(not_passed("must be passed explicitly, because the default value is not known"));
        }
        if let Some(e) = self.builtin_defaults.get(src) {
            return Ok(e.clone());
        }
        let e = crate::parser::parse_expr(src.as_bytes())
            .map_err(|_| not_passed("must be passed explicitly, because the default value is not known"))?;
        let e = Rc::new(e);
        self.builtin_defaults.insert(src, e.clone());
        Ok(e)
    }

    /// Named arguments to a built-in called directly: the arguments in parameter
    /// order, with defaults for those skipped. None when no argument is named
    fn order_builtin_args(&mut self, lname: &str, args: &[Arg]) -> EResult<Option<Vec<Arg>>> {
        if !args.iter().any(|a| a.name.is_some()) {
            return Ok(None);
        }
        if args.iter().any(|a| a.spread) {
            return Err(Fatal::error("Named arguments after argument unpacking are not supported for built-in functions yet"));
        }
        let Some(sig) = builtins::signature(lname) else {
            let name = args.iter().find_map(|a| a.name.clone()).unwrap_or_default();
            return Err(Fatal::error(format!("Unknown named parameter ${name}")));
        };
        let mut slots: Vec<Option<Arg>> = Vec::new();
        for a in args {
            let Some(name) = &a.name else {
                slots.push(Some(a.clone()));
                continue;
            };
            match sig.iter().position(|p| p.name == name && !p.variadic) {
                Some(i) => {
                    if slots.len() <= i {
                        slots.resize(i + 1, None);
                    }
                    if slots[i].is_some() {
                        return Err(Fatal::error(format!("Named parameter ${name} overwrites previous argument")));
                    }
                    slots[i] = Some(Arg { name: None, ..a.clone() });
                }
                None if sig.last().is_some_and(|p| p.variadic) => {
                    return Err(Fatal::new(
                        "ArgumentCountError",
                        format!("{lname}() does not accept unknown named parameters"),
                    ));
                }
                None => return Err(Fatal::error(format!("Unknown named parameter ${name}"))),
            }
        }
        let mut out = Vec::with_capacity(slots.len());
        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(a) => out.push(a),
                None => {
                    let d = self.builtin_default(lname, i, &sig[i])?;
                    out.push(Arg { value: (*d).clone(), spread: false, name: None });
                }
            }
        }
        Ok(Some(out))
    }

    fn invoke_closure(&mut self, closure: &Closure, args: Vec<ArgVal>, call_line: u32) -> EResult<Value> {
        if let Some(f) = &closure.forward {
            return self.call_forward(closure, f, args, call_line);
        }
        let ctx = CallCtx {
            name: closure.name.clone(),
            trace_fn: closure.name.clone(),
            // A closure with a class scope shows in traces like a method of the class
            trace_class: closure.scope.as_ref().map(|s| (s.name.to_string(), if closure.this.is_some() { "->" } else { "::" })),
            this: closure.this.clone(),
            scope: closure.scope.clone(),
            static_class: closure.this.as_ref().map(|t| t.class.clone()).or_else(|| closure.scope.clone()),
        };
        // Each call gets its own copy of by-value captures; by-reference ones stay shared
        self.invoke_with(&closure.func, ctx, closure.bound.clone(), args, call_line)
    }

    fn invoke_with(
        &mut self,
        f: &Rc<FuncDecl>,
        ctx: CallCtx,
        bound: Vec<(String, Value)>,
        args: Vec<ArgVal>,
        call_line: u32,
    ) -> EResult<Value> {
        if self.frames.len() + self.nesting > MAX_CALL_DEPTH {
            // php-src measures the native stack; its default limit reads like this
            return Err(Fatal::error(
                "Maximum call stack size of 8355840 bytes (zend.max_allowed_stack_size - zend.reserved_stack_size) reached. Infinite recursion?",
            ));
        }
        let shown = args.iter().map(ArgVal::value).collect();
        let vars = bound.into_iter().collect();
        let by_builtin = self.internal_calls.last().is_some_and(|(depth, _, include)| !include && *depth == self.frames.len());
        let caller_file = self.enter_file(&f.file);
        let call_file = caller_file.path.clone();
        self.frames.push(Frame { vars, func: Some(f.clone()), ctx, call_line, args: shown, by_builtin, call_file });
        let result = self.invoke_inner(f, args, call_line);
        if f.is_generator && result.is_ok() {
            // The bound arguments wait in the generator until it is first resumed
            let frame = self.frames.pop().expect("pushed above");
            self.enter_file(&caller_file);
            return Ok(self.make_generator(f, frame));
        }
        let result = result.map_err(|e| self.frame_error(e));
        self.frames.pop();
        self.enter_file(&caller_file);
        self.with_destructors(result)
    }

    /// An error leaving a frame. An internal error becomes its exception object
    /// here, while the failing frame is still on the stack: the trace is
    /// complete, and like php-src the object exists before the frame's own
    /// objects are released
    fn frame_error(&mut self, mut e: Fatal) -> Fatal {
        if e.object.is_some() {
            return e;
        }
        e.line = Some(e.line.unwrap_or_else(diag::line));
        e.file.get_or_insert_with(diag::file);
        if e.trace.is_none() {
            e.trace = Some(self.trace());
        }
        if e.uncatchable {
            return e;
        }
        Fatal::thrown(self.exception_object(e))
    }

    fn invoke_inner(&mut self, f: &FuncDecl, args: Vec<ArgVal>, call_line: u32) -> EResult<Value> {
        diag::set_line(f.line);
        let fname = self.ctx().name.clone();
        let required = f.required();
        let (args, named): (Vec<ArgVal>, Vec<ArgVal>) = args.into_iter().partition(|a| !matches!(a, ArgVal::Named(..)));
        if args.len() < required {
            let exact = required == f.params.len();
            return Err(Fatal::new(
                "ArgumentCountError",
                format!(
                    "Too few arguments to function {fname}(), {} passed in {} on line {call_line} and {} {required} expected",
                    args.len(),
                    self.frame().call_file,
                    if exact { "exactly" } else { "at least" }
                ),
            ));
        }
        let strict = strict_types() && !self.frame().by_builtin;
        let mut named = named.into_iter();
        let mut args = args.into_iter();
        for (i, p) in f.params.iter().enumerate() {
            let type_error = |v: &Value, file: &str| {
                Fatal::type_error(format!(
                    "{fname}(): Argument #{} (${}) must be of type {}, {} given, called in {file} on line {call_line}",
                    i + 1,
                    p.name,
                    p.ty.as_ref().map_or("mixed", |t| t.display.as_str()),
                    v.value_name()
                ))
            };
            if p.variadic {
                let mut rest = PhpArray::new();
                // Positional arguments first, then the named ones under their names
                let extra = named.by_ref().map(|a| match a {
                    ArgVal::Named(n, v) => (Some(n), *v),
                    other => (None, other),
                });
                for (offset, (name, a)) in args.by_ref().map(|a| (None, a)).chain(extra).enumerate() {
                    let type_error = |v: &Value, file: &str| {
                        let n = i + offset + 1;
                        let param = if offset == 0 { format!(" (${})", p.name) } else { String::new() };
                        Fatal::type_error(format!(
                            "{fname}(): Argument #{n}{param} must be of type {}, {} given, called in {file} on line {call_line}",
                            p.ty.as_ref().map_or("mixed", |t| t.display.as_str()), v.value_name()
                        ))
                    };
                    let elem = match a {
                        ArgVal::Ref(r) if p.by_ref => {
                            let v = value::with_deref(&r, Value::clone);
                            let v = self.check_type(v, p.ty.as_ref(), &type_error, strict)?;
                            *r.try_borrow_mut().map_err(|_| recursive_reference())? = v;
                            Value::Ref(r)
                        },
                        ArgVal::Ref(r) => {
                            let v = value::with_deref(&r, Value::clone);
                            self.check_type(v, p.ty.as_ref(), &type_error, strict)?
                        }
                        ArgVal::Val(v) => self.check_type(v, p.ty.as_ref(), &type_error, strict)?,
                        ArgVal::Missing | ArgVal::Named(..) => continue,
                    };
                    match name {
                        Some(n) => rest.insert(Key::Str(n.into_bytes().into()), elem),
                        None => {
                            rest.push(elem).map_err(|_| next_element_occupied())?;
                        }
                    }
                }
                self.frame().vars.insert(p.name.clone(), Value::Array(Rc::new(rest)));
                break;
            }
            let slot = match args.next() {
                Some(ArgVal::Ref(r)) => {
                    if p.ty.is_some() {
                        let v = value::with_deref(&r, Value::clone);
                        let v = self.check_type(v, p.ty.as_ref(), &type_error, strict)?;
                        *r.try_borrow_mut().map_err(|_| recursive_reference())? = v;
                    }
                    Value::Ref(r)
                }
                Some(ArgVal::Val(v)) => {
                    if p.by_ref {
                        diag::warning(format!(
                            "{fname}(): Argument #{} (${}) must be passed by reference, value given",
                            i + 1,
                            p.name
                        ));
                    }
                    self.check_type(v, p.ty.as_ref(), &type_error, strict)?
                }
                // Skipped by named arguments, or not passed at all
                Some(ArgVal::Missing) | Some(ArgVal::Named(..)) | None => match &p.default {
                    Some(d) => {
                        let v = self.expr(d)?;
                        if let Some(shown) = self.frame().args.get_mut(i) {
                            *shown = v.clone();
                        }
                        v
                    }
                    None => {
                        return Err(Fatal::new(
                            "ArgumentCountError",
                            format!("{fname}(): Argument #{} (${}) not passed", i + 1, p.name),
                        ))
                    }
                },
            };
            self.frame().vars.insert(p.name.clone(), slot);
        }
        if f.kind == FuncKind::Method && f.name.eq_ignore_ascii_case("__construct") {
            self.promote_params(f)?;
        }
        if f.is_generator {
            return Ok(Value::Null);
        }

        let ret = match self.block(&f.body)? {
            Flow::Return(v) => Some(v),
            _ => None,
        };
        let Some(ty) = &f.ret else { return Ok(ret.unwrap_or(Value::Null)) };
        // `mixed` allows anything, but not falling off the end
        if ty.types.iter().any(|group| group.len() == 1 && group[0] == "void") {
            return Ok(Value::Null);
        }
        let given = match ret {
            Some(v) => v,
            None => {
                return Err(Fatal::type_error(format!(
                    "{fname}(): Return value must be of type {}, none returned",
                    ty.display
                )))
            }
        };
        let type_error = |v: &Value, _: &str| {
            Fatal::type_error(format!(
                "{fname}(): Return value must be of type {}, {} returned",
                ty.display,
                v.value_name()
            ))
        };
        self.check_param(given, Some(ty), &type_error)
    }

    /// Type-checks (and unless strict_types is on, coerces) a value for a declared type
    fn check_param(&mut self, v: Value, ty: Option<&TypeHint>, err: &dyn Fn(&Value, &str) -> Fatal) -> EResult<Value> {
        self.check_type(v, ty, err, strict_types())
    }

    fn check_type(
        &mut self,
        v: Value,
        ty: Option<&TypeHint>,
        err: &dyn Fn(&Value, &str) -> Fatal,
        strict: bool,
    ) -> EResult<Value> {
        let Some(ty) = ty else { return Ok(v) };
        // Stringable objects pass for `string` by calling __toString
        if let Value::Object(o) = &v
            && !strict
            && !self.object_matches(o, ty)
            && ty.allows("string")
            && o.class.find_method("__tostring").is_some()
        {
            return Ok(Value::str(self.stringify(&v)?));
        }
        let ctx = self.ctx();
        let tc = TypeCtx {
            functions: &self.functions,
            classes: &self.classes,
            scope: ctx.scope.as_deref(),
            static_class: ctx.static_class.as_deref(),
        };
        coerce(v, ty, &tc, strict).map_err(|v| err(&v, &self.frame().call_file))
    }

    /// The stack trace of the current call frames, innermost first. Built-in
    /// calls in progress sit between the frames; anything a built-in calls
    /// shows as called from `[internal function]`
    fn trace(&self) -> Vec<TraceFrame> {
        let calls = &self.internal_calls;
        // Whether something at this depth was called by one of the first `n` built-in calls
        let by_builtin = |depth: usize, n: usize| calls[..n].iter().any(|(d, _, include)| !include && *d == depth);
        let builtin_frame = |i: usize, innermost: bool| {
            let (depth, f, include) = &calls[i];
            let mut f = f.clone();
            if by_builtin(*depth, i) {
                f.file.clear();
            }
            if *include && innermost {
                f.args.clear();
            }
            f
        };
        let mut out = Vec::new();
        let mut pending = calls.len();
        for i in (1..self.frames.len()).rev() {
            while pending > 0 && calls[pending - 1].0 > i {
                pending -= 1;
                out.push(builtin_frame(pending, out.is_empty()));
            }
            let f = &self.frames[i];
            let internal = by_builtin(i, pending) || (i == 1 && self.in_shutdown);
            out.push(TraceFrame {
                file: if internal { String::new() } else { f.call_file.to_string() },
                line: f.call_line,
                function: f.ctx.trace_fn.clone(),
                class: f.ctx.trace_class.clone(),
                args: f.args.clone(),
            });
        }
        while pending > 0 {
            pending -= 1;
            out.push(builtin_frame(pending, out.is_empty()));
        }
        out
    }

    fn callback(&mut self, fname: &str, n: usize, v: &Value) -> EResult<Callable> {
        if let Some(c) = self.callable(v)? {
            return Ok(c);
        }
        let detail = self.not_callable_reason(v)?;
        Err(Fatal::type_error(format!("{fname}(): Argument #{n} ($callback) must be a valid callback, {detail}")))
    }

    fn call_value(&mut self, f: &Callable, args: Vec<Value>, line: u32) -> EResult<Value> {
        match f {
            Callable::Builtin(_) => {}
            _ => return self.call_value_args(f, args.into_iter().map(ArgVal::Val).collect(), line),
        }
        let vals = || args.clone().into_iter().map(ArgVal::Val).collect::<Vec<_>>();
        let v = match f {
            Callable::User(f) => {
                let ctx = CallCtx { name: f.name.clone(), trace_fn: f.name.clone(), ..CallCtx::default() };
                self.invoke_with(f, ctx, Vec::new(), vals(), line)?
            }
            Callable::Closure(o) => {
                let c = o.closure().expect("checked when the callable was resolved");
                self.invoke_closure(c, vals(), line)?
            }
            Callable::Method { this, method, class } => self.call_method(this.clone(), method, vals(), line, class.clone())?,
            Callable::Builtin(name) if builtins::is_by_ref(name) => {
                // A callback cannot pass a variable by reference, so this works on a copy
                let mut args = args.clone().into_iter();
                let mut target = args.next().unwrap_or(Value::Null);
                diag::warning(format!("{name}(): Argument #1 ($array) must be passed by reference, value given"));
                builtins::call_by_ref(name, &mut target, args.collect())?
            }
            Callable::Builtin(name) => {
                self.internal_call(name.clone(), None, args.clone(), line, |this| this.call_builtin(name, name, args, line))?
            }
        };
        diag::set_line(line);
        Ok(v)
    }

    /// `$f(...)`: closures, invokable objects, and strings or arrays naming a function
    /// Calls a PHP callable with arguments that may be references
    fn call_value_args(&mut self, f: &Callable, args: Vec<ArgVal>, line: u32) -> EResult<Value> {
        let v = match f {
            Callable::User(f) => {
                let ctx = CallCtx { name: f.name.clone(), trace_fn: f.name.clone(), ..CallCtx::default() };
                self.invoke_with(f, ctx, Vec::new(), args, line)?
            }
            Callable::Closure(o) => {
                let c = o.closure().expect("checked when the callable was resolved");
                self.invoke_closure(c, args, line)?
            }
            Callable::Method { this, method, class } => self.call_method(this.clone(), method, args, line, class.clone())?,
            Callable::Builtin(_) => {
                let vals = args.iter().map(ArgVal::value).collect();
                return self.call_value(f, vals, line);
            }
        };
        diag::set_line(line);
        Ok(v)
    }

    /// Whether a callable takes its `n`th parameter by reference
    fn param_by_ref(f: &Callable, n: usize) -> bool {
        let decl = match f {
            Callable::User(f) => Some(f.clone()),
            Callable::Closure(o) => o.closure().map(|c| c.func.clone()),
            Callable::Method { method, .. } => match &method.body {
                object::MethodBody::User(f) => Some(f.clone()),
                object::MethodBody::Native(_) => None,
            },
            Callable::Builtin(_) => None,
        };
        decl.is_some_and(|f| f.params.get(n).is_some_and(|p| p.by_ref))
    }

    fn array_walk(&mut self, path: &Path<'_>, vals: Vec<Value>, line: u32) -> EResult<Value> {
        let n = vals.len() + 1;
        let mut vals = vals.into_iter();
        let Some(cb) = vals.next() else { return Err(builtins::arg_count("array_walk", 2, n)) };
        let extra = vals.next();
        let cb = self.callback("array_walk", 2, &cb)?;
        let by_ref = Self::param_by_ref(&cb, 0);
        let current = self.with_place(path, false, true, |slot| Ok(slot.clone()))?;
        let Value::Array(arr) = current else {
            return Err(Fatal::type_error(format!(
                "array_walk(): Argument #1 ($array) must be of type array, {} given",
                current.value_name()
            )));
        };
        let keys: Vec<Key> = arr.keys().cloned().collect();
        drop(arr);
        for k in keys {
            let mut elem = path.clone();
            elem.dims.push(Some(k.to_value()));
            // Elements removed by the callback are skipped
            let value = self.with_place(path, false, true, |slot| {
                Ok(match slot {
                    Value::Array(a) => a.get(&k).map(Value::copy_out),
                    _ => None,
                })
            })?;
            let Some(value) = value else { continue };
            let first = if by_ref { ArgVal::Ref(self.make_ref(&elem)?) } else { ArgVal::Val(value) };
            let mut args = vec![first, ArgVal::Val(k.to_value())];
            if let Some(e) = &extra {
                args.push(ArgVal::Val(e.clone()));
            }
            self.call_value_args(&cb, args, line)?;
        }
        Ok(Value::Bool(true))
    }

    fn call_value_expr(&mut self, f: Value, args: &[Arg], line: u32) -> EResult<Value> {
        match &f {
            Value::Object(o) => {
                if let Some(c) = o.closure() {
                    if let Some(fw) = &c.forward {
                        return self.call_forward_ast(c, fw, args, line);
                    }
                    let bound = self.bind_args(&c.func, &c.name, args)?;
                    return self.invoke_closure(c, bound, line);
                }
                if o.class.find_method("__invoke").is_none() {
                    return Err(Fatal::error(format!("Object of type {} is not callable", o.class.name)));
                }
                self.method_call(f.clone(), "__invoke", args, line)
            }
            Value::Str(name) => {
                let name = String::from_utf8_lossy(name).into_owned();
                if let Some((class, method)) = name.split_once("::") {
                    let class = crate::ast::ClassRef::Named(class.to_string());
                    return self.static_call(&class, method, args, line);
                }
                self.call(name.trim_start_matches('\\'), args, line, false)
            }
            Value::Array(_) => {
                let Some(c) = self.callable(&f)? else { return Err(Fatal::error("Array callback must have exactly two elements")) };
                let vals = self.eval_args(args)?;
                self.call_value(&c, vals, line)
            }
            _ => Err(Fatal::error(format!("Value of type {} is not callable", f.type_name()))),
        }
    }

    /// usort / uasort / uksort: sorts a copy with the user comparator, then stores it
    fn user_sort(&mut self, lname: &str, path: &Path<'_>, vals: Vec<Value>, line: u32) -> EResult<Value> {
        let Some(cb) = vals.first() else {
            return Err(builtins::arg_count(lname, 2, 1));
        };
        let cb = self.callback(lname, 2, cb)?;
        let current = self.with_place(path, false, true, |slot| Ok(slot.clone()))?;
        let Value::Array(arr) = current else {
            return Err(Fatal::type_error(format!(
                "{lname}(): Argument #1 ($array) must be of type array, {} given",
                current.value_name()
            )));
        };
        let mut entries: Vec<(Key, Value)> = arr.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let by_key = lname == "uksort";
        builtins::try_merge_sort(&mut entries, |a, b| {
            let args = if by_key { vec![a.0.to_value(), b.0.to_value()] } else { vec![a.1.copy_out(), b.1.copy_out()] };
            let r = self.call_value(&cb, args, line)?;
            Ok(r.to_int().cmp(&0))
        })?;
        // The callback may have changed the variable meanwhile; PHP sorts in place, we store the result
        let sorted = if lname == "usort" {
            PhpArray::from_values(entries.into_iter().map(|(_, v)| v))
        } else {
            entries.into_iter().collect()
        };
        self.with_place(path, false, true, |slot| {
            *slot = Value::Array(Rc::new(sorted));
            Ok(())
        })?;
        Ok(Value::Bool(true))
    }

    fn array_map(&mut self, vals: Vec<Value>, line: u32) -> EResult<Value> {
        let mut vals = vals.into_iter();
        let (Some(cb), Some(first)) = (vals.next(), vals.next()) else {
            return Err(builtins::arg_count("array_map", 2, 0));
        };
        let mut arrays = vec![first];
        arrays.extend(vals);
        for (i, a) in arrays.iter().enumerate() {
            if !matches!(a, Value::Array(_)) {
                return Err(Fatal::type_error(format!(
                    "array_map(): Argument #{} (${}) must be of type array, {} given",
                    i + 2,
                    if i == 0 { "array" } else { "arrays" },
                    a.value_name()
                )));
            }
        }
        let arrays: Vec<Rc<PhpArray>> =
            arrays.into_iter().map(|a| if let Value::Array(a) = a { a } else { unreachable!() }).collect();
        let cb = if cb.is_null() { None } else { Some(self.callback("array_map", 1, &cb)?) };

        if arrays.len() == 1 {
            // With one array, keys are preserved
            let mut out = PhpArray::new();
            for (k, v) in arrays[0].iter() {
                let mapped = match &cb {
                    Some(f) => self.call_value(f, vec![v.copy_out()], line)?,
                    None => v.copy_out(),
                };
                out.insert(k.clone(), mapped);
            }
            return Ok(Value::Array(Rc::new(out)));
        }
        // With several arrays, elements are zipped by position and shorter ones padded with null
        let longest = arrays.iter().map(|a| a.len()).max().unwrap_or(0);
        let columns: Vec<Vec<Value>> = arrays.iter().map(|a| a.values().map(Value::copy_out).collect()).collect();
        let mut out = Vec::with_capacity(longest);
        for i in 0..longest {
            let args: Vec<Value> = columns.iter().map(|c| c.get(i).cloned().unwrap_or(Value::Null)).collect();
            out.push(match &cb {
                Some(f) => self.call_value(f, args, line)?,
                None => list(args),
            });
        }
        Ok(list(out))
    }

    fn array_filter(&mut self, vals: Vec<Value>, line: u32) -> EResult<Value> {
        let mut vals = vals.into_iter();
        let arr = match vals.next() {
            Some(Value::Array(a)) => a,
            Some(other) => {
                return Err(Fatal::type_error(format!(
                    "array_filter(): Argument #1 ($array) must be of type array, {} given",
                    other.value_name()
                )))
            }
            None => return Err(builtins::arg_count("array_filter", 1, 0)),
        };
        let cb = match vals.next() {
            Some(v) if !v.is_null() => Some(self.callback("array_filter", 2, &v)?),
            _ => None,
        };
        let mode = vals.next().map_or(0, |m| m.to_int());
        let mut out = PhpArray::new();
        for (k, v) in arr.iter() {
            let keep = match &cb {
                None => v.to_bool(),
                Some(f) => {
                    let args = match mode {
                        builtins::ARRAY_FILTER_USE_KEY => vec![k.to_value()],
                        builtins::ARRAY_FILTER_USE_BOTH => vec![v.copy_out(), k.to_value()],
                        _ => vec![v.copy_out()],
                    };
                    self.call_value(f, args, line)?.to_bool()
                }
            };
            if keep {
                out.insert(k.clone(), v.clone());
            }
        }
        Ok(Value::Array(Rc::new(out)))
    }

    fn make_closure(&mut self, f: &Rc<FuncDecl>) -> EResult<Value> {
        let mut bound = Vec::with_capacity(f.uses.len());
        for (name, by_ref) in &f.uses {
            if *by_ref {
                let r = self.make_ref(&Path::var(name))?;
                bound.push((name.clone(), Value::Ref(r)));
            } else if f.kind == FuncKind::Arrow {
                // Arrow functions capture only what exists; using anything else warns when called
                if let Some(v) = self.frame().vars.get(name) {
                    bound.push((name.clone(), v.copy_out()));
                }
            } else {
                bound.push((name.clone(), self.lookup(name)));
            }
        }
        let name = format!("{{closure:{}:{}}}", self.file.path, f.line);
        // Closures created in a method keep its $this (unless static) and class scope
        let this = if f.is_static { None } else { self.ctx().this.clone() };
        let scope = self.ctx().scope.clone();
        let closure = Closure { func: f.clone(), bound, name, file: self.file.path.to_string(), this, scope, forward: None };
        Ok(Value::Object(Object::new(self.classes["closure"].clone(), Internal::Closure(closure))))
    }
}

enum Callable {
    User(Rc<FuncDecl>),
    Closure(Rc<Object>),
    Method { this: Option<Rc<Object>>, method: Rc<Method>, class: Rc<ClassEntry> },
    Builtin(String),
}

/// Puts a named argument into the position of its parameter, or with the
/// extra named arguments a variadic parameter collects
fn place_named(f: &FuncDecl, bound: &mut Vec<ArgVal>, extra: &mut Vec<(String, ArgVal)>, name: String, v: ArgVal) -> EResult<()> {
    let overwrites = || Fatal::error(format!("Named parameter ${name} overwrites previous argument"));
    match f.params.iter().position(|p| p.name == name && !p.variadic) {
        Some(i) => {
            if i < bound.len() {
                if !matches!(bound[i], ArgVal::Missing) {
                    return Err(overwrites());
                }
                bound[i] = v;
            } else {
                bound.resize_with(i, || ArgVal::Missing);
                bound.push(v);
            }
            Ok(())
        }
        None if f.params.last().is_some_and(|p| p.variadic) => {
            if extra.iter().any(|(n, _)| *n == name) {
                return Err(overwrites());
            }
            extra.push((name, v));
            Ok(())
        }
        None => Err(Fatal::error(format!("Unknown named parameter ${name}"))),
    }
}

/// The entries of an unpacked argument list: string keys are named arguments
fn spread_entries(v: Value) -> EResult<Vec<(Key, Value)>> {
    match v {
        Value::Array(arr) => Ok(arr.iter().map(|(k, v)| (k.clone(), v.copy_out())).collect()),
        other => Err(Fatal::type_error(format!(
            "Only arrays and Traversables can be unpacked, {} given",
            other.value_name()
        ))),
    }
}

fn spread(v: Value) -> EResult<Vec<Value>> {
    match v {
        Value::Array(arr) => {
            if arr.keys().any(|k| matches!(k, Key::Str(_))) {
                return Err(Fatal::error("Named arguments after argument unpacking are not supported for built-in functions yet"));
            }
            Ok(arr.values().map(Value::copy_out).collect())
        }
        other => Err(Fatal::type_error(format!(
            "Only arrays and Traversables can be unpacked, {} given",
            other.value_name()
        ))),
    }
}

fn list(values: Vec<Value>) -> Value {
    Value::Array(Rc::new(PhpArray::from_values(values)))
}

/// `(array)`: objects expose their properties under the mangled names, and
/// numeric property names become integer keys
fn cast_array(v: Value) -> Value {
    match v {
        Value::Array(_) => v,
        Value::Null => Value::Array(Rc::new(PhpArray::new())),
        Value::Object(ref o) if o.closure().is_some() => list(vec![v]),
        Value::Object(o) => Value::Array(Rc::new(
            o.export_props()
                .into_iter()
                .map(|(k, v)| match k {
                    Key::Str(s) => (Key::from_bytes(&s), v),
                    k => (k, v),
                })
                .collect(),
        )),
        other => list(vec![other]),
    }
}

fn foreach_type_warning(v: &Value) {
    diag::warning(format!("foreach() argument must be of type array|object, {} given", v.value_name()));
}

/// Walks `dims` from `slot`, following references, then runs `f` on the final storage
fn descend<T>(
    slot: &mut Value,
    dims: &[Option<Value>],
    read: bool,
    deref_last: bool,
    f: impl FnOnce(&mut Value) -> EResult<T>,
) -> EResult<T> {
    if let Value::Ref(r) = slot
        && (!dims.is_empty() || deref_last) {
            let r = r.clone();
            // Fails only for a reference cycle that is already being written to
            let mut inner = r.try_borrow_mut().map_err(|_| recursive_reference())?;
            return descend(&mut inner, dims, read, deref_last, f);
        }
    match dims.split_first() {
        None => f(slot),
        Some((dim, rest)) => {
            let next = write_dim(slot, dim.as_ref(), read && rest.is_empty())?;
            descend(next, rest, read, deref_last, f)
        }
    }
}

/// Walks `dims` from `v` through arrays, looking for an object that one of the
/// offsets is applied to. `depth` is the position of the offset applied to `v`
fn probe_object(v: &Value, dims: &[Option<Value>], depth: usize) -> Option<(usize, Rc<Object>)> {
    match v {
        Value::Ref(r) => probe_object(&*r.try_borrow().ok()?, dims, depth),
        Value::Object(o) if depth < dims.len() => Some((depth, o.clone())),
        // The element the last offset lands on is not indexed itself, so it is not looked up
        Value::Array(a) if depth + 1 < dims.len() => {
            let key = peek_key(dims[depth].as_ref()?)?;
            probe_object(a.get(&key)?, dims, depth + 1)
        }
        _ => None,
    }
}

/// The array key a value converts to, without the diagnostics of to_key
fn peek_key(v: &Value) -> Option<Key> {
    Some(match v {
        Value::Int(i) => Key::Int(*i),
        Value::Str(s) => Key::from_bytes(s),
        Value::Bool(b) => Key::Int(*b as i64),
        Value::Float(f) if float_fits_int(*f) => Key::Int(*f as i64),
        Value::Null => Key::Str(Rc::from(&b""[..])),
        Value::Ref(r) => return peek_key(&*r.try_borrow().ok()?),
        _ => return None,
    })
}

/// Converts a slot into a reference holding its old value, or returns its existing reference
fn make_ref_slot(slot: &mut Value) -> RefCellValue {
    if let Value::Ref(r) = slot {
        return r.clone();
    }
    let r = Rc::new(RefCell::new(std::mem::replace(slot, Value::Null)));
    *slot = Value::Ref(r.clone());
    r
}

/// Stores into a slot, writing through it if it is a reference
fn set_slot(slot: &mut Value, v: Value) -> EResult<()> {
    match slot {
        Value::Ref(r) => *r.try_borrow_mut().map_err(|_| recursive_reference())? = v,
        _ => *slot = v,
    }
    Ok(())
}

fn unset_in(slot: &mut Value, dims: &[Option<Value>], last: Option<Value>) -> EResult<()> {
    if let Value::Ref(r) = slot {
        let r = r.clone();
        let mut inner = r.try_borrow_mut().map_err(|_| recursive_reference())?;
        return unset_in(&mut inner, dims, last);
    }
    match dims.split_first() {
        Some((dim, rest)) => match (slot, dim) {
            (Value::Array(arr), Some(k)) => {
                let key = to_key(k, "array")?;
                // Only clone a shared array when there is actually something to remove
                if !arr.contains_key(&key) {
                    return Ok(());
                }
                match Rc::make_mut(arr).get_mut(&key) {
                    Some(next) => unset_in(next, rest, last),
                    None => Ok(()),
                }
            }
            (Value::Str(_), _) => Err(Fatal::error("Cannot unset string offsets")),
            (Value::Object(o), _) => Err(Fatal::error(format!("Cannot use object of type {} as array", o.class.name))),
            _ => Ok(()),
        },
        None => match (slot, last) {
            (Value::Array(arr), Some(k)) => {
                let key = to_key(&k, "array")?;
                if arr.contains_key(&key) {
                    Rc::make_mut(arr).remove(&key);
                }
                Ok(())
            }
            (_, None) => Err(Fatal::error("Cannot use [] for unsetting")),
            (Value::Str(_), _) => Err(Fatal::error("Cannot unset string offsets")),
            (Value::Object(o), _) => Err(Fatal::error(format!("Cannot use object of type {} as array", o.class.name))),
            _ => Ok(()),
        },
    }
}

/// What a type check needs to know about the calling context
struct TypeCtx<'a> {
    functions: &'a HashMap<String, Rc<FuncDecl>>,
    classes: &'a HashMap<String, Rc<ClassEntry>>,
    scope: Option<&'a ClassEntry>,
    static_class: Option<&'a ClassEntry>,
}

impl TypeCtx<'_> {
    /// Whether an object's class satisfies one member of a declared type
    fn class_matches(&self, class: &ClassEntry, ty: &str) -> bool {
        match ty {
            "self" => self.scope.is_some_and(|s| class.is_subclass_of(s.name)),
            "static" => self.static_class.is_some_and(|s| class.is_subclass_of(s.name)),
            "parent" => self.scope.and_then(|s| s.parent.as_deref()).is_some_and(|p| class.is_subclass_of(p.name)),
            "object" | "mixed" => true,
            "callable" => class.name == "Closure" || class.find_method("__invoke").is_some(),
            "iterable" => class.is_subclass_of("Traversable"),
            _ => class.is_subclass_of(ty),
        }
    }

    /// Whether a string names a user or built-in function, or `Class::method`
    fn names_callable(&self, s: &[u8]) -> bool {
        let name = String::from_utf8_lossy(s).trim_start_matches('\\').to_ascii_lowercase();
        if let Some((class, method)) = name.split_once("::") {
            return self.classes.get(class).is_some_and(|c| c.find_method(method).is_some());
        }
        self.functions.contains_key(&name) || builtins::exists(&name)
    }
}

/// Coercion of a value to a declared parameter or return type; in strict mode
/// only int to float. Err hands the value back for the error message
fn coerce(v: Value, ty: &TypeHint, ctx: &TypeCtx<'_>, strict: bool) -> Result<Value, Value> {
    if let Value::Object(o) = &v {
        if ty.types.iter().any(|group| group.iter().all(|t| ctx.class_matches(&o.class, t))) {
            return Ok(v);
        }
        return Err(v);
    }
    let actual = match &v {
        Value::Null => "null",
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Resource(_) => "resource",
        Value::Object(_) | Value::Ref(_) => unreachable!("handled above or dereferenced before type checks"),
    };
    let callable = ty.allows("callable")
        && match &v {
            Value::Str(s) => ctx.names_callable(s),
            Value::Array(a) => a.len() == 2,
            _ => false,
        };
    let exact = ty.allows(actual)
        || matches!(v, Value::Bool(_)) && ty.allows("bool")
        || matches!(v, Value::Array(_)) && ty.allows("iterable")
        || callable;
    if exact {
        return Ok(v);
    }
    // int -> float is always allowed
    if let Value::Int(i) = v
        && ty.allows("float") {
            return Ok(Value::Float(i as f64));
        }
    if strict || matches!(v, Value::Null | Value::Array(_)) {
        return Err(v);
    }
    // Scalar juggling, in PHP's order of preference: int, float, string, bool
    let numeric = match &v {
        Value::Str(s) => match parse_numeric(s) {
            (n, Numeric::Full) => Some(n),
            _ => None,
        },
        Value::Float(f) => Some(Num::Float(*f)),
        Value::Bool(b) => Some(Num::Int(*b as i64)),
        _ => None,
    };
    if ty.allows("int") {
        match numeric {
            Some(Num::Int(i)) => return Ok(Value::Int(i)),
            Some(Num::Float(f)) if f.fract() == 0.0 && float_fits_int(f) => return Ok(Value::Int(f as i64)),
            _ => {}
        }
    }
    if ty.allows("float")
        && let Some(n) = numeric {
            return Ok(Value::Float(n.to_f64()));
        }
    if ty.allows("string") && matches!(v, Value::Int(_) | Value::Float(_) | Value::Bool(_)) {
        return Ok(Value::str(v.to_bytes()));
    }
    if ty.allows("bool") {
        return Ok(Value::Bool(v.to_bool()));
    }
    // A fractional number for an int-only type is truncated with a deprecation
    if ty.allows("int")
        && let Some(Num::Float(f)) = numeric
            && f.is_finite() && float_fits_int(f) {
                return Ok(Value::Int(float_to_int_implicit(f, &v)));
            }
    Err(v)
}

/// String conversion of non-object values: arrays warn. Objects need the
/// interpreter (for __toString), see Interp::to_str
pub fn to_str(v: &Value) -> EResult<Vec<u8>> {
    match v {
        Value::Array(_) => diag::warning("Array to string conversion"),
        Value::Object(o) => {
            return Err(Fatal::error(format!("Object of class {} could not be converted to string", o.class.name)));
        }
        Value::Ref(r) => return value::with_deref(r, to_str),
        _ => {}
    }
    Ok(v.to_bytes())
}

fn next_element_occupied() -> Fatal {
    Fatal::error("Cannot add element to the array as the next element is already occupied")
}

/// Converts a value to an array key. `container` names the type in error messages
pub fn to_key(v: &Value, container: &str) -> EResult<Key> {
    Ok(match v {
        Value::Int(i) => Key::Int(*i),
        Value::Str(s) => Key::from_bytes(s),
        Value::Bool(b) => Key::Int(*b as i64),
        Value::Float(f) => Key::Int(float_to_int_implicit(*f, v)),
        Value::Null => {
            diag::deprecated("Using null as an array offset is deprecated, use an empty string instead");
            Key::Str(Rc::from(&b""[..]))
        }
        Value::Array(_) | Value::Object(_) => {
            return Err(Fatal::type_error(format!("Cannot access offset of type {} on {container}", v.type_name())));
        }
        Value::Resource(r) => {
            diag::warning(format!("Resource ID#{} used as offset, casting to integer ({})", r.id, r.id));
            Key::Int(r.id)
        }
        Value::Ref(r) => return value::with_deref(r, |v| to_key(v, container)),
    })
}

fn key_display(k: &Key) -> String {
    match k {
        Key::Int(i) => i.to_string(),
        Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s)),
    }
}

/// `$base[$index]` for reading
fn read_dim(base: &Value, index: &Value) -> EResult<Value> {
    match base {
        Value::Array(arr) => {
            let key = to_key(index, "array")?;
            Ok(match arr.get(&key) {
                Some(v) => v.copy_out(),
                None => {
                    diag::warning(format!("Undefined array key {}", key_display(&key)));
                    Value::Null
                }
            })
        }
        Value::Str(s) => {
            let off = string_offset(index)?;
            Ok(match byte_at(s, off) {
                Some(b) => Value::str(vec![b]),
                None => {
                    diag::warning(format!("Uninitialized string offset {off}"));
                    Value::str(Vec::new())
                }
            })
        }
        Value::Object(o) => Err(Fatal::error(format!("Cannot use object of type {} as array", o.class.name))),
        other => {
            diag::warning(format!("Trying to access array offset on {}", other.type_name()));
            Ok(Value::Null)
        }
    }
}

/// Converts a string offset index to an integer, with PHP's diagnostics
fn string_offset(index: &Value) -> EResult<i64> {
    match index {
        Value::Int(i) => Ok(*i),
        Value::Str(t) => match parse_numeric(t) {
            (Num::Int(i), Numeric::Full) => Ok(i),
            (n, Numeric::Leading | Numeric::Full) => {
                diag::warning(format!("Illegal string offset \"{}\"", String::from_utf8_lossy(t)));
                Ok(match n {
                    Num::Int(i) => i,
                    Num::Float(f) => f as i64,
                })
            }
            (_, Numeric::No) => Err(Fatal::type_error("Cannot access offset of type string on string")),
        },
        Value::Array(_) | Value::Object(_) => {
            Err(Fatal::type_error(format!("Cannot access offset of type {} on string", index.type_name())))
        }
        other => {
            diag::warning("String offset cast occurred");
            Ok(other.to_int())
        }
    }
}

/// The byte at a string offset; negative offsets count from the end
fn byte_at(s: &[u8], off: i64) -> Option<u8> {
    // checked so that i64::MIN cannot overflow
    let pos = if off < 0 { (s.len() as i64).checked_add(off)? } else { off };
    s.get(usize::try_from(pos).ok()?).copied()
}

fn assign_string_offset(s: &mut Vec<u8>, index: &Value, v: &Value) -> EResult<()> {
    let off = string_offset(index)?;
    let pos = if off < 0 {
        match (s.len() as i64).checked_add(off) {
            Some(p) if p >= 0 => p as usize,
            _ => {
                diag::warning(format!("Illegal string offset {off}"));
                return Ok(());
            }
        }
    } else {
        off as usize
    };
    let bytes = to_str(v)?;
    let Some(&b) = bytes.first() else {
        return Err(Fatal::error("Cannot assign an empty string to a string offset"));
    };
    // Writing past the end pads with spaces, as PHP does. Cap it so a huge offset
    // is an error rather than an attempt to allocate exabytes
    if pos >= s.len() {
        if pos > isize::MAX as usize / 2 {
            return Err(Fatal::error("String size overflow"));
        }
        s.resize(pos, b' ');
        s.push(b);
    } else {
        s[pos] = b;
    }
    Ok(())
}

/// One step of a write path: turns `cur` into an array if needed and returns the
/// element. With `read`, a missing key warns because its old value is used
fn write_dim<'a>(cur: &'a mut Value, dim: Option<&Value>, read: bool) -> EResult<&'a mut Value> {
    if matches!(cur, Value::Null) {
        *cur = Value::Array(Rc::new(PhpArray::new()));
    }
    if matches!(cur, Value::Bool(false)) {
        diag::deprecated("Automatic conversion of false to array is deprecated");
        *cur = Value::Array(Rc::new(PhpArray::new()));
    }
    match cur {
        Value::Array(arr) => {
            let arr = Rc::make_mut(arr);
            match dim {
                Some(k) => {
                    let key = to_key(k, "array")?;
                    if read && !arr.contains_key(&key) {
                        diag::warning(format!("Undefined array key {}", key_display(&key)));
                    }
                    Ok(arr.entry(key))
                }
                None => arr.push(Value::Null).map_err(|_| next_element_occupied()),
            }
        }
        Value::Str(_) if dim.is_none() => Err(Fatal::error("[] operator not supported for strings")),
        Value::Str(_) => Err(Fatal::error("Cannot use string offset as an array")),
        Value::Object(o) => Err(Fatal::error(format!("Cannot use object of type {} as array", o.class.name))),
        _ => Err(Fatal::error("Cannot use a scalar value as an array")),
    }
}

/// Given the result of a loop body, returns the Flow to propagate outward if the loop should exit
fn has_ref(items: &[Option<ListItem>]) -> bool {
    items.iter().flatten().any(|i| i.by_ref || matches!(&i.target.kind, ExprKind::List(inner) if has_ref(inner)))
}

/// The subject as shown in "Unhandled match case ...": scalars as literals, strings
/// escaped and cut at 15 bytes, anything else by type
fn match_case(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            let s = value::format_float(*f);
            if s.bytes().all(|c| c.is_ascii_digit() || c == b'-') { s + ".0" } else { s }
        }
        Value::Str(s) => {
            let mut out = String::from("'");
            for &c in &s[..s.len().min(15)] {
                match c {
                    b'\n' => out.push_str("\\n"),
                    b'\r' => out.push_str("\\r"),
                    b'\t' => out.push_str("\\t"),
                    0x0c => out.push_str("\\f"),
                    0x0b => out.push_str("\\v"),
                    b'\\' => out.push_str("\\\\"),
                    0x1b => out.push_str("\\e"),
                    c if !(32..=126).contains(&c) => out.push_str(&format!("\\x{c:02X}")),
                    c => out.push(c as char),
                }
            }
            if s.len() > 15 {
                out.push_str("...");
            }
            out.push('\'');
            out
        }
        Value::Array(_) => "of type array".into(),
        Value::Resource(_) => "of type resource".into(),
        Value::Object(o) => format!("of type {}", o.class.name),
        Value::Ref(r) => value::with_deref(r, match_case),
    }
}

fn loop_flow(flow: Flow) -> Option<Flow> {
    match flow {
        Flow::Normal | Flow::Continue(1) => None,
        Flow::Break(1) => Some(Flow::Normal),
        Flow::Break(n) => Some(Flow::Break(n - 1)),
        Flow::Continue(n) => Some(Flow::Continue(n - 1)),
        Flow::Return(v) => Some(Flow::Return(v)),
    }
}

fn binary(op: &str, l: &Value, r: &Value) -> EResult<Value> {
    let v = binary_op(op, l, r)?;
    if value::take_too_deep() {
        return Err(Fatal::error("Nesting level too deep - recursive dependency?"));
    }
    Ok(v)
}

fn binary_op(op: &str, l: &Value, r: &Value) -> EResult<Value> {
    use Ordering::*;
    Ok(match op {
        "+" => value::add(l, r)?,
        "-" => value::sub(l, r)?,
        "*" => value::mul(l, r)?,
        "/" => value::div(l, r)?,
        "%" => value::modulo(l, r)?,
        "**" => value::pow(l, r)?,
        "." => {
            let mut s = to_str(l)?;
            s.extend_from_slice(&to_str(r)?);
            Value::str(s)
        }
        "<<" => value::shift(l, r, true)?,
        ">>" => value::shift(l, r, false)?,
        "&" | "|" | "^" => value::bitwise(op, l, r)?,
        "xor" => Value::Bool(l.to_bool() ^ r.to_bool()),
        "==" => Value::Bool(l.loose_eq(r)),
        "!=" => Value::Bool(!l.loose_eq(r)),
        "===" => Value::Bool(l.strict_eq(r)),
        "!==" => Value::Bool(!l.strict_eq(r)),
        // `a > b` is evaluated as `b < a` in PHP, which matters for uncomparable arrays
        "<" => Value::Bool(loose_cmp(l, r) == Some(Less)),
        "<=" => Value::Bool(matches!(loose_cmp(l, r), Some(Less | Equal))),
        ">" => Value::Bool(loose_cmp(r, l) == Some(Less)),
        ">=" => Value::Bool(matches!(loose_cmp(r, l), Some(Less | Equal))),
        "<=>" => Value::Int(match loose_cmp(l, r) {
            Some(Less) => -1,
            Some(Equal) => 0,
            // Uncomparable (NaN, mismatched array keys) yields 1
            Some(Greater) | None => 1,
        }),
        _ => unreachable!("unknown binary operator {op}"),
    })
}

fn increment(v: &Value) -> EResult<Value> {
    Ok(match v {
        Value::Null => Value::Int(1),
        Value::Int(i) => i.checked_add(1).map(Value::Int).unwrap_or(Value::Float(*i as f64 + 1.0)),
        Value::Float(f) => Value::Float(f + 1.0),
        Value::Bool(_) => {
            diag::warning("Increment on type bool has no effect, this will change in the next major version of PHP");
            v.clone()
        }
        Value::Str(s) => match parse_numeric(s) {
            (_, Numeric::Full) => value::add(v, &Value::Int(1))?,
            _ => {
                diag::deprecated("Increment on non-numeric string is deprecated, use str_increment() instead");
                if s.is_empty() { Value::str(b"1".to_vec()) } else { Value::str(str_increment(s)) }
            }
        },
        Value::Array(_) | Value::Object(_) | Value::Resource(_) => {
            return Err(Fatal::type_error(format!("Cannot increment {}", v.type_name())));
        }
        Value::Ref(r) => return value::with_deref(r, increment),
    })
}

fn decrement(v: &Value) -> EResult<Value> {
    Ok(match v {
        Value::Null => {
            diag::warning("Decrement on type null has no effect, this will change in the next major version of PHP");
            Value::Null
        }
        Value::Bool(_) => {
            diag::warning("Decrement on type bool has no effect, this will change in the next major version of PHP");
            v.clone()
        }
        Value::Int(i) => i.checked_sub(1).map(Value::Int).unwrap_or(Value::Float(*i as f64 - 1.0)),
        Value::Float(f) => Value::Float(f - 1.0),
        Value::Str(s) if s.is_empty() => {
            diag::deprecated("Decrement on empty string is deprecated as non-numeric");
            Value::Int(-1)
        }
        Value::Str(s) => match parse_numeric(s) {
            (_, Numeric::Full) => value::sub(v, &Value::Int(1))?,
            _ => {
                diag::deprecated("Decrement on non-numeric string has no effect and is deprecated");
                v.clone()
            }
        },
        Value::Array(_) | Value::Object(_) | Value::Resource(_) => {
            return Err(Fatal::type_error(format!("Cannot decrement {}", v.type_name())));
        }
        Value::Ref(r) => return value::with_deref(r, decrement),
    })
}

/// Perl-style string increment: "a" -> "b", "Az" -> "Ba", "zz" -> "aaa"
fn str_increment(s: &[u8]) -> Vec<u8> {
    let mut out = s.to_vec();
    let mut i = out.len();
    while i > 0 {
        i -= 1;
        let c = out[i];
        let (wrapped, first) = match c {
            b'z' => (b'a', b'a'),
            b'Z' => (b'A', b'A'),
            b'9' => (b'0', b'1'),
            b'a'..=b'y' | b'A'..=b'Y' | b'0'..=b'8' => {
                out[i] = c + 1;
                return out;
            }
            // Stop at the first non-alphanumeric character
            _ => return out,
        };
        out[i] = wrapped;
        if i == 0 || !out[i - 1].is_ascii_alphanumeric() {
            out.insert(i, first);
            return out;
        }
    }
    out
}

fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "PHP_EOL" => Value::str(b"\n".to_vec()),
        "PHP_INT_MAX" => Value::Int(i64::MAX),
        "PHP_INT_MIN" => Value::Int(i64::MIN),
        "PHP_INT_SIZE" => Value::Int(8),
        "PHP_FLOAT_EPSILON" => Value::Float(f64::EPSILON),
        "PHP_FLOAT_MAX" => Value::Float(f64::MAX),
        "PHP_FLOAT_MIN" => Value::Float(f64::MIN_POSITIVE),
        "NAN" => Value::Float(f64::NAN),
        "INF" => Value::Float(f64::INFINITY),
        "ARRAY_FILTER_USE_KEY" => Value::Int(builtins::ARRAY_FILTER_USE_KEY),
        "ARRAY_FILTER_USE_BOTH" => Value::Int(builtins::ARRAY_FILTER_USE_BOTH),
        _ if name.starts_with("PHP_OUTPUT_HANDLER_") => Value::Int(output::output_constant(name)?),
        "PHP_VERSION" => Value::str(b"8.5.0".to_vec()),
        "PHP_MAJOR_VERSION" => Value::Int(8),
        "PHP_MINOR_VERSION" => Value::Int(5),
        "PHP_RELEASE_VERSION" => Value::Int(0),
        "PHP_VERSION_ID" => Value::Int(80500),
        "PHP_OS" | "PHP_OS_FAMILY" => Value::str(b"Linux".to_vec()),
        "DIRECTORY_SEPARATOR" => Value::str(b"/".to_vec()),
        "PATH_SEPARATOR" => Value::str(b":".to_vec()),
        "PHP_FLOAT_DIG" => Value::Int(15),
        "M_PI" => Value::Float(std::f64::consts::PI),
        "M_E" => Value::Float(std::f64::consts::E),
        "M_SQRT2" => Value::Float(std::f64::consts::SQRT_2),
        "E_ERROR" => Value::Int(1),
        "E_WARNING" => Value::Int(2),
        "E_PARSE" => Value::Int(4),
        "E_NOTICE" => Value::Int(8),
        "E_USER_ERROR" => Value::Int(256),
        "E_USER_WARNING" => Value::Int(512),
        "E_USER_NOTICE" => Value::Int(1024),
        "E_DEPRECATED" => Value::Int(8192),
        "E_USER_DEPRECATED" => Value::Int(16384),
        "E_ALL" => Value::Int(32767),
        _ => return builtins::constant(name),
    })
}
