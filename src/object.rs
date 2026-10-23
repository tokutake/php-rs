//! Objects and classes.
//!
//! Objects are handles: copying a value shares the object, so they live behind
//! an Rc and keep their properties in a RefCell. Handle numbers (`#3` in
//! var_dump) are recycled like php-src's object store: a freed number is handed
//! out again before a new one is allocated.
//!
//! Properties are stored under php-src's mangled names: `"\0A\0x"` for a
//! private `$x` declared in class A, `"\0*\0x"` for a protected one and `"x"`
//! for public and dynamic ones. A parent's private property and a child's
//! property of the same name therefore stay distinct, and `(array)` casts and
//! var_dump come out exactly as in PHP.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::array::{Key, PhpArray};
use crate::ast::{Expr, FuncDecl, TypeHint};
use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

impl Visibility {
    pub fn name(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Protected => "protected",
            Visibility::Private => "private",
        }
    }
}

/// The storage key of a property declared in `class`
pub fn mangle(name: &str, vis: Visibility, class: &str) -> Key {
    let s = match vis {
        Visibility::Public => name.to_string(),
        Visibility::Protected => format!("\0*\0{name}"),
        Visibility::Private => format!("\0{class}\0{name}"),
    };
    Key::Str(s.into_bytes().into())
}

/// Splits a mangled key into its name, visibility and (for private) class
pub fn unmangle(key: &Key) -> (String, Visibility, Option<String>) {
    let bytes: Vec<u8> = match key {
        Key::Int(i) => return (i.to_string(), Visibility::Public, None),
        Key::Str(s) => s.to_vec(),
    };
    if let [0, rest @ ..] = bytes.as_slice()
        && let Some(sep) = rest.iter().position(|&b| b == 0) {
            let class = String::from_utf8_lossy(&rest[..sep]).into_owned();
            let name = String::from_utf8_lossy(&rest[sep + 1..]).into_owned();
            return if class == "*" {
                (name, Visibility::Protected, None)
            } else {
                (name, Visibility::Private, Some(class))
            };
        }
    (String::from_utf8_lossy(&bytes).into_owned(), Visibility::Public, None)
}

#[derive(Debug, Clone)]
pub struct PropInfo {
    pub name: String,
    pub key: Key,
    pub vis: Visibility,
    /// The declaring class
    pub class: &'static str,
    pub ty: Option<TypeHint>,
    /// None leaves a typed property uninitialized; untyped ones default to null
    pub default: Option<Value>,
    pub readonly: bool,
}

#[derive(Debug, Clone)]
pub struct StaticProp {
    pub vis: Visibility,
    pub class: &'static str,
    pub ty: Option<TypeHint>,
    pub cell: Rc<RefCell<Value>>,
}

#[derive(Debug)]
pub enum ConstState {
    Unevaluated(Expr),
    Evaluating,
    Done(Value),
}

#[derive(Debug)]
pub struct ClassConst {
    pub vis: Visibility,
    pub class: &'static str,
    pub state: RefCell<ConstState>,
}

/// Methods implemented natively for built-in classes
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Native {
    ThrowableConstruct,
    GetMessage,
    GetCode,
    GetPrevious,
    GetFile,
    GetLine,
    GetTrace,
    GetTraceAsString,
    ThrowableToString,
    ClosureInvoke,
}

#[derive(Debug)]
pub enum MethodBody {
    User(Rc<FuncDecl>),
    Native(Native),
}

#[derive(Debug)]
pub struct Method {
    /// As declared, for messages
    pub name: String,
    /// The declaring class
    pub class: &'static str,
    pub vis: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    pub body: MethodBody,
}

