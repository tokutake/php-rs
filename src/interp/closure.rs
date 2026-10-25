//! Closures made from callables: the first-class callable syntax (`strlen(...)`,
//! `$obj->method(...)`, `A::method(...)`), Closure::fromCallable(), and calls
//! through such closures.

use super::*;
use crate::ast::{ClassRef, Param};
use crate::object::Forward;

impl<W: Write> Interp<W> {
    /// `f(...)` and the like: a closure of what the call would run. Nothing is
    /// called, but the callee must exist, as for a call
    pub(super) fn callable_from(&mut self, call: &Expr) -> EResult<Value> {
        diag::set_line(call.line);
        match &call.kind {
            ExprKind::Call(name, _) => self.function_closure(name.trim_start_matches('\\')),
            ExprKind::NsCall(name, global, _) => {
                let lglobal = global.to_ascii_lowercase();
                let use_global = !self.functions.contains_key(&name.to_ascii_lowercase())
                    && (self.functions.contains_key(&lglobal) || builtins::exists(&lglobal));
                self.function_closure(if use_global { global } else { name })
            }
            ExprKind::MethodCall { object, name, .. } => {
                let o = self.expr(object)?;
                let name = self.member_name(name)?;
                match o {
                    Value::Object(obj) => self.method_closure(&obj, &name),
                    other => Err(Fatal::error(format!("Call to a member function {name}() on {}", other.type_name()))),
                }
            }
            ExprKind::StaticCall { class, name, .. } => self.static_method_closure(class, name),
            ExprKind::CallValue(callee, _) => {
                let v = self.expr(callee)?;
                self.value_closure(v)
            }
            _ => Err(Fatal::error("Cannot create a closure of this expression")),
        }
    }

    /// `$f(...)`: what calling the value would run
    fn value_closure(&mut self, v: Value) -> EResult<Value> {
        match &v {
            Value::Object(o) if o.closure().is_some() => Ok(v),
            Value::Object(o) if o.class.find_method("__invoke").is_none() => {
                Err(Fatal::error(format!("Object of type {} is not callable", o.class.name)))
            }
            Value::Object(o) => self.method_closure(&o.clone(), "__invoke"),
            Value::Str(s) => {
                let name = String::from_utf8_lossy(s).into_owned();
                match name.split_once("::") {
                    Some((class, method)) => self.static_method_closure(&ClassRef::Named(class.to_string()), method),
                    None => self.function_closure(name.trim_start_matches('\\')),
                }
            }
            Value::Array(_) => match self.callable(&v)? {
                Some(c) => Ok(self.closure_of(c)),
                None => Err(Fatal::error("Array callback must have exactly two elements")),
            },
            _ => Err(Fatal::error(format!("Value of type {} is not callable", v.type_name()))),
        }
    }

    fn function_closure(&mut self, name: &str) -> EResult<Value> {
        let lname = name.to_ascii_lowercase();
        if let Some(f) = self.functions.get(&lname).cloned() {
            return Ok(self.closure_of(Callable::User(f)));
        }
        if builtins::exists(&lname) {
            return Ok(self.closure_of(Callable::Builtin(lname)));
        }
        Err(Fatal::error(format!("Call to undefined function {name}()")))
    }

    /// `$obj->name(...)`, resolved like the call `$obj->name()`
    fn method_closure(&mut self, obj: &Rc<Object>, name: &str) -> EResult<Value> {
        let found = obj.class.find_method(name).cloned();
        if let Some(m) = &found
            && self.can_access(m.vis, m.class)
        {
            let this = (!m.is_static).then(|| obj.clone());
            return Ok(self.method_forward(m.clone(), obj.class.clone(), this));
        }
        if obj.class.find_method("__call").is_some() {
            return Ok(self.magic_forward(name, obj.class.clone(), Some(obj.clone())));
        }
        Err(self.method_not_callable(found.as_deref(), obj.class.name, name))
    }

    /// `A::name(...)`, resolved like the call `A::name()`
    fn static_method_closure(&mut self, class_ref: &ClassRef, name: &str) -> EResult<Value> {
        let class = self.resolve_class(class_ref)?;
        let forwarding = matches!(class_ref, ClassRef::SelfRef | ClassRef::Parent | ClassRef::Static);
        let this = self.ctx().this.clone().filter(|t| t.class.is_subclass_of(class.name));
        let found = class.find_method(name).cloned();
        if let Some(m) = &found
            && self.can_access(m.vis, m.class)
        {
            if m.is_static {
                let static_class =
                    if forwarding { self.ctx().static_class.clone().unwrap_or(class.clone()) } else { class.clone() };
                return Ok(self.method_forward(m.clone(), static_class, None));
            }
            let Some(this) = this else {
                return Err(Fatal::error(format!("Non-static method {}::{}() cannot be called statically", m.class, m.name)));
            };
            let static_class = this.class.clone();
            return Ok(self.method_forward(m.clone(), static_class, Some(this)));
        }
        if let Some(this) = this.filter(|t| t.class.find_method("__call").is_some()) {
            return Ok(self.magic_forward(name, this.class.clone(), Some(this)));
        }
        if class.find_method("__callstatic").is_some() {
            return Ok(self.magic_forward(name, class, None));
        }
        Err(self.method_not_callable(found.as_deref(), class.name, name))
    }

    fn method_not_callable(&self, found: Option<&Method>, class: &str, name: &str) -> Fatal {
        match found {
            Some(m) => Fatal::error(format!("Call to {} method {}::{}() from {}", m.vis.name(), m.class, m.name, self.scope_desc())),
            None => Fatal::error(format!("Call to undefined method {class}::{name}()")),
        }
    }

