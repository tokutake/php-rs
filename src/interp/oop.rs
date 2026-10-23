//! Classes and objects: declarations, property access with visibility and type
//! checks, method dispatch with `self`/`parent`/`static`, constants and magic methods.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use super::{to_key, ArgVal, Base, CallCtx, Callable, EResult, Fatal, Interp, Path, RefCellValue, TypeCtx};
use crate::array::Key;
use crate::ast::{Arg, ClassDecl, ClassRef, Expr, FuncDecl, Member, TypeHint};
use crate::builtins;
use crate::diag;
use crate::object::{
    self, mangle, unmangle, ClassConst, ClassEntry, ConstState, Internal, Method, MethodBody, Native, Object, PropInfo,
    StaticProp, Visibility,
};
use crate::value::{self, Value};

/// How a property name resolves on an object, seen from the current scope
enum PropLookup {
    Declared(PropInfo),
    /// Declared, but not visible from here
    Inaccessible(PropInfo),
    /// Not declared: a dynamic property (which may or may not exist)
    Dynamic,
}

/// Which magic accessor is running, for the recursion guard
const GET: u8 = 0;
const SET: u8 = 1;
const ISSET: u8 = 2;
const UNSET: u8 = 3;

impl<W: Write> Interp<W> {
    // ---- Declarations ----

    pub(super) fn declare_class(&mut self, decl: &Rc<ClassDecl>) -> EResult<()> {
        diag::set_line(decl.line);
        let kind = if decl.is_interface { "interface" } else { "class" };
        if self.classes.contains_key(&decl.name.to_ascii_lowercase()) {
            return Err(Fatal::compile(format!(
                "Cannot declare {kind} {}, because the name is already in use",
                decl.name
            )));
        }
        // Class names live as long as the program, like the class table in php-src
        let name: &'static str = Box::leak(decl.name.clone().into_boxed_str());

        let parent = match &decl.parent {
            Some(p) => {
                let pe = self.classes.get(&p.to_ascii_lowercase()).cloned().ok_or_else(|| {
                    Fatal::error(format!("Class \"{p}\" not found"))
                })?;
                if pe.is_interface {
                    return Err(Fatal::compile(format!("Class {name} cannot extend interface {}", pe.name)));
                }
                if pe.is_final {
                    return Err(Fatal::compile(format!("Class {name} cannot extend final class {}", pe.name)));
                }
                Some(pe)
            }
            None => None,
        };
        let mut entry = match &parent {
            Some(p) => object::plain_subclass(name, p),
            None => ClassEntry { name, ..Default::default() },
        };
        // Private constants stay with the class that declares them
        entry.consts.retain(|_, c| c.vis != Visibility::Private);
        entry.is_interface = decl.is_interface;
        entry.is_abstract = decl.is_abstract;
        entry.is_final = decl.is_final;
        entry.is_readonly = decl.is_readonly;
        // A readonly class cannot get dynamic properties at all
        entry.allow_dynamic &= !decl.is_readonly;

        for iname in &decl.interfaces {
            let ie = self.classes.get(&iname.to_ascii_lowercase()).cloned().ok_or_else(|| {
                Fatal::error(format!("Interface \"{iname}\" not found"))
            })?;
            if !ie.is_interface {
                return Err(Fatal::compile(format!("{name} cannot implement {} - it is not an interface", ie.name)));
            }
            if ie.name.eq_ignore_ascii_case("Throwable") && !entry.is_throwable && !decl.is_interface {
                return Err(Fatal::compile(format!(
                    "Class {name} cannot implement interface Throwable, extend Exception or Error instead"
                )));
            }
            for i in std::iter::once(ie.name).chain(ie.interfaces.iter().copied()) {
                if !entry.interfaces.contains(&i) {
                    entry.interfaces.push(i);
                }
            }
            for (k, c) in &ie.consts {
                entry.consts.entry(k.clone()).or_insert_with(|| c.clone());
            }
            for (k, m) in &ie.methods {
                entry.methods.entry(k.clone()).or_insert_with(|| m.clone());
            }
        }

        let mut prop_defaults: Vec<(Key, Expr)> = Vec::new();
        let mut static_defaults: Vec<(String, Expr)> = Vec::new();
        for member in &decl.members {
            match member {
                Member::Const { name: cname, value, vis } => {
                    let c = ClassConst { vis: *vis, class: name, state: RefCell::new(ConstState::Unevaluated(value.clone())) };
                    entry.consts.insert(cname.clone(), Rc::new(c));
                }
                Member::Prop { name: pname, ty, default, vis, is_static: true, .. } => {
                    let cell = Rc::new(RefCell::new(Value::Null));
                    entry.static_props.insert(pname.clone(), StaticProp { vis: *vis, class: name, ty: ty.clone(), cell });
                    if let Some(d) = default {
                        static_defaults.push((pname.clone(), d.clone()));
                    }
                }
                Member::Prop { name: pname, ty, default, vis, is_static: false, readonly } => {
                    let readonly = *readonly || decl.is_readonly;
                    if readonly && ty.is_none() {
                        return Err(Fatal::compile(format!("Readonly property {name}::${pname} must have type")));
                    }
                    if readonly && default.is_some() {
                        return Err(Fatal::compile(format!(
                            "Readonly property {name}::${pname} cannot have default value"
                        )));
                    }
                    let info = PropInfo {
                        name: pname.clone(),
                        key: mangle(pname, *vis, name),
                        vis: *vis,
                        class: name,
                        ty: ty.clone(),
                        // Placeholder until the default expression is evaluated below
                        default: default.as_ref().map(|_| Value::Null),
                        readonly,
                    };
                    if let Some(d) = default {
                        prop_defaults.push((info.key.clone(), d.clone()));
                    }
                    add_prop(&mut entry, info)?;
                }
                Member::Method { func, vis, is_static, is_abstract, is_final } => {
                    if decl.is_interface && *vis != Visibility::Public {
                        return Err(Fatal::compile(format!(
                            "Access type for interface method {name}::{}() must be public",
                            func.name
                        )));
                    }
                    if *is_abstract && !decl.is_abstract && !decl.is_interface {
                        return Err(Fatal::compile(format!(
                            "Class {name} contains abstract method {} and must therefore be declared abstract",
                            func.name
                        )));
                    }
                    if func.name.eq_ignore_ascii_case("__construct") {
                        for p in func.params.iter().filter(|p| p.promote.is_some()) {
                            let (vis, readonly) = p.promote.unwrap();
                            let info = PropInfo {
                                name: p.name.clone(),
                                key: mangle(&p.name, vis, name),
                                vis,
                                class: name,
                                ty: p.ty.clone(),
                                default: None,
                                readonly: readonly || decl.is_readonly,
                            };
                            add_prop(&mut entry, info)?;
                        }
                    }
                    let method = Method {
                        name: func.name.clone(),
                        class: name,
                        vis: *vis,
                        is_static: *is_static,
                        is_abstract: *is_abstract,
                        is_final: *is_final,
                        body: MethodBody::User(func.clone()),
                    };
                    add_method(&mut entry, method)?;
                }
            }
        }

        if !decl.is_abstract && !decl.is_interface {
            let mut missing: Vec<String> =
                entry.methods.values().filter(|m| m.is_abstract).map(|m| format!("{}::{}", m.class, m.name)).collect();
            if !missing.is_empty() {
                missing.sort();
                let n = missing.len();
                let mut shown = missing.into_iter().take(3).collect::<Vec<_>>().join(", ");
                if n > 3 {
                    shown.push_str(", ...");
                }
                let s = if n == 1 { "" } else { "s" };
                return Err(Fatal::compile(format!(
                    "Class {name} contains {n} abstract method{s} and must therefore be declared abstract or implement the remaining method{s} ({shown})"
                )));
            }
        }

        // Defaults may refer to the class's own constants (`self::X`), so the class
        // is registered first and the defaults evaluated in its scope
        let lname = decl.name.to_ascii_lowercase();
        let provisional = Rc::new(entry.clone());
        self.classes.insert(lname.clone(), provisional.clone());
        self.declared.insert(Rc::as_ptr(decl));
        self.with_scope(provisional, |this| {
            for (key, expr) in &prop_defaults {
                let v = this.expr(expr)?;
                if let Some(p) = entry.props.iter_mut().find(|p| &p.key == key) {
                    p.default = Some(v);
                }
            }
            for (pname, expr) in &static_defaults {
                let v = this.expr(expr)?;
                *entry.static_props[pname].cell.borrow_mut() = v;
            }
            Ok(())
        })?;
        self.classes.insert(lname, Rc::new(entry));
        Ok(())
    }