#[derive(Debug, Default, Clone)]
pub struct ClassEntry {
    pub name: &'static str,
    pub parent: Option<Rc<ClassEntry>>,
    /// Every interface implemented, including inherited ones
    pub interfaces: Vec<&'static str>,
    pub is_interface: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    pub is_readonly: bool,
    /// Built-ins like Closure that `new` refuses
    pub internal_only: bool,
    pub is_throwable: bool,
    pub allow_dynamic: bool,
    /// Instance properties in layout order, inherited ones first
    pub props: Vec<PropInfo>,
    /// Own static properties; lookups walk up to the parent
    pub static_props: HashMap<String, StaticProp>,
    /// Constants by name, including inherited ones
    pub consts: HashMap<String, Rc<ClassConst>>,
    /// Methods by lowercase name, including inherited ones
    pub methods: HashMap<String, Rc<Method>>,
}

impl ClassEntry {
    /// `instanceof` by (case-insensitive) class or interface name
    pub fn is_subclass_of(&self, name: &str) -> bool {
        let mut cur = Some(self);
        while let Some(c) = cur {
            if c.name.eq_ignore_ascii_case(name) {
                return true;
            }
            cur = c.parent.as_deref();
        }
        self.interfaces.iter().any(|i| i.eq_ignore_ascii_case(name))
    }

    pub fn find_method(&self, name: &str) -> Option<&Rc<Method>> {
        self.methods.get(&name.to_ascii_lowercase())
    }

    /// The declaration a static property name resolves to
    pub fn find_static(&self, name: &str) -> Option<&StaticProp> {
        let mut cur = Some(self);
        while let Some(c) = cur {
            if let Some(p) = c.static_props.get(name) {
                return Some(p);
            }
            cur = c.parent.as_deref();
        }
        None
    }

    /// The class that declares the private Exception/Error properties
    pub fn throwable_base(&self) -> &'static str {
        if self.is_subclass_of("Error") { "Error" } else { "Exception" }
    }
}

pub struct Object {
    pub id: u32,
    pub class: Rc<ClassEntry>,
    pub props: RefCell<PhpArray>,
    pub internal: Internal,
}

pub enum Internal {
    None,
    Closure(Closure),
}

#[derive(Debug, Clone)]
pub struct TraceFrame {
    pub file: String,
    pub line: u32,
    pub function: String,
    /// `Class` and `->` or `::` for method frames
    pub class: Option<(String, &'static str)>,
    pub args: Vec<Value>,
}

pub struct Closure {
    pub func: Rc<FuncDecl>,
    /// Captured `use` variables; by-reference captures hold a Value::Ref
    pub bound: Vec<(String, Value)>,
    /// Display name, `{closure:FILE:LINE}`
    pub name: String,
    pub file: String,
    pub this: Option<Rc<Object>>,
    pub scope: Option<Rc<ClassEntry>>,
}

impl std::fmt::Debug for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "object({})#{}", self.class.name, self.id)
    }
}

