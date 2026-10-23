//! Tree-walking interpreter that executes the AST directly.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use crate::array::{Key, PhpArray};
use crate::ast::{Arg, Expr, ExprKind, FuncDecl, FuncKind, Stmt, TypeHint};
use crate::builtins;
use crate::diag;
use crate::object::{self, ClassEntry, Closure, Object, ObjectKind, ThrowableData, TraceFrame};
use crate::value::{self, float_fits_int, float_to_int_implicit, loose_cmp, parse_numeric, ArithError, Num, Numeric, Value};

/// PHP code recursing deeper than this gets an Error instead of overflowing the
/// native stack. The interpreter thread is given a stack large enough for it
pub const MAX_CALL_DEPTH: usize = 10_000;

/// An uncaught PHP exception (Error hierarchy)
#[derive(Debug)]
pub struct Fatal {
    pub class: &'static str,
    pub msg: String,
    /// Where it was thrown and the stack trace, filled in as it leaves the innermost call
    pub line: Option<u32>,
    pub trace: Option<Vec<TraceFrame>>,
    /// A user-thrown exception object, which carries its own message, line and trace
    pub object: Option<Rc<Object>>,
}

impl Fatal {
    pub fn new(class: &'static str, msg: impl Into<String>) -> Self {
        Fatal { class, msg: msg.into(), line: None, trace: None, object: None }
    }

    fn thrown(object: Rc<Object>) -> Self {
        Fatal { class: object.class.name, msg: String::new(), line: None, trace: None, object: Some(object) }
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

/// A write target: variable name (borrowed from the AST) plus evaluated offsets (None for `[]`)
type Path<'a> = (&'a str, Vec<Option<Value>>);

type RefCellValue = Rc<RefCell<Value>>;

/// An argument after evaluation: by-reference parameters receive the reference itself
enum ArgVal {
    Val(Value),
    Ref(RefCellValue),
}

struct Frame {
    vars: HashMap<String, Value>,
    /// None for the main script
    func: Option<Rc<FuncDecl>>,
    /// The function as named in traces and errors (`{closure:FILE:LINE}` for closures)
    name: String,
    call_line: u32,
    /// Arguments as passed, for func_get_args() and stack traces
    args: Vec<Value>,
}

pub struct Interp<W: Write> {
    out: W,
    /// Script path as shown in diagnostics
    file: String,
    frames: Vec<Frame>,
    functions: HashMap<String, Rc<FuncDecl>>,
    /// `static` variables, keyed by the declaring function (null for the main script)
    statics: HashMap<*const FuncDecl, HashMap<String, RefCellValue>>,
    classes: HashMap<String, Rc<ClassEntry>>,
}

fn recursive_reference() -> Fatal {
    Fatal::error("Cannot modify a value through a reference to itself")
}

impl<W: Write> Interp<W> {
    pub fn new(out: W, file: String) -> Self {
        let main = Frame { vars: HashMap::new(), func: None, name: String::new(), call_line: 0, args: Vec::new() };
        Interp {
            out,
            file,
            frames: vec![main],
            functions: HashMap::new(),
            statics: HashMap::new(),
            classes: object::builtin_classes(),
        }
    }

    pub fn into_output(self) -> W {
        self.out
    }

    /// Runs a script. An uncaught error is reported the way PHP does; returns false then
    pub fn run(&mut self, stmts: &[Stmt]) -> bool {
        // Unconditional top-level functions exist before any code runs
        let result = self.hoist(stmts).and_then(|_| self.block(stmts));
        self.flush_diags();
        match result {
            Ok(_) => true,
            Err(e) => {
                let ex = self.exception_object(e);
                let text = String::from_utf8_lossy(&object::to_string(&ex).unwrap_or_default()).into_owned();
                let (file, line) = ex.throwable().map(|t| (t.file.clone(), t.line)).unwrap_or_default();
                let msg = format!("\nFatal error: Uncaught {text}\n  thrown in {file} on line {line}\n");
                let _ = self.out.write_all(msg.as_bytes());
                false
            }
        }
    }

    fn hoist(&mut self, stmts: &[Stmt]) -> EResult<()> {
        for s in stmts {
            if let Stmt::Function(f) = s {
                self.declare(f)?;
            }
        }
        Ok(())
    }

    fn declare(&mut self, f: &Rc<FuncDecl>) -> EResult<()> {
        let lname = f.name.to_ascii_lowercase();
        if let Some(existing) = self.functions.get(&lname) {
            // Hoisted functions are seen again when their statement executes
            if Rc::ptr_eq(existing, f) {
                return Ok(());
            }
            diag::set_line(f.line);
            return Err(Fatal::error(format!(
                "Cannot redeclare function {}() (previously declared in {}:{})",
                f.name, self.file, existing.line
            )));
        }
        if builtins::exists(&lname) {
            diag::set_line(f.line);
            return Err(Fatal::error(format!("Cannot redeclare function {}()", f.name)));
        }
        self.functions.insert(lname, f.clone());
        Ok(())
    }