    /// Runs `f` with `self` and `static` set to `class`, as while evaluating its constants
    fn with_scope<T>(&mut self, class: Rc<ClassEntry>, f: impl FnOnce(&mut Self) -> EResult<T>) -> EResult<T> {
        let ctx = &mut self.frame().ctx;
        let saved = (ctx.scope.replace(class.clone()), ctx.static_class.replace(class));
        let out = f(self);
        let ctx = &mut self.frame().ctx;
        (ctx.scope, ctx.static_class) = saved;
        out
    }

    // ---- Class resolution ----

    pub(super) fn resolve_class(&mut self, class: &ClassRef) -> EResult<Rc<ClassEntry>> {
        match class {
            ClassRef::Named(n) => self
                .classes
                .get(&n.to_ascii_lowercase())
                .cloned()
                .ok_or_else(|| Fatal::error(format!("Class \"{n}\" not found"))),
            ClassRef::SelfRef => self.ctx().scope.clone().ok_or_else(|| self.no_scope("self")),
            ClassRef::Static => self.ctx().static_class.clone().ok_or_else(|| self.no_scope("static")),
            ClassRef::Parent => {
                let scope = self.ctx().scope.clone().ok_or_else(|| self.no_scope("parent"))?;
                scope.parent.clone().ok_or_else(|| Fatal::error("Cannot use \"parent\" when current class scope has no parent"))
            }
            ClassRef::Dynamic(e) => match self.expr(e)? {
                Value::Object(o) => Ok(o.class.clone()),
                Value::Str(s) => {
                    let n = String::from_utf8_lossy(&s).trim_start_matches('\\').to_string();
                    self.resolve_class(&ClassRef::Named(n))
                }
                other => Err(Fatal::error(format!("Cannot use value of type {} as class name", other.type_name()))),
            },
        }
    }

    /// php-src rejects these at compile time; the message depends on whether the
    /// code sits at the top level (closures there included) or in a function
    fn no_scope(&self, what: &str) -> Fatal {
        let top_level = self.frames.len() == 1 || self.ctx().name.starts_with("{closure");
        if top_level {
            Fatal::error(format!("Cannot use \"{what}\" in the global scope"))
        } else {
            Fatal::error(format!("Cannot use \"{what}\" when no class scope is active"))
        }
    }

    /// Like resolve_class, but an unknown class is None instead of an error
    pub(super) fn resolve_class_quiet(&mut self, class: &ClassRef) -> EResult<Option<Rc<ClassEntry>>> {
        match class {
            ClassRef::Named(n) => Ok(self.classes.get(&n.to_ascii_lowercase()).cloned()),
            ClassRef::Dynamic(e) => match self.expr(e)? {
                Value::Object(o) => Ok(Some(o.class.clone())),
                Value::Str(s) => {
                    let n = String::from_utf8_lossy(&s).trim_start_matches('\\').to_ascii_lowercase();
                    Ok(self.classes.get(&n).cloned())
                }
                _ => Ok(None),
            },
            other => self.resolve_class(other).map(Some),
        }
    }

    /// `instanceof` / catch matching, where `self`, `static` and `parent` follow the scope
    pub(super) fn is_instance_of(&self, class: &ClassEntry, name: &str) -> bool {
        let ctx = self.ctx();
        let target = match name.to_ascii_lowercase().as_str() {
            "self" => ctx.scope.as_ref().map(|c| c.name),
            "static" => ctx.static_class.as_ref().map(|c| c.name),
            "parent" => ctx.scope.as_ref().and_then(|c| c.parent.as_ref()).map(|c| c.name),
            _ => return class.is_subclass_of(name),
        };
        target.is_some_and(|t| class.is_subclass_of(t))
    }