    /// The closure that calls a resolved callable
    pub(super) fn closure_of(&mut self, c: Callable) -> Value {
        match c {
            Callable::Closure(o) => Value::Object(o),
            Callable::User(f) => {
                let name = f.name.clone();
                self.forward_closure(f, name.clone(), None, None, Forward::Function(name))
            }
            Callable::Builtin(lname) => {
                let func = self.native_decl(&lname, &lname);
                self.forward_closure(func, lname.clone(), None, None, Forward::Function(lname))
            }
            Callable::Method { this, method, class } => self.method_forward(method, class, this),
        }
    }

    fn method_forward(&mut self, m: Rc<Method>, static_class: Rc<ClassEntry>, this: Option<Rc<Object>>) -> Value {
        let name = format!("{}::{}", m.class, m.name);
        let func = match &m.body {
            object::MethodBody::User(f) => f.clone(),
            object::MethodBody::Native(_) => self.native_decl(&name, &name.to_ascii_lowercase()),
        };
        let scope = self.classes.get(&m.class.to_ascii_lowercase()).cloned();
        self.forward_closure(func, name, this, scope, Forward::Method(m, static_class))
    }

    fn magic_forward(&mut self, name: &str, class: Rc<ClassEntry>, this: Option<Rc<Object>>) -> Value {
        let shown = format!("{}::{name}", class.name);
        let func = self.native_decl(&shown, "");
        self.forward_closure(func, shown, this, Some(class.clone()), Forward::Magic(name.to_string(), class))
    }

    fn forward_closure(
        &mut self,
        func: Rc<FuncDecl>,
        name: String,
        this: Option<Rc<Object>>,
        scope: Option<Rc<ClassEntry>>,
        forward: Forward,
    ) -> Value {
        // Only user code has a location to show
        let file = if func.body.is_empty() && func.line == 0 { String::new() } else { func.file.path.to_string() };
        let closure = Closure { func, bound: Vec::new(), name, file, this, scope, forward: Some(forward) };
        Value::Object(Object::new(self.classes["closure"].clone(), Internal::Closure(closure)))
    }

    /// A declaration standing in for a built-in function or method, which
    /// describes its parameters (for by-reference arguments and var_dump)
    fn native_decl(&self, name: &str, key: &str) -> Rc<FuncDecl> {
        let params = builtins::signature(key)
            .unwrap_or(&[])
            .iter()
            .map(|p| Param {
                promote: None,
                name: p.name.to_string(),
                ty: None,
                default: p.default.map(|_| Expr::new(ExprKind::Null, 0)),
                by_ref: p.by_ref,
                variadic: p.variadic,
            })
            .collect();
        Rc::new(FuncDecl {
            kind: FuncKind::Named,
            name: name.to_string(),
            uses: Vec::new(),
            params,
            ret: None,
            body: Vec::new(),
            line: 0,
            is_generator: false,
            file: self.file.clone(),
        })
    }

    pub(super) fn closure_from_callable(&mut self, vals: Vec<Value>) -> EResult<Value> {
        let [cb] = <[Value; 1]>::try_from(vals).map_err(|v| {
            Fatal::new("ArgumentCountError", format!("Closure::fromCallable() expects exactly 1 argument, {} given", v.len()))
        })?;
        if let Some(c) = self.callable(&cb)? {
            return Ok(self.closure_of(c));
        }
        let detail = self.not_callable_reason(&cb)?;
        Err(Fatal::type_error(format!("Failed to create closure from callable: {detail}")))
    }

    /// Calls a closure made from a callable, with evaluated arguments
    pub(super) fn call_forward(&mut self, c: &Closure, f: &Forward, args: Vec<ArgVal>, line: u32) -> EResult<Value> {
        let callable = match f {
            Forward::Function(name) => match self.functions.get(&name.to_ascii_lowercase()) {
                Some(decl) => Callable::User(decl.clone()),
                None => Callable::Builtin(name.to_ascii_lowercase()),
            },
            Forward::Method(m, class) => Callable::Method { this: c.this.clone(), method: m.clone(), class: class.clone() },
            Forward::Magic(name, class) => {
                let vals = list(args.iter().map(ArgVal::value).collect());
                return self.call_magic(c, name, class, vals, line);
            }
        };
        self.call_value_args(&callable, args, line)
    }

    /// Calls a closure made from a callable with arguments still to be
    /// evaluated, so that named and by-reference arguments work as in a call
    pub(super) fn call_forward_ast(&mut self, c: &Closure, f: &Forward, args: &[Arg], line: u32) -> EResult<Value> {
        let v = match f {
            Forward::Function(name) => self.call(name, args, line, false)?,
            Forward::Method(m, class) => self.call_method_ast(c.this.clone(), m, args, line, class.clone())?,
            Forward::Magic(name, class) => {
                let vals = self.magic_call_args(args)?;
                self.call_magic(c, name, class, vals, line)?
            }
        };
        diag::set_line(line);
        Ok(v)
    }

    fn call_magic(&mut self, c: &Closure, name: &str, class: &Rc<ClassEntry>, args: Value, line: u32) -> EResult<Value> {
        let magic = if c.this.is_some() { "__call" } else { "__callstatic" };
        let Some(m) = class.find_method(magic).cloned() else {
            return Err(Fatal::error(format!("Call to undefined method {}::{name}()", class.name)));
        };
        let margs = vec![ArgVal::Val(Value::str(name)), ArgVal::Val(args)];
        self.call_method(c.this.clone(), &m, margs, line, class.clone())
    }
}