thread_local! {
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    static FREE_IDS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

impl Object {
    /// A new object with the class's default property values
    pub fn new(class: Rc<ClassEntry>, internal: Internal) -> Rc<Object> {
        let id = FREE_IDS.with(|f| f.borrow_mut().pop()).unwrap_or_else(|| {
            NEXT_ID.with(|n| {
                let id = n.get();
                n.set(id + 1);
                id
            })
        });
        let mut props = PhpArray::new();
        for p in &class.props {
            match (&p.default, &p.ty) {
                (Some(v), _) => props.insert(p.key.clone(), v.clone()),
                (None, None) => props.insert(p.key.clone(), Value::Null),
                // Typed without a default: uninitialized, i.e. absent
                (None, Some(_)) => {}
            }
        }
        Rc::new(Object { id, class, props: RefCell::new(props), internal })
    }

    /// A property by storage key, for native code
    pub fn get(&self, key: &str) -> Value {
        let key = Key::Str(key.as_bytes().into());
        self.props.try_borrow().ok().and_then(|p| p.get(&key).map(Value::copy_out)).unwrap_or(Value::Null)
    }

    pub fn set(&self, key: &str, v: Value) {
        if let Ok(mut p) = self.props.try_borrow_mut() {
            p.insert(Key::Str(key.as_bytes().into()), v);
        }
    }

    /// Initialized properties in slot order: declared ones in class layout order
    /// (even if re-set later), then dynamic ones in creation order
    pub fn ordered_props(&self) -> Vec<(Key, Value)> {
        let Ok(stored) = self.props.try_borrow() else { return Vec::new() };
        let mut out: Vec<(Key, Value)> =
            self.class.props.iter().filter_map(|p| stored.get(&p.key).map(|v| (p.key.clone(), v.clone()))).collect();
        for (k, v) in stored.iter() {
            if !self.class.props.iter().any(|p| &p.key == k) {
                out.push((k.clone(), v.clone()));
            }
        }
        out
    }

    pub fn closure(&self) -> Option<&Closure> {
        match &self.internal {
            Internal::Closure(c) => Some(c),
            Internal::None => None,
        }
    }

    /// Storage key of a Throwable's private property (declared in Exception or Error)
    pub fn throwable_key(&self, name: &str) -> String {
        format!("\0{}\0{name}", self.class.throwable_base())
    }

    pub fn previous(&self) -> Option<Rc<Object>> {
        match self.get(&self.throwable_key("previous")) {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        // Like php-src, the object's contents are released before its handle is,
        // so handles of objects it held are recycled first
        drop(std::mem::take(self.props.get_mut()));
        drop(std::mem::replace(&mut self.internal, Internal::None));
        let id = self.id;
        // try_with: during thread teardown the free list may already be gone
        let _ = FREE_IDS.try_with(|f| f.borrow_mut().push(id));
    }
}

fn native(name: &str, class: &'static str, body: Native, is_final: bool) -> (String, Rc<Method>) {
    let m = Method {
        name: name.to_string(),
        class,
        vis: Visibility::Public,
        is_static: false,
        is_abstract: false,
        is_final,
        body: MethodBody::Native(body),
    };
    (name.to_ascii_lowercase(), Rc::new(m))
}

/// The Throwable base classes, Exception and Error, with their properties and methods
fn throwable_base(name: &'static str) -> ClassEntry {
    use Visibility::*;
    let prop = |pname: &str, vis: Visibility, default: Value| PropInfo {
        name: pname.to_string(),
        key: mangle(pname, vis, name),
        vis,
        class: name,
        ty: None,
        default: Some(default),
        readonly: false,
    };
    let props = vec![
        prop("message", Protected, Value::str("")),
        prop("string", Private, Value::str("")),
        prop("code", Protected, Value::Int(0)),
        prop("file", Protected, Value::str("")),
        prop("line", Protected, Value::Int(0)),
        prop("trace", Private, Value::Array(Rc::new(PhpArray::new()))),
        prop("previous", Private, Value::Null),
    ];
    let methods = [
        native("__construct", name, Native::ThrowableConstruct, false),
        native("getMessage", name, Native::GetMessage, true),
        native("getCode", name, Native::GetCode, true),
        native("getPrevious", name, Native::GetPrevious, true),
        native("getFile", name, Native::GetFile, true),
        native("getLine", name, Native::GetLine, true),
        native("getTrace", name, Native::GetTrace, true),
        native("getTraceAsString", name, Native::GetTraceAsString, true),
        native("__toString", name, Native::ThrowableToString, false),
    ]
    .into_iter()
    .collect();
    ClassEntry {
        name,
        interfaces: vec!["Throwable", "Stringable"],
        is_throwable: true,
        props,
        methods,
        ..Default::default()
    }
}

/// A subclass that adds nothing, as the built-in exception classes are
pub fn plain_subclass(name: &'static str, parent: &Rc<ClassEntry>) -> ClassEntry {
    ClassEntry {
        name,
        parent: Some(parent.clone()),
        interfaces: parent.interfaces.clone(),
        is_throwable: parent.is_throwable,
        allow_dynamic: parent.allow_dynamic,
        props: parent.props.clone(),
        consts: parent.consts.clone(),
        methods: parent.methods.clone(),
        ..Default::default()
    }
}

/// Built-in classes by lowercase name
pub fn builtin_classes() -> HashMap<String, Rc<ClassEntry>> {
    let mut map: HashMap<String, Rc<ClassEntry>> = HashMap::new();
    let mut put = |c: ClassEntry| {
        let c = Rc::new(c);
        map.insert(c.name.to_ascii_lowercase(), c.clone());
        c
    };
    put(ClassEntry { name: "Stringable", is_interface: true, ..Default::default() });
    put(ClassEntry { name: "Throwable", is_interface: true, interfaces: vec!["Stringable"], ..Default::default() });
    put(ClassEntry { name: "stdClass", allow_dynamic: true, ..Default::default() });
    let exception = put(throwable_base("Exception"));
    let error = put(throwable_base("Error"));
    let mut by_name: HashMap<&str, Rc<ClassEntry>> = HashMap::new();
    by_name.insert("Exception", exception);
    by_name.insert("Error", error);
    for (name, parent) in [
        ("ErrorException", "Exception"),
        ("CompileError", "Error"),
        ("ParseError", "CompileError"),
        ("TypeError", "Error"),
        ("ArgumentCountError", "TypeError"),
        ("ValueError", "Error"),
        ("ArithmeticError", "Error"),
        ("DivisionByZeroError", "ArithmeticError"),
        ("UnhandledMatchError", "Error"),
        ("LogicException", "Exception"),
        ("BadFunctionCallException", "LogicException"),
        ("BadMethodCallException", "BadFunctionCallException"),
        ("DomainException", "LogicException"),
        ("InvalidArgumentException", "LogicException"),
        ("LengthException", "LogicException"),
        ("OutOfRangeException", "LogicException"),
        ("RuntimeException", "Exception"),
        ("OutOfBoundsException", "RuntimeException"),
        ("OverflowException", "RuntimeException"),
        ("RangeException", "RuntimeException"),
        ("UnderflowException", "RuntimeException"),
        ("UnexpectedValueException", "RuntimeException"),
        ("JsonException", "Exception"),
    ] {
        let c = put(plain_subclass(name, &by_name[parent]));
        by_name.insert(name, c);
    }
    let invoke = native("__invoke", "Closure", Native::ClosureInvoke, false);
    put(ClassEntry {
        name: "Closure",
        is_final: true,
        internal_only: true,
        methods: [invoke].into_iter().collect(),
        ..Default::default()
    });
    map
}

/// One argument as shown in a trace string: strings are cut at 15 bytes
pub fn trace_arg(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => crate::value::format_float_repr(*f),
        Value::Str(s) => {
            let shown = String::from_utf8_lossy(&s[..s.len().min(15)]).into_owned();
            format!("'{shown}{}'", if s.len() > 15 { "..." } else { "" })
        }
        Value::Array(_) => "Array".into(),
        Value::Object(o) => format!("Object({})", o.class.name),
        Value::Ref(r) => crate::value::with_deref(r, trace_arg),
    }
}

/// `#0 file(line): Class->func(args)` lines followed by `#n {main}`
pub fn trace_string(frames: &[TraceFrame]) -> String {
    let mut out = String::new();
    for (i, f) in frames.iter().enumerate() {
        let args: Vec<String> = f.args.iter().map(trace_arg).collect();
        let class = f.class.as_ref().map_or(String::new(), |(c, t)| format!("{c}{t}"));
        out.push_str(&format!("#{i} {}({}): {class}{}({})\n", f.file, f.line, f.function, args.join(", ")));
    }
    out.push_str(&format!("#{} {{main}}", frames.len()));
    out
}

/// getTrace(): one array per frame
pub fn trace_array(frames: &[TraceFrame]) -> Value {
    let rows = frames.iter().map(|f| {
        let mut a = PhpArray::new();
        let mut put = |k: &str, v: Value| a.insert(Key::from_bytes(k.as_bytes()), v);
        put("file", Value::str(f.file.clone()));
        put("line", Value::Int(f.line as i64));
        put("function", Value::str(f.function.clone()));
        if let Some((class, call)) = &f.class {
            put("class", Value::str(class.clone()));
            put("type", Value::str(*call));
        }
        put("args", Value::Array(Rc::new(PhpArray::from_values(f.args.clone()))));
        Value::Array(Rc::new(a))
    });
    Value::Array(Rc::new(PhpArray::from_values(rows)))
}

/// Recovers trace frames from a getTrace()-shaped array
pub fn trace_frames(v: &Value) -> Vec<TraceFrame> {
    let Value::Array(rows) = v else { return Vec::new() };
    rows.values()
        .filter_map(|row| {
            let Value::Array(r) = row.copy_out() else { return None };
            let get = |k: &str| r.get(&Key::from_bytes(k.as_bytes())).map(Value::copy_out);
            let text = |k: &str| get(k).map(|v| String::from_utf8_lossy(&v.to_bytes()).into_owned());
            let class = match (text("class"), text("type")) {
                (Some(c), Some(t)) => Some((c, if t == "::" { "::" } else { "->" })),
                _ => None,
            };
            let args = match get("args") {
                Some(Value::Array(a)) => a.values().map(Value::copy_out).collect(),
                _ => Vec::new(),
            };
            Some(TraceFrame {
                file: text("file").unwrap_or_default(),
                line: get("line").map_or(0, |l| l.to_int() as u32),
                function: text("function").unwrap_or_default(),
                class,
                args,
            })
        })
        .collect()
}

/// Exception::__toString: the previous chain first, then each outer exception after "Next"
pub fn throwable_to_string(o: &Rc<Object>) -> Vec<u8> {
    let mut chain = Vec::new();
    let mut cur = Some(o.clone());
    // Bounded so that a cycle through `previous` cannot loop forever
    while let Some(obj) = cur.take() {
        if chain.len() >= 10_000 {
            break;
        }
        let mut msg = String::from_utf8_lossy(&obj.get("\0*\0message").to_bytes()).into_owned();
        // Argument errors point at the declaration too
        if obj.class.is_subclass_of("TypeError") && msg.contains(", called in ") {
            msg.push_str(" and defined");
        }
        let class = obj.class.name;
        let head = if msg.is_empty() { class.to_string() } else { format!("{class}: {msg}") };
        let file = String::from_utf8_lossy(&obj.get("\0*\0file").to_bytes()).into_owned();
        let line = obj.get("\0*\0line").to_int();
        let trace = trace_frames(&obj.get(&obj.throwable_key("trace")));
        chain.push(format!("{head} in {file}:{line}\nStack trace:\n{}", trace_string(&trace)));
        cur = obj.previous();
    }
    chain.reverse();
    chain.join("\n\nNext ").into_bytes()
}

/// Appends `previous` at the end of `ex`'s previous chain, unless that would
/// create a cycle (it is already part of the chain, or `ex` is part of its chain)
pub fn chain_previous(ex: &Rc<Object>, previous: Rc<Object>) {
    let in_chain = |start: &Rc<Object>, needle: &Rc<Object>| {
        let mut cur = Some(start.clone());
        let mut steps = 0;
        while let Some(obj) = cur {
            if Rc::ptr_eq(&obj, needle) || steps > 10_000 {
                return true;
            }
            steps += 1;
            cur = obj.previous();
        }
        false
    };
    if in_chain(ex, &previous) || in_chain(&previous, ex) {
        return;
    }
    let mut last = ex.clone();
    while let Some(next) = last.previous() {
        last = next;
    }
    let key = last.throwable_key("previous");
    last.set(&key, Value::Object(previous));
}