    // ---- Visibility ----

    /// Whether a member declared in `class` with `vis` is visible from the current scope
    fn can_access(&self, vis: Visibility, class: &str) -> bool {
        let scope = self.ctx().scope.as_deref();
        match vis {
            Visibility::Public => true,
            Visibility::Private => scope.is_some_and(|s| s.name.eq_ignore_ascii_case(class)),
            Visibility::Protected => scope.is_some_and(|s| {
                s.is_subclass_of(class)
                    || self.classes.get(&class.to_ascii_lowercase()).is_some_and(|c| c.is_subclass_of(s.name))
            }),
        }
    }

    fn scope_desc(&self) -> String {
        match &self.ctx().scope {
            Some(s) => format!("scope {}", s.name),
            None => "global scope".to_string(),
        }
    }

    // ---- Objects ----

    pub(super) fn instantiate(&mut self, class: &Rc<ClassEntry>, args: &[Arg], line: u32) -> EResult<Value> {
        if class.is_interface {
            return Err(Fatal::error(format!("Cannot instantiate interface {}", class.name)));
        }
        if class.is_abstract {
            return Err(Fatal::error(format!("Cannot instantiate abstract class {}", class.name)));
        }
        if class.internal_only {
            return Err(Fatal::error(format!("Instantiation of class {} is not allowed", class.name)));
        }
        let obj = Object::new(class.clone(), Internal::None);
        if class.is_throwable {
            // Like php-src, a Throwable remembers where `new` ran, not where it is thrown
            obj.set("\0*\0file", Value::str(self.file.clone()));
            obj.set("\0*\0line", Value::Int(line as i64));
            obj.set(&obj.throwable_key("trace"), object::trace_array(&self.trace()));
        }
        // Without a constructor the arguments are not even evaluated
        if let Some(ctor) = class.find_method("__construct").cloned() {
            if !self.can_access(ctor.vis, ctor.class) {
                return Err(Fatal::error(format!(
                    "Call to {} {}::__construct() from {}",
                    ctor.vis.name(),
                    class.name,
                    self.scope_desc()
                )));
            }
            self.call_method_ast(Some(obj.clone()), &ctor, args, line, class.clone())?;
        }
        Ok(Value::Object(obj))
    }

    /// The exception object for an error: thrown objects as they are, internal
    /// errors materialized with the place they happened
    pub(super) fn exception_object(&mut self, e: Fatal) -> Rc<Object> {
        if let Some(obj) = e.object {
            return obj;
        }
        let class = self.classes.get(&e.class.to_ascii_lowercase()).cloned().unwrap_or_else(|| self.classes["error"].clone());
        let obj = Object::new(class, Internal::None);
        obj.set("\0*\0message", Value::str(e.msg));
        obj.set("\0*\0file", Value::str(self.file.clone()));
        obj.set("\0*\0line", Value::Int(e.line.unwrap_or_else(diag::line) as i64));
        let trace = e.trace.unwrap_or_else(|| self.trace());
        obj.set(&obj.throwable_key("trace"), object::trace_array(&trace));
        obj
    }

    pub(super) fn clone_value(&mut self, v: Value) -> EResult<Value> {
        let Value::Object(o) = v else {
            return Err(Fatal::error("__clone method called on non-object"));
        };
        if o.class.is_throwable {
            return Err(Fatal::error(format!("Trying to clone an uncloneable object of class {}", o.class.name)));
        }
        let internal = match &o.internal {
            Internal::Closure(c) => Internal::Closure(object::Closure {
                func: c.func.clone(),
                bound: c.bound.clone(),
                name: c.name.clone(),
                file: c.file.clone(),
                this: c.this.clone(),
                scope: c.scope.clone(),
            }),
            Internal::None => Internal::None,
        };
        let copy = Object::new(o.class.clone(), internal);
        let props = o.props.try_borrow().map_err(|_| super::recursive_reference())?.clone();
        *copy.props.borrow_mut() = props;
        if let Some(m) = o.class.find_method("__clone").cloned() {
            if !self.can_access(m.vis, m.class) {
                return Err(Fatal::error(format!(
                    "Call to {} {}::__clone() from {}",
                    m.vis.name(),
                    o.class.name,
                    self.scope_desc()
                )));
            }
            let line = diag::line();
            self.call_method(Some(copy.clone()), &m, Vec::new(), line, copy.class.clone())?;
        }
        Ok(Value::Object(copy))
    }

    /// `(object)`: arrays become stdClass objects
    pub(super) fn cast_object(&mut self, v: Value) -> Value {
        if let Value::Object(_) = v {
            return v;
        }
        let obj = Object::new(self.classes["stdclass"].clone(), Internal::None);
        match &v {
            Value::Null => {}
            Value::Array(arr) => {
                let mut props = obj.props.borrow_mut();
                for (k, val) in arr.iter() {
                    // Integer keys become property names
                    let name = match k {
                        Key::Int(i) => Key::Str(i.to_string().into_bytes().into()),
                        k => k.clone(),
                    };
                    props.insert(name, val.clone());
                }
            }
            other => obj.set("scalar", other.clone()),
        }
        Value::Object(obj)
    }

    // ---- Methods ----

    /// Calls a method with arguments still to be evaluated (so by-reference parameters work)
    fn call_method_ast(
        &mut self,
        this: Option<Rc<Object>>,
        method: &Rc<Method>,
        args: &[Arg],
        line: u32,
        static_class: Rc<ClassEntry>,
    ) -> EResult<Value> {
        let bound = match &method.body {
            MethodBody::User(f) => self.bind_args(f, &format!("{}::{}", method.class, method.name), args)?,
            MethodBody::Native(_) => self.eval_args(args)?.into_iter().map(ArgVal::Val).collect(),
        };
        self.call_method(this, method, bound, line, static_class)
    }