    fn flush_diags(&mut self) {
        for d in diag::take() {
            let msg = format!("\n{}: {} in {} on line {}\n", d.level.label(), d.msg, self.file, d.line);
            let _ = self.out.write_all(msg.as_bytes());
        }
    }

    fn write(&mut self, bytes: &[u8]) -> EResult<()> {
        self.flush_diags();
        self.out.write_all(bytes).map_err(|e| Fatal::error(format!("write failed: {e}")))
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    fn block(&mut self, stmts: &[Stmt]) -> EResult<Flow> {
        for s in stmts {
            match self.stmt(s)? {
                Flow::Normal => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }

    fn stmt(&mut self, s: &Stmt) -> EResult<Flow> {
        match s {
            Stmt::Echo(exprs) => {
                for e in exprs {
                    let v = self.expr(e)?;
                    let bytes = to_str(&v)?;
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
            Stmt::Foreach { subject, key, value, by_ref: false, body } => {
                let arr = match self.expr(subject)? {
                    Value::Array(arr) => arr,
                    other => {
                        foreach_type_warning(&other);
                        return Ok(Flow::Normal);
                    }
                };
                // Iterating our own Rc handle means writes to the variable inside the
                // loop copy-on-write, so the loop sees the array as it was (PHP semantics)
                for (k, v) in arr.iter() {
                    if let Some(key_target) = key {
                        self.assign(key_target, k.to_value())?;
                    }
                    self.assign(value, v.copy_out())?;
                    if let Some(flow) = loop_flow(self.block(body)?) {
                        return Ok(flow);
                    }
                }
            }
            Stmt::Foreach { subject, key, value, by_ref: true, body } => {
                let path = self.path(subject)?;
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
                    Err(other) => {
                        foreach_type_warning(&other);
                        return Ok(Flow::Normal);
                    }
                };
                for k in keys {
                    let mut elem = path.clone();
                    elem.1.push(Some(k.to_value()));
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
            Stmt::Break(n) => return Ok(Flow::Break(*n)),
            Stmt::Continue(n) => return Ok(Flow::Continue(*n)),
            Stmt::Unset(targets) => {
                for t in targets {
                    self.unset(t)?;
                }
            }
            Stmt::Function(f) => self.declare(f)?,
            Stmt::Try { body, catches, finally } => {
                let mut result = self.block(body);
                if let Err(e) = result {
                    let ex = self.exception_object(e);
                    result = Err(Fatal::thrown(ex.clone()));
                    if let Some(c) = catches.iter().find(|c| c.types.iter().any(|t| ex.class.is_subclass_of(t))) {
                        result = match &c.var {
                            Some(var) => self.with_place(&(var.as_str(), Vec::new()), false, true, |slot| {
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
                        Err(new) => {
                            let Err(pending) = result else { return Err(new) };
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
                    let global = &mut self.frames[0].vars;
                    let r = make_ref_slot(global.entry(name.clone()).or_insert(Value::Null));
                    self.frame().vars.insert(name.clone(), Value::Ref(r));
                }
            }
            Stmt::StaticVar(vars) => {
                let key = self.frame().func.as_ref().map_or(std::ptr::null(), Rc::as_ptr);
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

    fn lookup(&mut self, name: &str) -> Value {
        match self.frame().vars.get(name) {
            Some(v) => v.copy_out(),
            None => {
                diag::warning(format!("Undefined variable ${name}"));
                Value::Null
            }
        }
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
                    s.extend_from_slice(&to_str(&v)?);
                }
                Value::str(s)
            }
            ExprKind::Var(name) => self.lookup(name),
            ExprKind::Const(name) => {
                constant(name).ok_or_else(|| Fatal::error(format!("Undefined constant \"{name}\"")))?
            }
            ExprKind::Array(items) => {
                let mut arr = PhpArray::new();
                for item in items {
                    let key = match &item.key {
                        Some(k) => Some(to_key(&self.expr(k)?, "array")?),
                        None => None,
                    };
                    let v = self.expr(&item.value)?;
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
            ExprKind::Index(base, Some(index)) => {
                let b = self.expr(base)?;
                let i = self.expr(index)?;
                read_dim(&b, &i)?
            }
            ExprKind::Assign(target, rhs) => {
                let v = self.expr(rhs)?;
                self.assign(target, v.clone())?;
                v
            }
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
                self.with_place(&path, true, true, |place| {
                    if let (".", Value::Str(s)) = (*op, &mut *place) {
                        // Append in place so `$s .= ...` in a loop stays linear
                        Rc::make_mut(s).extend_from_slice(&to_str(&r)?);
                    } else {
                        *place = binary(op, place, &r)?;
                    }
                    Ok(place.clone())
                })?
            }
            ExprKind::IncDec { target, inc, prefix } => {
                let path = self.path(target)?;
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
                    "(string)" => Value::str(to_str(&v)?),
                    "(bool)" => Value::Bool(v.to_bool()),
                    _ => unreachable!("unknown unary operator {op}"),
                }
            }
            ExprKind::Binary("&&", l, r) => Value::Bool(self.expr(l)?.to_bool() && self.expr(r)?.to_bool()),
            ExprKind::Binary("||", l, r) => Value::Bool(self.expr(l)?.to_bool() || self.expr(r)?.to_bool()),
            ExprKind::Binary("??", l, r) => match self.quiet(l)? {
                Some(v) if !v.is_null() => v,
                _ => self.expr(r)?,
            },
            ExprKind::Binary(op, l, r) => {
                let lv = self.expr(l)?;
                let rv = self.expr(r)?;
                binary(op, &lv, &rv)?
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
                    if self.quiet(t)?.is_none_or(|v| v.is_null()) {
                        return Ok(Value::Bool(false));
                    }
                }
                Value::Bool(true)
            }
            ExprKind::Empty(target) => Value::Bool(!self.quiet(target)?.is_some_and(|v| v.to_bool())),
            ExprKind::Call(name, args) => {
                let v = self.call(name.trim_start_matches('\\'), args, e.line)?;
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
            ExprKind::New(class, args) => self.instantiate(class, args, e.line)?,
            ExprKind::MethodCall { object, name, args, nullsafe } => {
                let o = self.expr(object)?;
                if *nullsafe && o.is_null() {
                    // `?->` short-circuits: the arguments are not evaluated
                    return Ok(Value::Null);
                }
                let v = self.method_call(o, name, args, e.line)?;
                diag::set_line(e.line);
                v
            }
            ExprKind::Prop { object, name, nullsafe } => {
                let o = self.expr(object)?;
                read_prop(&o, name, *nullsafe)?
            }
            ExprKind::InstanceOf(x, class) => {
                let v = self.expr(x)?;
                Value::Bool(matches!(&v, Value::Object(o) if o.class.is_subclass_of(class)))
            }
            ExprKind::Throw(x) => {
                return match self.expr(x)? {
                    Value::Object(o) if o.throwable().is_some() => Err(Fatal::thrown(o)),
                    _ => Err(Fatal::error("Can only throw objects")),
                };
            }
        })
    }

    /// Evaluates without "undefined" warnings, as isset/empty/?? do.
    /// None means the variable or a key along the way does not exist
    fn quiet(&mut self, e: &Expr) -> EResult<Option<Value>> {
        match &e.kind {
            ExprKind::Var(name) => Ok(self.frame().vars.get(name).map(Value::copy_out)),
            ExprKind::Index(base, Some(index)) => {
                let Some(b) = self.quiet(base)? else { return Ok(None) };
                let i = self.expr(index)?;
                Ok(match &b {
                    Value::Array(arr) => arr.get(&to_key(&i, "array")?).map(Value::copy_out),
                    Value::Str(s) => {
                        let off = match &i {
                            Value::Str(t) => match parse_numeric(t) {
                                (Num::Int(n), Numeric::Full) => Some(n),
                                _ => None,
                            },
                            Value::Float(f) => Some(float_to_int_implicit(*f, &i)),
                            Value::Array(_) => None,
                            other => Some(other.to_int()),
                        };
                        off.and_then(|o| byte_at(s, o)).map(|b| Value::str(vec![b]))
                    }
                    _ => None,
                })
            }
            _ => self.expr(e).map(Some),
        }
    }

    /// Resolves a write target into its variable name and evaluated offsets
    fn path<'e>(&mut self, target: &'e Expr) -> EResult<Path<'e>> {
        diag::set_line(target.line);
        match &target.kind {
            ExprKind::Var(name) => Ok((name.as_str(), Vec::new())),
            ExprKind::Index(base, index) => {
                let (name, mut dims) = self.path(base)?;
                dims.push(match index {
                    Some(i) => Some(self.expr(i)?),
                    None => None,
                });
                Ok((name, dims))
            }
            _ => Err(Fatal::error("Cannot assign to this expression")),
        }
    }

    /// Runs `f` on the storage for a path in the current frame, creating arrays and
    /// elements along the way. With `read`, the old value is about to be used
    /// (`.=`, `++`), so a missing variable or final key is reported like a read.
    /// With `deref_last`, `f` sees the value behind a final reference rather than
    /// the reference slot itself
    fn with_place<T>(
        &mut self,
        (name, dims): &Path<'_>,
        read: bool,
        deref_last: bool,
        f: impl FnOnce(&mut Value) -> EResult<T>,
    ) -> EResult<T> {
        let vars = &mut self.frames.last_mut().unwrap().vars;
        if !vars.contains_key(*name) {
            if read && dims.is_empty() {
                diag::warning(format!("Undefined variable ${name}"));
            }
            vars.insert(name.to_string(), Value::Null);
        }
        // Looked up again so the common (existing variable) path does not allocate a key
        let slot = vars.get_mut(*name).unwrap();
        descend(slot, dims, read, deref_last, f)
    }

    /// Turns the slot at `path` into a reference (if it is not one yet) and returns it
    fn make_ref(&mut self, path: &Path<'_>) -> EResult<RefCellValue> {
        self.with_place(path, false, false, |slot| Ok(make_ref_slot(slot)))
    }

    /// Makes `target` an alias of `r`, as `$target = &...` does
    fn bind_ref(&mut self, target: &Expr, r: RefCellValue) -> EResult<()> {
        let path = self.path(target)?;
        self.with_place(&path, false, false, |slot| {
            *slot = Value::Ref(r);
            Ok(())
        })
    }

    fn assign(&mut self, target: &Expr, v: Value) -> EResult<()> {
        let mut path = self.path(target)?;
        // `$s[i] = "x"` on a string replaces a byte rather than creating an element
        if let Some(Some(index)) = path.1.last() {
            let index = index.clone();
            path.1.pop();
            return self.with_place(&path, false, true, |parent| {
                if let Value::Str(s) = parent {
                    return assign_string_offset(Rc::make_mut(s), &index, &v);
                }
                set_slot(write_dim(parent, Some(&index), false)?, v)
            });
        }
        self.with_place(&path, false, true, |slot| {
            *slot = v;
            Ok(())
        })
    }

    fn unset(&mut self, target: &Expr) -> EResult<()> {
        let (name, mut dims) = self.path(target)?;
        let Some(last) = dims.pop() else {
            // Unsetting a variable only removes this name; a reference survives elsewhere
            self.frame().vars.remove(name);
            return Ok(());
        };
        match self.frame().vars.get_mut(name) {
            Some(slot) => unset_in(slot, &dims, last),
            None => {
                diag::warning(format!("Undefined variable ${name}"));
                Ok(())
            }
        }
    }

    fn call(&mut self, name: &str, args: &[Arg], line: u32) -> EResult<Value> {
        let lname = name.to_ascii_lowercase();
        if let Some(f) = self.functions.get(&lname).cloned() {
            let bound = self.bind_args(&f, args)?;
            return self.invoke(&f, bound, line);
        }

        // Built-ins that take their first argument by reference
        if builtins::BY_REF.contains(&lname.as_str()) || matches!(lname.as_str(), "usort" | "uasort" | "uksort") {
            let Some((first, rest)) = args.split_first() else {
                return Err(builtins::arg_count(name, 1, 0));
            };
            if !first.value.is_lvalue() {
                return Err(Fatal::error(format!("{lname}(): Argument #1 ($array) could not be passed by reference")));
            }
            let path = self.path(&first.value)?;
            let vals = self.eval_args(rest)?;
            if lname.starts_with('u') {
                return self.user_sort(&lname, &path, vals, line);
            }
            return self.with_place(&path, false, true, |place| builtins::call_by_ref(&lname, place, vals));
        }

        let vals = self.eval_args(args)?;
        self.call_builtin(name, &lname, vals, line)
    }

    /// Built-ins called with evaluated arguments
    fn call_builtin(&mut self, name: &str, lname: &str, vals: Vec<Value>, line: u32) -> EResult<Value> {
        match lname {
            "print" => {
                self.write(&to_str(&vals[0])?)?;
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
                let n = String::from_utf8_lossy(&n).to_ascii_lowercase();
                Ok(Value::Bool(self.functions.contains_key(&n) || builtins::exists(&n)))
            }
            "is_callable" => Ok(Value::Bool(vals.first().is_some_and(|v| self.callable_name(v).is_some()))),
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
                        args.type_name()
                    )));
                };
                self.call_value(&f, arr.values().map(Value::copy_out).collect(), line)
            }
            "array_map" => self.array_map(vals, line),
            "array_filter" => self.array_filter(vals, line),
            _ => builtins::call(name, lname, vals),
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
    fn bind_args(&mut self, f: &FuncDecl, args: &[Arg]) -> EResult<Vec<ArgVal>> {
        let mut bound = Vec::with_capacity(args.len());
        for a in args {
            if a.spread {
                let v = self.expr(&a.value)?;
                bound.extend(spread(v)?.into_iter().map(ArgVal::Val));
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
                        "{}(): Argument #{} (${pname}) could not be passed by reference",
                        f.name,
                        i + 1
                    )));
                }
                let path = self.path(&a.value)?;
                bound.push(ArgVal::Ref(self.make_ref(&path)?));
            } else {
                bound.push(ArgVal::Val(self.expr(&a.value)?));
            }
        }
        Ok(bound)
    }

    fn invoke(&mut self, f: &Rc<FuncDecl>, args: Vec<ArgVal>, call_line: u32) -> EResult<Value> {
        self.invoke_with(f, f.name.clone(), Vec::new(), args, call_line)
    }

    fn invoke_closure(&mut self, closure: &Closure, args: Vec<ArgVal>, call_line: u32) -> EResult<Value> {
        // Each call gets its own copy of by-value captures; by-reference ones stay shared
        let bound = closure.bound.clone();
        self.invoke_with(&closure.func, closure.name.clone(), bound, args, call_line)
    }

    fn invoke_with(
        &mut self,
        f: &Rc<FuncDecl>,
        name: String,
        bound: Vec<(String, Value)>,
        args: Vec<ArgVal>,
        call_line: u32,
    ) -> EResult<Value> {
        if self.frames.len() > MAX_CALL_DEPTH {
            return Err(Fatal::error("Maximum call stack size reached. Infinite recursion?"));
        }
        let shown = args
            .iter()
            .map(|a| match a {
                ArgVal::Val(v) => v.clone(),
                ArgVal::Ref(r) => value::with_deref(r, Value::clone),
            })
            .collect();
        let vars = bound.into_iter().collect();
        self.frames.push(Frame { vars, func: Some(f.clone()), name: name.clone(), call_line, args: shown });
        let result = self.invoke_inner(f, &name, args, call_line);
        // The trace is captured while the failing frames are still on the stack
        let result = result.map_err(|mut e| {
            if e.object.is_none() && e.trace.is_none() {
                e.line = Some(e.line.unwrap_or_else(diag::line));
                e.trace = Some(self.trace());
            }
            e
        });
        self.frames.pop();
        result
    }

    fn invoke_inner(&mut self, f: &FuncDecl, fname: &str, args: Vec<ArgVal>, call_line: u32) -> EResult<Value> {
        diag::set_line(f.line);
        let required = f.required();
        if args.len() < required {
            let exact = required == f.params.len();
            return Err(Fatal::new(
                "ArgumentCountError",
                format!(
                    "Too few arguments to function {}(), {} passed in {} on line {call_line} and {} {required} expected",
                    fname,
                    args.len(),
                    self.file,
                    if exact { "exactly" } else { "at least" }
                ),
            ));
        }
        let mut args = args.into_iter();
        for (i, p) in f.params.iter().enumerate() {
            let check = |v: Value| -> EResult<Value> {
                match &p.ty {
                    Some(ty) => coerce(v, ty, &|v| names_callable(&self.functions, v)).map_err(|v| {
                        Fatal::type_error(format!(
                            "{}(): Argument #{} (${}) must be of type {}, {} given, called in {} on line {call_line}",
                            fname,
                            i + 1,
                            p.name,
                            ty.display,
                            v.type_name(),
                            self.file
                        ))
                    }),
                    None => Ok(v),
                }
            };
            if p.variadic {
                let mut rest = PhpArray::new();
                for a in args.by_ref() {
                    let elem = match a {
                        ArgVal::Ref(r) if p.by_ref => Value::Ref(r),
                        ArgVal::Ref(r) => check(value::with_deref(&r, Value::clone))?,
                        ArgVal::Val(v) => check(v)?,
                    };
                    rest.push(elem).map_err(|_| next_element_occupied())?;
                }
                self.frame().vars.insert(p.name.clone(), Value::Array(Rc::new(rest)));
                break;
            }
            let slot = match args.next() {
                Some(ArgVal::Ref(r)) => {
                    if p.ty.is_some() {
                        let v = value::with_deref(&r, Value::clone);
                        let v = check(v)?;
                        *r.try_borrow_mut().map_err(|_| recursive_reference())? = v;
                    }
                    Value::Ref(r)
                }
                Some(ArgVal::Val(v)) => {
                    if p.by_ref {
                        diag::warning(format!(
                            "{}(): Argument #{} (${}) must be passed by reference, value given",
                            fname,
                            i + 1,
                            p.name
                        ));
                    }
                    check(v)?
                }
                None => match &p.default {
                    Some(d) => self.expr(d)?,
                    None => unreachable!("argument count was checked"),
                },
            };
            self.frame().vars.insert(p.name.clone(), slot);
        }

        let ret = match self.block(&f.body)? {
            Flow::Return(v) => Some(v),
            _ => None,
        };
        let Some(ty) = &f.ret else { return Ok(ret.unwrap_or(Value::Null)) };
        if ty.allows("void") {
            return Ok(Value::Null);
        }
        let given = match ret {
            Some(v) => v,
            None if ty.allows("null") => return Ok(Value::Null),
            None => {
                return Err(Fatal::type_error(format!(
                    "{}(): Return value must be of type {}, none returned",
                    fname, ty.display
                )))
            }
        };
        coerce(given, ty, &|v| names_callable(&self.functions, v)).map_err(|v| {
            Fatal::type_error(format!(
                "{}(): Return value must be of type {}, {} returned",
                fname,
                ty.display,
                v.type_name()
            ))
        })
    }

    /// The stack trace of the current call frames, innermost first
    fn trace(&self) -> Vec<TraceFrame> {
        self.frames
            .iter()
            .skip(1)
            .rev()
            .map(|f| TraceFrame { file: self.file.clone(), line: f.call_line, function: f.name.clone(), args: f.args.clone() })
            .collect()
    }

    fn make_closure(&mut self, f: &Rc<FuncDecl>) -> EResult<Value> {
        let mut bound = Vec::with_capacity(f.uses.len());
        for (name, by_ref) in &f.uses {
            if *by_ref {
                let r = self.make_ref(&(name.as_str(), Vec::new()))?;
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
        let name = format!("{{closure:{}:{}}}", self.file, f.line);
        let closure = Closure { func: f.clone(), bound, name, file: self.file.clone() };
        Ok(Value::Object(Object::new(self.classes["closure"].clone(), ObjectKind::Closure(closure))))
    }

    /// `$f(...)`: closures, and strings naming a function
    fn call_value_expr(&mut self, f: Value, args: &[Arg], line: u32) -> EResult<Value> {
        match &f {
            Value::Object(o) => match &o.kind {
                ObjectKind::Closure(c) => {
                    let bound = self.bind_args(&c.func, args)?;
                    self.invoke_closure(c, bound, line)
                }
                _ => Err(Fatal::error(format!("Object of type {} is not callable", o.class.name))),
            },
            Value::Str(name) => {
                let name = String::from_utf8_lossy(name).into_owned();
                self.call(name.trim_start_matches('\\'), args, line)
            }
            _ => Err(Fatal::error("Value not callable")),
        }
    }

    fn instantiate(&mut self, class: &str, args: &[Arg], line: u32) -> EResult<Value> {
        let Some(ce) = self.classes.get(&class.to_ascii_lowercase()).cloned() else {
            return Err(Fatal::error(format!("Class \"{class}\" not found")));
        };
        if ce.is_interface {
            return Err(Fatal::error(format!("Cannot instantiate interface {}", ce.name)));
        }
        if !ce.instantiable {
            return Err(Fatal::error(format!("Instantiation of class {} is not allowed", ce.name)));
        }
        // Every instantiable built-in class is a Throwable so far
        let ctor = format!("{}::__construct", ce.throwable_base());
        let vals = self.eval_args(args)?;
        if vals.len() > 3 {
            return Err(Fatal::new(
                "ArgumentCountError",
                format!("{ctor}() expects at most 3 arguments, {} given", vals.len()),
            ));
        }
        let mut vals = vals.into_iter();
        let message = match vals.next() {
            Some(v) => builtins::arg_str(&ctor, 1, "message", &v)?,
            None => Vec::new(),
        };
        let code = match vals.next() {
            Some(v) => builtins::arg_int(&ctor, 2, "code", &v)?,
            None => 0,
        };
        let previous = match vals.next() {
            None | Some(Value::Null) => None,
            Some(Value::Object(o)) if o.throwable().is_some() => Some(o),
            Some(other) => {
                return Err(Fatal::type_error(format!(
                    "{ctor}(): Argument #3 ($previous) must be of type ?Throwable, {} given",
                    other.type_name()
                )))
            }
        };
        // Like php-src, the object remembers where `new` ran, not where it is thrown
        let data = ThrowableData { message, code, file: self.file.clone(), line, trace: self.trace(), previous };
        Ok(Value::Object(Object::new(ce, ObjectKind::Throwable(RefCell::new(data)))))
    }

    fn method_call(&mut self, o: Value, name: &str, args: &[Arg], line: u32) -> EResult<Value> {
        let Value::Object(obj) = &o else {
            return Err(Fatal::error(format!("Call to a member function {name}() on {}", o.type_name())));
        };
        let lname = name.to_ascii_lowercase();
        let undefined = || Fatal::error(format!("Call to undefined method {}::{name}()", obj.class.name));
        match &obj.kind {
            ObjectKind::Closure(c) => match lname.as_str() {
                "__invoke" => {
                    let bound = self.bind_args(&c.func, args)?;
                    self.invoke_closure(c, bound, line)
                }
                _ => Err(undefined()),
            },
            ObjectKind::Throwable(t) => {
                let given = self.eval_args(args)?.len();
                let base = obj.class.throwable_base();
                let known = [
                    "getmessage", "getcode", "getprevious", "getfile", "getline", "gettrace", "gettraceasstring",
                    "__tostring",
                ];
                if !known.contains(&lname.as_str()) {
                    return Err(undefined());
                }
                if given > 0 {
                    return Err(Fatal::new(
                        "ArgumentCountError",
                        format!("{base}::{name}() expects exactly 0 arguments, {given} given"),
                    ));
                }
                if lname == "__tostring" {
                    return Ok(Value::str(object::to_string(obj).unwrap_or_default()));
                }
                let t = t.try_borrow().map_err(|_| recursive_reference())?;
                Ok(match lname.as_str() {
                    "getmessage" => Value::str(t.message.clone()),
                    "getcode" => Value::Int(t.code),
                    "getprevious" => t.previous.clone().map_or(Value::Null, Value::Object),
                    "getfile" => Value::str(t.file.clone()),
                    "getline" => Value::Int(t.line as i64),
                    "gettrace" => trace_array(&t.trace),
                    _ => Value::str(object::trace_string(&t.trace)),
                })
            }
        }
    }

    /// The object for a caught or uncaught error: thrown objects as they are,
    /// internal errors materialized with the place they happened
    fn exception_object(&mut self, e: Fatal) -> Rc<Object> {
        if let Some(obj) = e.object {
            return obj;
        }
        let class = self.classes.get(&e.class.to_ascii_lowercase()).cloned().unwrap_or_else(|| self.classes["error"].clone());
        let data = ThrowableData {
            message: e.msg.into_bytes(),
            code: 0,
            file: self.file.clone(),
            line: e.line.unwrap_or_else(diag::line),
            trace: e.trace.unwrap_or_else(|| self.trace()),
            previous: None,
        };
        Object::new(class, ObjectKind::Throwable(RefCell::new(data)))
    }

    /// The function a callable value names, if any
    fn callable_name(&self, v: &Value) -> Option<Callable> {
        if let Value::Object(o) = v {
            return matches!(o.kind, ObjectKind::Closure(_)).then(|| Callable::Closure(o.clone()));
        }
        let Value::Str(s) = v else { return None };
        let name = String::from_utf8_lossy(s).trim_start_matches('\\').to_ascii_lowercase();
        if let Some(f) = self.functions.get(&name) {
            return Some(Callable::User(f.clone()));
        }
        builtins::exists(&name).then_some(Callable::Builtin(name))
    }

    fn callback(&self, fname: &str, n: usize, v: &Value) -> EResult<Callable> {
        self.callable_name(v).ok_or_else(|| {
            let detail = match v {
                Value::Str(s) => format!("function \"{}\" not found or invalid function name", String::from_utf8_lossy(s)),
                _ => "no array or string given".to_string(),
            };
            Fatal::type_error(format!("{fname}(): Argument #{n} ($callback) must be a valid callback, {detail}"))
        })
    }

    fn call_value(&mut self, f: &Callable, args: Vec<Value>, line: u32) -> EResult<Value> {
        let v = match f {
            Callable::User(f) => self.invoke(f, args.into_iter().map(ArgVal::Val).collect(), line)?,
            Callable::Closure(o) => {
                let ObjectKind::Closure(c) = &o.kind else { unreachable!("checked in callable_name") };
                self.invoke_closure(c, args.into_iter().map(ArgVal::Val).collect(), line)?
            }
            Callable::Builtin(name) if builtins::BY_REF.contains(&name.as_str()) => {
                // A callback cannot pass a variable by reference, so this works on a copy
                let mut args = args.into_iter();
                let mut target = args.next().unwrap_or(Value::Null);
                diag::warning(format!("{name}(): Argument #1 ($array) must be passed by reference, value given"));
                builtins::call_by_ref(name, &mut target, args.collect())?
            }
            Callable::Builtin(name) => self.call_builtin(name, name, args, line)?,
        };
        diag::set_line(line);
        Ok(v)
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
                current.type_name()
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
                    a.type_name()
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
                    other.type_name()
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
}

enum Callable {
    User(Rc<FuncDecl>),
    Closure(Rc<Object>),
    Builtin(String),
}

fn spread(v: Value) -> EResult<Vec<Value>> {
    match v {
        Value::Array(arr) => {
            if arr.keys().any(|k| matches!(k, Key::Str(_))) {
                return Err(Fatal::error("Named arguments are not supported yet"));
            }
            Ok(arr.values().map(Value::copy_out).collect())
        }
        other => Err(Fatal::type_error(format!(
            "Only arrays and Traversables can be unpacked, {} given",
            other.type_name()
        ))),
    }
}

fn list(values: Vec<Value>) -> Value {
    Value::Array(Rc::new(PhpArray::from_values(values)))
}

/// getTrace(): one array per frame
pub fn trace_array(frames: &[TraceFrame]) -> Value {
    list(
        frames
            .iter()
            .map(|f| {
                let mut a = PhpArray::new();
                a.insert(Key::from_bytes(b"file"), Value::str(f.file.clone()));
                a.insert(Key::from_bytes(b"line"), Value::Int(f.line as i64));
                a.insert(Key::from_bytes(b"function"), Value::str(f.function.clone()));
                a.insert(Key::from_bytes(b"args"), list(f.args.clone()));
                Value::Array(Rc::new(a))
            })
            .collect(),
    )
}

/// `$obj->name` for reading. Only built-in objects exist so far, whose
/// properties are all protected or private
fn read_prop(o: &Value, name: &str, nullsafe: bool) -> EResult<Value> {
    match o {
        Value::Object(obj) => match &obj.kind {
            ObjectKind::Closure(_) => Err(Fatal::error("Closure object cannot have properties")),
            ObjectKind::Throwable(_) => {
                let class = obj.class.name;
                match name {
                    "message" | "code" | "file" | "line" => {
                        Err(Fatal::error(format!("Cannot access protected property {class}::${name}")))
                    }
                    "string" | "trace" | "previous" => {
                        Err(Fatal::error(format!("Cannot access private property {class}::${name}")))
                    }
                    _ => {
                        diag::warning(format!("Undefined property: {class}::${name}"));
                        Ok(Value::Null)
                    }
                }
            }
        },
        Value::Null if nullsafe => Ok(Value::Null),
        other => {
            diag::warning(format!("Attempt to read property \"{name}\" on {}", other.type_name()));
            Ok(Value::Null)
        }
    }
}

fn foreach_type_warning(v: &Value) {
    diag::warning(format!("foreach() argument must be of type array|object, {} given", v.type_name()));
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
            _ => Ok(()),
        },
    }
}

/// Non-strict-mode coercion of a value to a declared parameter or return type.
/// Err hands the value back for the error message
/// Whether a string names a user or built-in function (closures are checked by kind)
fn names_callable(functions: &HashMap<String, Rc<FuncDecl>>, v: &Value) -> bool {
    match v {
        Value::Str(s) => {
            let name = String::from_utf8_lossy(s).trim_start_matches('\\').to_ascii_lowercase();
            functions.contains_key(&name) || builtins::exists(&name)
        }
        Value::Object(o) => matches!(o.kind, ObjectKind::Closure(_)),
        _ => false,
    }
}

fn coerce(v: Value, ty: &TypeHint, is_callable: &dyn Fn(&Value) -> bool) -> Result<Value, Value> {
    if let Value::Object(o) = &v {
        let ok = ty.allows("object")
            || ty.types.iter().any(|t| o.class.is_subclass_of(t))
            || ty.allows("callable") && is_callable(&v);
        if ok {
            return Ok(v);
        }
        // Stringable objects pass for string in non-strict mode
        if ty.allows("string")
            && let Some(s) = object::to_string(o) {
                return Ok(Value::str(s));
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
        Value::Object(_) | Value::Ref(_) => unreachable!("handled above or dereferenced before type checks"),
    };
    let exact = ty.allows(actual)
        || matches!(v, Value::Bool(_)) && ty.allows("bool")
        || matches!(v, Value::Array(_)) && ty.allows("iterable")
        || ty.allows("callable") && is_callable(&v);
    if exact {
        return Ok(v);
    }
    // int -> float is always allowed
    if let Value::Int(i) = v
        && ty.allows("float") {
            return Ok(Value::Float(i as f64));
        }
    if matches!(v, Value::Null | Value::Array(_)) {
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

/// String conversion as done by echo, `.` and interpolation: arrays warn, and
/// objects without a string form are an Error
pub fn to_str(v: &Value) -> EResult<Vec<u8>> {
    match v {
        Value::Array(_) => diag::warning("Array to string conversion"),
        Value::Object(o) if o.throwable().is_none() => {
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
        Value::Array(_) => Err(Fatal::type_error("Cannot access offset of type array on string")),
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
        Value::Array(_) | Value::Object(_) => return Err(Fatal::type_error(format!("Cannot increment {}", v.type_name()))),
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
        Value::Array(_) | Value::Object(_) => return Err(Fatal::type_error(format!("Cannot decrement {}", v.type_name()))),
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
        _ => return None,
    })
}
