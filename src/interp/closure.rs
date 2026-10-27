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
            returns_ref: false,
            is_static: false,
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

impl<W: Write> Interp<W> {
    /// Closure::bind($closure, $newThis, $newScope) and $closure->bindTo($newThis, $newScope)
    pub(super) fn closure_bind(&mut self, this: Option<&Rc<Object>>, vals: Vec<Value>) -> EResult<Value> {
        let (fname, min) = if this.is_some() { ("Closure::bindTo", 1) } else { ("Closure::bind", 2) };
        // Argument positions are counted from the closure for bind()
        let first = min - 1;
        if vals.len() < min || vals.len() > min + 1 {
            let (bound, n) = if vals.len() < min { ("at least", min) } else { ("at most", min + 1) };
            return Err(Fatal::new(
                "ArgumentCountError",
                format!("{fname}() expects {bound} {n} argument{}, {} given", if n == 1 { "" } else { "s" }, vals.len()),
            ));
        }
        let mut vals = vals.into_iter();
        let closure = match this {
            Some(t) => t.clone(),
            None => match vals.next() {
                Some(Value::Object(o)) if o.closure().is_some() => o,
                other => {
                    let given = other.unwrap_or(Value::Null).value_name();
                    return Err(Fatal::type_error(format!("{fname}(): Argument #1 ($closure) must be of type Closure, {given} given")));
                }
            },
        };
        let new_this = match vals.next().unwrap_or(Value::Null) {
            Value::Null => None,
            Value::Object(o) => Some(o),
            other => {
                return Err(Fatal::type_error(format!(
                    "{fname}(): Argument #{} ($newThis) must be of type ?object, {} given",
                    first + 1,
                    other.value_name()
                )))
            }
        };
        let c = closure.closure().expect("checked above");
        let scope = match vals.next() {
            None => c.scope.clone(),
            Some(Value::Null) => None,
            Some(Value::Object(o)) => Some(o.class.clone()),
            Some(v @ (Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_))) => {
                let name = String::from_utf8_lossy(&to_str(&v)?).into_owned();
                if name == "static" {
                    c.scope.clone()
                } else {
                    match self.find_class(&name)? {
                        Some(class) => Some(class),
                        None => {
                            diag::warning(format!("Class \"{name}\" not found"));
                            return Ok(Value::Null);
                        }
                    }
                }
            }
            Some(other) => {
                return Err(Fatal::type_error(format!(
                    "{fname}(): Argument #{} ($newScope) must be of type object|string|null, {} given",
                    first + 2,
                    other.value_name()
                )))
            }
        };
        if !self.valid_binding(c, new_this.as_ref(), scope.as_ref()) {
            return Ok(Value::Null);
        }
        // An object bound without a scope gets the scope of Closure itself, as in php-src
        let scope = match (scope, &new_this) {
            (None, Some(_)) => Some(self.classes["closure"].clone()),
            (scope, _) => scope,
        };
        let copy = rebound(c, new_this, scope);
        Ok(Value::Object(Object::new(self.classes["closure"].clone(), Internal::Closure(copy))))
    }

    /// $closure->call($newThis, ...$args): calls it bound to the object, in its class's scope
    pub(super) fn closure_call(&mut self, closure: &Rc<Object>, vals: Vec<Value>, line: u32) -> EResult<Value> {
        let mut vals = vals.into_iter();
        let new_this = match vals.next() {
            Some(Value::Object(o)) => o,
            Some(other) => {
                return Err(Fatal::type_error(format!(
                    "Closure::call(): Argument #1 ($newThis) must be of type object, {} given",
                    other.value_name()
                )))
            }
            None => return Err(Fatal::new("ArgumentCountError", "Closure::call() expects at least 1 argument, 0 given")),
        };
        let c = closure.closure().ok_or_else(|| Fatal::error("Closure::call() called on a non-closure"))?;
        let scope = Some(new_this.class.clone());
        if !self.valid_binding(c, Some(&new_this), scope.as_ref()) {
            return Ok(Value::Null);
        }
        let copy = rebound(c, Some(new_this), scope);
        self.invoke_closure(&copy, vals.map(ArgVal::Val).collect(), line)
    }

    /// Whether a closure may get this `$this` and scope, warning as php-src does if not
    fn valid_binding(&self, c: &Closure, new_this: Option<&Rc<Object>>, scope: Option<&Rc<ClassEntry>>) -> bool {
        let fake = c.forward.is_some();
        let is_static = match &c.forward {
            Some(Forward::Method(m, _)) => m.is_static,
            Some(Forward::Magic(..)) => c.this.is_none(),
            Some(Forward::Function(_)) => false,
            None => c.func.is_static,
        };
        let func_scope = c.scope.as_ref();
        let same_scope = match (scope, func_scope) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        };
        let refuse = |msg: String| {
            diag::warning(msg);
            false
        };
        if let Some(t) = new_this {
            if is_static {
                return refuse("Cannot bind an instance to a static closure".into());
            }
            if fake && let Some(s) = func_scope
                && !t.class.is_subclass_of(s.name)
            {
                return refuse(format!("Cannot bind method {}() to object of class {}", c.name, t.class.name));
            }
        } else if fake && func_scope.is_some() && !is_static {
            return refuse("Cannot unbind $this of method".into());
        } else if !fake && c.this.is_some() && crate::genlower::uses_this(&c.func.body) {
            return refuse("Cannot unbind $this of closure using $this".into());
        }
        if let Some(s) = scope
            && !same_scope
            && s.is_internal
        {
            return refuse(format!("Cannot bind closure to scope of internal class {}", s.name));
        }
        if fake && !same_scope {
            let from = if func_scope.is_none() { "function" } else { "method" };
            return refuse(format!("Cannot rebind scope of closure created from {from}"));
        }
        true
    }
}

/// A copy of a closure with another `$this` and scope
fn rebound(c: &Closure, this: Option<Rc<Object>>, scope: Option<Rc<ClassEntry>>) -> Closure {
    // A method closure bound to an object of a subclass calls it with that class as `static`
    let forward = match (&c.forward, &this) {
        (Some(Forward::Method(m, _)), Some(t)) => Some(Forward::Method(m.clone(), t.class.clone())),
        (f, _) => f.clone(),
    };
    Closure { func: c.func.clone(), bound: c.bound.clone(), name: c.name.clone(), file: c.file.clone(), this, scope, forward }
}