    pub(super) fn call_method(
        &mut self,
        this: Option<Rc<Object>>,
        method: &Rc<Method>,
        args: Vec<ArgVal>,
        line: u32,
        static_class: Rc<ClassEntry>,
    ) -> EResult<Value> {
        let func = match &method.body {
            MethodBody::Native(n) => {
                let vals = args
                    .into_iter()
                    .map(|a| match a {
                        ArgVal::Val(v) => v,
                        ArgVal::Ref(r) => value::with_deref(&r, Value::clone),
                    })
                    .collect();
                return self.native_method(*n, method, this, vals, line);
            }
            MethodBody::User(f) => f.clone(),
        };
        if method.is_abstract {
            return Err(Fatal::error(format!("Cannot call abstract method {}::{}()", method.class, method.name)));
        }
        let this = if method.is_static { None } else { this };
        let ctx = CallCtx {
            name: format!("{}::{}", method.class, method.name),
            trace_fn: method.name.clone(),
            trace_class: Some((method.class.to_string(), if this.is_some() { "->" } else { "::" })),
            scope: self.classes.get(&method.class.to_ascii_lowercase()).cloned(),
            static_class: Some(static_class),
            this,
        };
        self.invoke_with(&func, ctx, Vec::new(), args, line)
    }

    pub(super) fn method_call(&mut self, o: Value, name: &str, args: &[Arg], line: u32) -> EResult<Value> {
        let Value::Object(obj) = &o else {
            return Err(Fatal::error(format!("Call to a member function {name}() on {}", o.type_name())));
        };
        if let Some(c) = obj.closure()
            && name.eq_ignore_ascii_case("__invoke") {
                let bound = self.bind_args(&c.func, &c.name, args)?;
                return self.invoke_closure(c, bound, line);
            }
        let found = obj.class.find_method(name).cloned();
        if let Some(m) = &found
            && self.can_access(m.vis, m.class) {
                return self.call_method_ast(Some(obj.clone()), m, args, line, obj.class.clone());
            }
        if let Some(magic) = obj.class.find_method("__call").cloned() {
            let vals = self.eval_args(args)?;
            let margs = vec![ArgVal::Val(Value::str(name)), ArgVal::Val(super::list(vals))];
            return self.call_method(Some(obj.clone()), &magic, margs, line, obj.class.clone());
        }
        Err(match found {
            Some(m) => Fatal::error(format!(
                "Call to {} method {}::{}() from {}",
                m.vis.name(),
                m.class,
                m.name,
                self.scope_desc()
            )),
            None => Fatal::error(format!("Call to undefined method {}::{name}()", obj.class.name)),
        })
    }

    pub(super) fn static_call(&mut self, class_ref: &ClassRef, name: &str, args: &[Arg], line: u32) -> EResult<Value> {
        let class = self.resolve_class(class_ref)?;
        // self::, parent:: and static:: forward the late static binding class
        let forwarding = matches!(class_ref, ClassRef::SelfRef | ClassRef::Parent | ClassRef::Static);
        let this = self.ctx().this.clone().filter(|t| t.class.is_subclass_of(class.name));
        let found = class.find_method(name).cloned();
        if let Some(m) = &found
            && self.can_access(m.vis, m.class) {
                if m.is_static {
                    let static_class =
                        if forwarding { self.ctx().static_class.clone().unwrap_or(class.clone()) } else { class.clone() };
                    return self.call_method_ast(None, m, args, line, static_class);
                }
                // An instance method called through a class name, like parent::__construct()
                let Some(this) = this else {
                    return Err(Fatal::error(format!("Non-static method {}::{}() cannot be called statically", m.class, m.name)));
                };
                let static_class = this.class.clone();
                return self.call_method_ast(Some(this), m, args, line, static_class);
            }
        if let (Some(this), Some(magic)) = (&this, class.find_method("__call").cloned()) {
            let vals = self.eval_args(args)?;
            let margs = vec![ArgVal::Val(Value::str(name)), ArgVal::Val(super::list(vals))];
            return self.call_method(Some(this.clone()), &magic, margs, line, this.class.clone());
        }
        if let Some(magic) = class.find_method("__callstatic").cloned() {
            let vals = self.eval_args(args)?;
            let margs = vec![ArgVal::Val(Value::str(name)), ArgVal::Val(super::list(vals))];
            return self.call_method(None, &magic, margs, line, class.clone());
        }
        Err(match found {
            Some(m) => Fatal::error(format!(
                "Call to {} method {}::{}() from {}",
                m.vis.name(),
                m.class,
                m.name,
                self.scope_desc()
            )),
            None => Fatal::error(format!("Call to undefined method {}::{name}()", class.name)),
        })
    }

    fn native_method(
        &mut self,
        n: Native,
        method: &Method,
        this: Option<Rc<Object>>,
        vals: Vec<Value>,
        line: u32,
    ) -> EResult<Value> {
        let Some(this) = this else {
            return Err(Fatal::error(format!("Non-static method {}::{}() cannot be called statically", method.class, method.name)));
        };
        let fname = format!("{}::{}", method.class, method.name);
        if n == Native::ThrowableConstruct {
            if vals.len() > 3 {
                return Err(Fatal::new(
                    "ArgumentCountError",
                    format!("{fname}() expects at most 3 arguments, {} given", vals.len()),
                ));
            }
            let mut vals = vals.into_iter();
            if let Some(v) = vals.next() {
                let s = builtins::arg_str(self, &fname, 1, "message", &v)?;
                this.set("\0*\0message", Value::str(s));
            }
            if let Some(v) = vals.next() {
                this.set("\0*\0code", Value::Int(builtins::arg_int(&fname, 2, "code", &v)?));
            }
            match vals.next() {
                None | Some(Value::Null) => {}
                Some(Value::Object(p)) if p.class.is_throwable => this.set(&this.throwable_key("previous"), Value::Object(p)),
                Some(other) => {
                    return Err(Fatal::type_error(format!(
                        "{fname}(): Argument #3 ($previous) must be of type ?Throwable, {} given",
                        other.type_name()
                    )))
                }
            }
            return Ok(Value::Null);
        }
        if n == Native::ClosureInvoke {
            let c = this.closure().ok_or_else(|| Fatal::error("Closure::__invoke() called on a non-closure"))?;
            return self.invoke_closure(c, vals.into_iter().map(ArgVal::Val).collect(), line);
        }
        if !vals.is_empty() {
            return Err(Fatal::new(
                "ArgumentCountError",
                format!("{fname}() expects exactly 0 arguments, {} given", vals.len()),
            ));
        }
        Ok(match n {
            Native::GetMessage => this.get("\0*\0message"),
            Native::GetCode => this.get("\0*\0code"),
            Native::GetFile => this.get("\0*\0file"),
            Native::GetLine => this.get("\0*\0line"),
            Native::GetPrevious => this.get(&this.throwable_key("previous")),
            Native::GetTrace => this.get(&this.throwable_key("trace")),
            Native::GetTraceAsString => {
                let frames = object::trace_frames(&this.get(&this.throwable_key("trace")));
                Value::str(object::trace_string(&frames))
            }
            Native::ThrowableToString => Value::str(object::throwable_to_string(&this)),
            Native::ThrowableConstruct | Native::ClosureInvoke => unreachable!("handled above"),
        })
    }

    /// Writes constructor-promoted parameters into the new object's properties
    pub(super) fn promote_params(&mut self, f: &FuncDecl) -> EResult<()> {
        let (Some(this), Some(scope)) = (self.ctx().this.clone(), self.ctx().scope.clone()) else { return Ok(()) };
        for p in f.params.iter().filter(|p| p.promote.is_some()) {
            let (vis, _) = p.promote.unwrap();
            let v = self.frame().vars.get(&p.name).map(Value::copy_out).unwrap_or(Value::Null);
            this.props.try_borrow_mut().map_err(|_| super::recursive_reference())?.insert(mangle(&p.name, vis, scope.name), v);
        }
        Ok(())
    }

    // ---- Properties ----

    fn find_prop(&self, class: &ClassEntry, name: &str) -> PropLookup {
        // A private property of the calling class wins when the object is an instance of it
        if let Some(scope) = &self.ctx().scope
            && class.is_subclass_of(scope.name) {
                let own = class.props.iter().find(|p| p.name == name && p.vis == Visibility::Private && p.class == scope.name);
                if let Some(p) = own {
                    return PropLookup::Declared(p.clone());
                }
            }
        // Private properties of ancestors are invisible to the class itself
        let visible = class.props.iter().rev().find(|p| p.name == name && (p.vis != Visibility::Private || p.class == class.name));
        match visible {
            None => PropLookup::Dynamic,
            Some(p) if self.can_access(p.vis, p.class) => PropLookup::Declared(p.clone()),
            Some(p) => PropLookup::Inaccessible(p.clone()),
        }
    }

    fn stored(obj: &Object, key: &Key) -> Option<Value> {
        obj.props.try_borrow().ok().and_then(|p| p.get(key).map(Value::copy_out))
    }

    /// Runs a magic accessor unless it is already running for this property
    fn magic(&mut self, obj: &Rc<Object>, which: u8, name: &str, extra: Option<Value>) -> EResult<Option<Value>> {
        let mname = ["__get", "__set", "__isset", "__unset"][which as usize];
        let Some(m) = obj.class.find_method(mname).cloned() else { return Ok(None) };
        let guard = (obj.id, name.to_string(), which);
        if self.magic_guard.contains(&guard) {
            return Ok(None);
        }
        self.magic_guard.insert(guard.clone());
        let mut args = vec![ArgVal::Val(Value::str(name))];
        if let Some(v) = extra {
            args.push(ArgVal::Val(v));
        }
        let line = diag::line();
        let r = self.call_method(Some(obj.clone()), &m, args, line, obj.class.clone());
        self.magic_guard.remove(&guard);
        r.map(Some)
    }

    pub(super) fn read_prop(&mut self, obj: &Rc<Object>, name: &str) -> EResult<Value> {
        let class = obj.class.name;
        match self.find_prop(&obj.class, name) {
            PropLookup::Declared(p) => {
                if let Some(v) = Self::stored(obj, &p.key) {
                    return Ok(v);
                }
                if let Some(v) = self.magic(obj, GET, name, None)? {
                    return Ok(v);
                }
                if p.ty.is_some() {
                    return Err(Fatal::error(format!(
                        "Typed property {}::${name} must not be accessed before initialization",
                        p.class
                    )));
                }
                diag::warning(format!("Undefined property: {class}::${name}"));
                Ok(Value::Null)
            }
            PropLookup::Inaccessible(p) => match self.magic(obj, GET, name, None)? {
                Some(v) => Ok(v),
                None => Err(Fatal::error(format!("Cannot access {} property {class}::${name}", p.vis.name()))),
            },
            PropLookup::Dynamic => {
                if let Some(v) = Self::stored(obj, &Key::Str(name.as_bytes().into())) {
                    return Ok(v);
                }
                if let Some(v) = self.magic(obj, GET, name, None)? {
                    return Ok(v);
                }
                diag::warning(format!("Undefined property: {class}::${name}"));
                Ok(Value::Null)
            }
        }
    }

    pub(super) fn write_prop(&mut self, obj: &Rc<Object>, name: &str, v: Value) -> EResult<()> {
        let class = obj.class.name;
        match self.find_prop(&obj.class, name) {
            PropLookup::Declared(p) => {
                let initialized = Self::stored(obj, &p.key).is_some();
                if p.readonly {
                    if initialized {
                        return Err(Fatal::error(format!("Cannot modify readonly property {}::${name}", p.class)));
                    }
                    let in_scope = self.ctx().scope.as_ref().is_some_and(|s| s.name == p.class);
                    if !in_scope {
                        return Err(Fatal::error(format!(
                            "Cannot initialize readonly property {}::${name} from {}",
                            p.class,
                            self.scope_desc()
                        )));
                    }
                }
                let v = match &p.ty {
                    Some(ty) => self.coerce_prop(v, ty, &p)?,
                    None => v,
                };
                self.store(obj, p.key, v)
            }
            PropLookup::Inaccessible(p) => match self.magic(obj, SET, name, Some(v))? {
                Some(_) => Ok(()),
                None => Err(Fatal::error(format!("Cannot access {} property {class}::${name}", p.vis.name()))),
            },
            PropLookup::Dynamic => {
                let key = Key::Str(name.as_bytes().into());
                if Self::stored(obj, &key).is_none() {
                    if self.magic(obj, SET, name, Some(v.clone()))?.is_some() {
                        return Ok(());
                    }
                    self.create_dynamic(obj, name)?;
                }
                self.store(obj, key, v)
            }
        }
    }

    fn create_dynamic(&self, obj: &Object, name: &str) -> EResult<()> {
        let class = obj.class.name;
        if obj.class.is_readonly {
            return Err(Fatal::error(format!("Cannot create dynamic property {class}::${name}")));
        }
        if !obj.class.allow_dynamic {
            diag::deprecated(format!("Creation of dynamic property {class}::${name} is deprecated"));
        }
        Ok(())
    }

    fn store(&self, obj: &Object, key: Key, v: Value) -> EResult<()> {
        let mut props = obj.props.try_borrow_mut().map_err(|_| super::recursive_reference())?;
        super::set_slot(props.entry(key), v)
    }

    fn coerce_prop(&mut self, v: Value, ty: &TypeHint, p: &PropInfo) -> EResult<Value> {
        let err = |v: &Value, _: &str| {
            Fatal::type_error(format!(
                "Cannot assign {} to property {}::${} of type {}",
                v.type_name(),
                p.class,
                p.name,
                ty.display
            ))
        };
        self.check_param(v, Some(ty), &err)
    }

    /// The storage key for writing into a property in place (`$o->a[] = 1`, `$o->n++`)
    pub(super) fn prop_key_for_update(&mut self, obj: &Rc<Object>, name: &str, read: bool) -> EResult<Key> {
        let class = obj.class.name;
        match self.find_prop(&obj.class, name) {
            PropLookup::Declared(p) => {
                let initialized = Self::stored(obj, &p.key).is_some();
                if p.readonly && initialized {
                    return Err(Fatal::error(format!("Cannot indirectly modify readonly property {}::${name}", p.class)));
                }
                if !initialized && p.ty.is_some() {
                    return Err(Fatal::error(format!(
                        "Typed property {}::${name} must not be accessed before initialization",
                        p.class
                    )));
                }
                Ok(p.key)
            }
            PropLookup::Inaccessible(p) => {
                Err(Fatal::error(format!("Cannot access {} property {class}::${name}", p.vis.name())))
            }
            PropLookup::Dynamic => {
                let key = Key::Str(name.as_bytes().into());
                if Self::stored(obj, &key).is_none() {
                    if read {
                        diag::warning(format!("Undefined property: {class}::${name}"));
                    }
                    self.create_dynamic(obj, name)?;
                }
                Ok(key)
            }
        }
    }

    /// isset()/??/empty() on a property: None when it does not exist (or is not visible)
    pub(super) fn isset_prop(&mut self, obj: &Rc<Object>, name: &str, need_value: bool) -> EResult<Option<Value>> {
        let stored = match self.find_prop(&obj.class, name) {
            PropLookup::Declared(p) => Self::stored(obj, &p.key),
            PropLookup::Inaccessible(_) => None,
            PropLookup::Dynamic => Self::stored(obj, &Key::Str(name.as_bytes().into())),
        };
        if stored.is_some() {
            return Ok(stored);
        }
        match self.magic(obj, ISSET, name, None)? {
            Some(r) if r.to_bool() && !need_value => Ok(Some(Value::Bool(true))),
            Some(r) if r.to_bool() => Ok(Some(self.magic(obj, GET, name, None)?.unwrap_or(Value::Null))),
            _ => Ok(None),
        }
    }

    pub(super) fn unset_prop(&mut self, obj: &Rc<Object>, name: &str) -> EResult<()> {
        let class = obj.class.name;
        let key = match self.find_prop(&obj.class, name) {
            PropLookup::Declared(p) => {
                if p.readonly {
                    return Err(Fatal::error(format!("Cannot unset readonly property {}::${name}", p.class)));
                }
                p.key
            }
            PropLookup::Inaccessible(p) => {
                return match self.magic(obj, UNSET, name, None)? {
                    Some(_) => Ok(()),
                    None => Err(Fatal::error(format!("Cannot access {} property {class}::${name}", p.vis.name()))),
                };
            }
            PropLookup::Dynamic => {
                let key = Key::Str(name.as_bytes().into());
                if Self::stored(obj, &key).is_none() {
                    self.magic(obj, UNSET, name, None)?;
                    return Ok(());
                }
                key
            }
        };
        obj.props.try_borrow_mut().map_err(|_| super::recursive_reference())?.remove(&key);
        Ok(())
    }

    /// Properties visible from the current scope, as foreach and get_object_vars see them
    pub(super) fn visible_props(&self, obj: &Object) -> Vec<(Value, Value)> {
        if obj.closure().is_some() {
            return Vec::new();
        }
        obj.ordered_props()
            .into_iter()
            .filter_map(|(k, v)| {
                let (name, vis, class) = unmangle(&k);
                let visible = match vis {
                    Visibility::Private => self.can_access(vis, class.as_deref().unwrap_or("")),
                    _ => self.can_access(vis, obj.class.name),
                };
                visible.then(|| (Value::str(name), v.copy_out()))
            })
            .collect()
    }

    /// A static property's storage, plus its type and `Class::$name` when it is typed
    pub(super) fn static_prop_cell(
        &mut self,
        class_ref: &ClassRef,
        name: &str,
    ) -> EResult<(RefCellValue, Option<(TypeHint, String)>)> {
        let class = self.resolve_class(class_ref)?;
        let Some(p) = class.find_static(name) else {
            return Err(Fatal::error(format!("Access to undeclared static property {}::${name}", class.name)));
        };
        if !self.can_access(p.vis, p.class) {
            return Err(Fatal::error(format!("Cannot access {} property {}::${name}", p.vis.name(), class.name)));
        }
        let ty = p.ty.clone().map(|t| (t, format!("{}::${name}", p.class)));
        Ok((p.cell.clone(), ty))
    }

    pub(super) fn class_const(&mut self, class_ref: &ClassRef, name: &str) -> EResult<Value> {
        if name.eq_ignore_ascii_case("class") {
            return match class_ref {
                ClassRef::Named(n) => Ok(Value::str(n.as_str())),
                ClassRef::Dynamic(e) => match self.expr(e)? {
                    Value::Object(o) => Ok(Value::str(o.class.name)),
                    other => Err(Fatal::type_error(format!("Cannot use \"::class\" on {}", other.type_name()))),
                },
                other => Ok(Value::str(self.resolve_class(other)?.name)),
            };
        }
        let class = self.resolve_class(class_ref)?;
        let Some(c) = class.consts.get(name).cloned() else {
            return Err(Fatal::error(format!("Undefined constant {}::{name}", class.name)));
        };
        if !self.can_access(c.vis, c.class) {
            return Err(Fatal::error(format!("Cannot access {} constant {}::{name}", c.vis.name(), class.name)));
        }
        let expr = {
            let mut state = c.state.borrow_mut();
            match std::mem::replace(&mut *state, ConstState::Evaluating) {
                ConstState::Done(v) => {
                    *state = ConstState::Done(v.clone());
                    return Ok(v);
                }
                ConstState::Evaluating => {
                    return Err(Fatal::error(format!("Cannot declare self-referencing constant {}::{name}", c.class)));
                }
                ConstState::Unevaluated(e) => e,
            }
        };
        // Constants are evaluated lazily, in the scope of the class declaring them
        let scope = self.classes.get(&c.class.to_ascii_lowercase()).cloned().unwrap_or(class);
        let result = self.with_scope(scope, |this| this.expr(&expr));
        *c.state.borrow_mut() = match &result {
            Ok(v) => ConstState::Done(v.clone()),
            Err(_) => ConstState::Unevaluated(expr),
        };
        result
    }

    // ---- Callables and type checks ----

    /// Resolves a callable value: closures, invokable objects, function names,
    /// "Class::method" strings and [object-or-class, method] arrays
    pub(super) fn callable(&mut self, v: &Value) -> EResult<Option<Callable>> {
        Ok(match v {
            Value::Object(o) if o.closure().is_some() => Some(Callable::Closure(o.clone())),
            Value::Object(o) => o.class.find_method("__invoke").cloned().map(|method| Callable::Method {
                this: Some(o.clone()),
                method,
                class: o.class.clone(),
            }),
            Value::Str(s) => {
                let name = String::from_utf8_lossy(s).trim_start_matches('\\').to_string();
                if let Some((class, method)) = name.split_once("::") {
                    return self.method_callable(Value::str(class), method);
                }
                let lname = name.to_ascii_lowercase();
                if let Some(f) = self.functions.get(&lname) {
                    Some(Callable::User(f.clone()))
                } else {
                    builtins::exists(&lname).then_some(Callable::Builtin(lname))
                }
            }
            Value::Array(a) if a.len() == 2 => {
                let target = a.get(&Key::Int(0)).map(Value::copy_out);
                let method = a.get(&Key::Int(1)).map(Value::copy_out);
                match (target, method) {
                    (Some(t), Some(Value::Str(m))) => self.method_callable(t, &String::from_utf8_lossy(&m))?,
                    _ => None,
                }
            }
            _ => None,
        })
    }

    fn method_callable(&mut self, target: Value, method: &str) -> EResult<Option<Callable>> {
        let (this, class) = match target {
            Value::Object(o) => (Some(o.clone()), o.class.clone()),
            Value::Str(s) => {
                let n = String::from_utf8_lossy(&s).to_ascii_lowercase();
                let class = match n.as_str() {
                    "self" | "static" | "parent" => self.resolve_class(&ClassRef::from_name(&n))?,
                    _ => match self.classes.get(&n) {
                        Some(c) => c.clone(),
                        None => return Ok(None),
                    },
                };
                (None, class)
            }
            _ => return Ok(None),
        };
        let Some(m) = class.find_method(method).cloned() else { return Ok(None) };
        if !self.can_access(m.vis, m.class) {
            return Ok(None);
        }
        let this = if m.is_static { None } else { this.or_else(|| self.ctx().this.clone()) };
        if !m.is_static && this.is_none() {
            return Ok(None);
        }
        Ok(Some(Callable::Method { this, method: m, class }))
    }

    pub(super) fn object_matches(&self, o: &Object, ty: &TypeHint) -> bool {
        let ctx = self.ctx();
        let tc = TypeCtx {
            functions: &self.functions,
            classes: &self.classes,
            scope: ctx.scope.as_deref(),
            static_class: ctx.static_class.as_deref(),
        };
        ty.types.iter().any(|t| tc.class_matches(&o.class, t))
    }

    /// Built-in functions about classes and objects, which need the class table or scope
    pub(super) fn object_builtin(&mut self, lname: &str, vals: &[Value]) -> EResult<Option<Value>> {
        let class_arg = |this: &Self, v: Option<&Value>| -> Option<Rc<ClassEntry>> {
            match v? {
                Value::Object(o) => Some(o.class.clone()),
                Value::Str(s) => this.classes.get(&String::from_utf8_lossy(s).trim_start_matches('\\').to_ascii_lowercase()).cloned(),
                _ => None,
            }
        };
        let name_arg = |v: Option<&Value>| v.map(|v| String::from_utf8_lossy(&v.to_bytes()).into_owned()).unwrap_or_default();
        Ok(Some(match lname {
            "get_class" => match vals.first() {
                None => match &self.ctx().scope {
                    Some(s) => Value::str(s.name),
                    None => return Err(Fatal::error("get_class() without arguments must be called from within a class")),
                },
                Some(Value::Object(o)) => Value::str(o.class.name),
                Some(other) => {
                    return Err(Fatal::type_error(format!(
                        "get_class(): Argument #1 ($object) must be of type object, {} given",
                        other.type_name()
                    )))
                }
            },
            "get_parent_class" => {
                let class = if vals.is_empty() { self.ctx().scope.clone() } else { class_arg(self, vals.first()) };
                match class.and_then(|c| c.parent.clone()) {
                    Some(p) => Value::str(p.name),
                    None => Value::Bool(false),
                }
            }
            "get_called_class" => match &self.ctx().static_class {
                Some(c) => Value::str(c.name),
                None => return Err(Fatal::error("get_called_class() must be called from within a class")),
            },
            "get_object_vars" => match vals.first() {
                Some(Value::Object(o)) => {
                    let props = self.visible_props(o);
                    Value::Array(Rc::new(props.into_iter().map(|(k, v)| (to_key(&k, "array").unwrap(), v)).collect()))
                }
                _ => return Err(Fatal::type_error("get_object_vars(): Argument #1 ($object) must be of type object")),
            },
            "method_exists" => Value::Bool(class_arg(self, vals.first()).is_some_and(|c| c.find_method(&name_arg(vals.get(1))).is_some())),
            "property_exists" => {
                let name = name_arg(vals.get(1));
                let declared = class_arg(self, vals.first())
                    .is_some_and(|c| c.props.iter().any(|p| p.name == name) || c.find_static(&name).is_some());
                let dynamic = matches!(vals.first(), Some(Value::Object(o)) if Self::stored(o, &Key::Str(name.as_bytes().into())).is_some());
                Value::Bool(declared || dynamic)
            }
            "class_exists" => Value::Bool(class_arg(self, vals.first()).is_some_and(|c| !c.is_interface)),
            "interface_exists" => Value::Bool(class_arg(self, vals.first()).is_some_and(|c| c.is_interface)),
            "is_a" | "is_subclass_of" => {
                let allow_string = lname == "is_subclass_of" || vals.get(2).is_some_and(Value::to_bool);
                let class = match vals.first() {
                    Some(Value::Object(o)) => Some(o.class.clone()),
                    Some(Value::Str(_)) if allow_string => class_arg(self, vals.first()),
                    _ => None,
                };
                let target = name_arg(vals.get(1));
                let target = target.trim_start_matches('\\');
                Value::Bool(class.is_some_and(|c| {
                    c.is_subclass_of(target) && (lname == "is_a" || !c.name.eq_ignore_ascii_case(target))
                }))
            }
            "spl_object_id" => match vals.first() {
                Some(Value::Object(o)) => Value::Int(o.id as i64),
                _ => return Err(Fatal::type_error("spl_object_id(): Argument #1 ($object) must be of type object")),
            },
            "spl_object_hash" => match vals.first() {
                Some(Value::Object(o)) => Value::str(format!("{:032x}", o.id)),
                _ => return Err(Fatal::type_error("spl_object_hash(): Argument #1 ($object) must be of type object")),
            },
            _ => return Ok(None),
        }))
    }
}

/// Adds a declared property, replacing an inherited one of the same name in place
fn add_prop(entry: &mut ClassEntry, info: PropInfo) -> EResult<()> {
    let inherited = entry.props.iter().position(|p| p.name == info.name && p.vis != Visibility::Private);
    match inherited {
        Some(i) => {
            let parent = &entry.props[i];
            let weaker = matches!(
                (parent.vis, info.vis),
                (Visibility::Public, Visibility::Protected | Visibility::Private) | (Visibility::Protected, Visibility::Private)
            );
            if weaker {
                return Err(Fatal::compile(format!(
                    "Access level to {}::${} must be {} (as in class {}){}",
                    info.class,
                    info.name,
                    parent.vis.name(),
                    parent.class,
                    if parent.vis == Visibility::Protected { " or weaker" } else { "" }
                )));
            }
            entry.props[i] = info;
        }
        None => entry.props.push(info),
    }
    Ok(())
}

/// Adds a declared method, checking it against the one it overrides
fn add_method(entry: &mut ClassEntry, m: Method) -> EResult<()> {
    let key = m.name.to_ascii_lowercase();
    if let Some(parent) = entry.methods.get(&key) {
        // Private methods are not inherited in a way that constrains the child
        if parent.vis != Visibility::Private && parent.class != m.class {
            if parent.is_final {
                return Err(Fatal::compile(format!("Cannot override final method {}::{}()", parent.class, parent.name)));
            }
            if parent.is_static && !m.is_static {
                return Err(Fatal::compile(format!(
                    "Cannot make static method {}::{}() non static in class {}",
                    parent.class, parent.name, m.class
                )));
            }
            if !parent.is_static && m.is_static {
                return Err(Fatal::compile(format!(
                    "Cannot make non static method {}::{}() static in class {}",
                    parent.class, parent.name, m.class
                )));
            }
            let weaker = matches!(
                (parent.vis, m.vis),
                (Visibility::Public, Visibility::Protected | Visibility::Private) | (Visibility::Protected, Visibility::Private)
            );
            if weaker {
                return Err(Fatal::compile(format!(
                    "Access level to {}::{}() must be {} (as in class {}){}",
                    m.class,
                    m.name,
                    parent.vis.name(),
                    parent.class,
                    if parent.vis == Visibility::Protected { " or weaker" } else { "" }
                )));
            }
        }
        // An abstract declaration does not replace an existing implementation
        if m.is_abstract && !parent.is_abstract {
            return Ok(());
        }
    }
    entry.methods.insert(key, Rc::new(m));
    Ok(())
}

impl<'e> Path<'e> {
    /// Whether this path writes a whole property, which goes through write_prop
    pub(super) fn whole_prop(&self) -> Option<(&Rc<Object>, &str)> {
        match (&self.base, self.dims.is_empty()) {
            (Base::Prop(o, n), true) => Some((o, n.as_str())),
            _ => None,
        }
    }
}
