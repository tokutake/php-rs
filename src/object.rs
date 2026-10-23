//! Objects and the built-in class hierarchy.
//!
//! Objects are handles: copying a value shares the object, so they live behind
//! an Rc. Handle numbers (`#3` in var_dump) are recycled like php-src's object
//! store: a freed number is handed out again before a new one is allocated.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::FuncDecl;
use crate::value::Value;

#[derive(Debug)]
pub struct ClassEntry {
    pub name: &'static str,
    pub parent: Option<Rc<ClassEntry>>,
    pub interfaces: Vec<&'static str>,
    pub is_interface: bool,
    /// Classes like Closure that cannot be created with `new`
    pub instantiable: bool,
}

impl ClassEntry {
    /// `instanceof` by (case-insensitive) class or interface name
    pub fn is_subclass_of(&self, name: &str) -> bool {
        let mut cur = Some(self);
        while let Some(c) = cur {
            if c.name.eq_ignore_ascii_case(name) || c.interfaces.iter().any(|i| i.eq_ignore_ascii_case(name)) {
                return true;
            }
            cur = c.parent.as_deref();
        }
        false
    }

    /// The class that declares the private Exception/Error properties
    pub fn throwable_base(&self) -> &'static str {
        if self.is_subclass_of("Error") { "Error" } else { "Exception" }
    }
}

pub struct Object {
    pub id: u32,
    pub class: Rc<ClassEntry>,
    pub kind: ObjectKind,
}

pub enum ObjectKind {
    Throwable(RefCell<ThrowableData>),
    Closure(Closure),
}

#[derive(Debug, Clone)]
pub struct ThrowableData {
    pub message: Vec<u8>,
    pub code: i64,
    pub file: String,
    pub line: u32,
    pub trace: Vec<TraceFrame>,
    pub previous: Option<Rc<Object>>,
}

#[derive(Debug, Clone)]
pub struct TraceFrame {
    pub file: String,
    pub line: u32,
    pub function: String,
    pub args: Vec<Value>,
}

pub struct Closure {
    pub func: Rc<FuncDecl>,
    /// Captured `use` variables; by-reference captures hold a Value::Ref
    pub bound: Vec<(String, Value)>,
    /// Display name, `{closure:FILE:LINE}`
    pub name: String,
    pub file: String,
}

pub enum Visibility {
    Public,
    Protected,
    /// Private to the named class
    Private(&'static str),
}

/// Properties as var_dump and print_r show them
pub fn debug_props(o: &Object) -> Vec<(String, Visibility, Value)> {
    use Visibility::*;
    match &o.kind {
        ObjectKind::Throwable(t) => {
            let Ok(t) = t.try_borrow() else { return Vec::new() };
            let base = o.class.throwable_base();
            vec![
                ("message".into(), Protected, Value::str(t.message.clone())),
                ("string".into(), Private(base), Value::str(Vec::new())),
                ("code".into(), Protected, Value::Int(t.code)),
                ("file".into(), Protected, Value::str(t.file.clone())),
                ("line".into(), Protected, Value::Int(t.line as i64)),
                ("trace".into(), Private(base), crate::interp::trace_array(&t.trace)),
                ("previous".into(), Private(base), t.previous.clone().map_or(Value::Null, Value::Object)),
            ]
        }
        ObjectKind::Closure(c) => {
            let mut props = vec![
                ("name".into(), Public, Value::str(c.name.clone())),
                ("file".into(), Public, Value::str(c.file.clone())),
                ("line".into(), Public, Value::Int(c.func.line as i64)),
            ];
            if !c.bound.is_empty() {
                let vars = c.bound.iter().map(|(k, v)| (crate::array::Key::from_bytes(k.as_bytes()), v.clone())).collect();
                props.push(("static".into(), Public, Value::Array(Rc::new(vars))));
            }
            if !c.func.params.is_empty() {
                let params = c
                    .func
                    .params
                    .iter()
                    .map(|p| {
                        let name = format!("{}${}", if p.by_ref { "&" } else { "" }, p.name);
                        let kind = if p.default.is_some() || p.variadic { "<optional>" } else { "<required>" };
                        (crate::array::Key::from_bytes(name.as_bytes()), Value::str(kind))
                    })
                    .collect();
                props.push(("parameter".into(), Public, Value::Array(Rc::new(params))));
            }
            props
        }
    }
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
    pub fn new(class: Rc<ClassEntry>, kind: ObjectKind) -> Rc<Object> {
        let id = FREE_IDS.with(|f| f.borrow_mut().pop()).unwrap_or_else(|| {
            NEXT_ID.with(|n| {
                let id = n.get();
                n.set(id + 1);
                id
            })
        });
        Rc::new(Object { id, class, kind })
    }

    pub fn throwable(&self) -> Option<std::cell::Ref<'_, ThrowableData>> {
        match &self.kind {
            ObjectKind::Throwable(t) => t.try_borrow().ok(),
            _ => None,
        }
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        let id = self.id;
        // try_with: during thread teardown the free list may already be gone
        let _ = FREE_IDS.try_with(|f| f.borrow_mut().push(id));
    }
}

/// Built-in classes by lowercase name
pub fn builtin_classes() -> HashMap<String, Rc<ClassEntry>> {
    let mut map: HashMap<String, Rc<ClassEntry>> = HashMap::new();
    let mut add = |name: &'static str, parent: Option<&str>, interfaces: Vec<&'static str>, kind: u8| {
        let parent = parent.map(|p| map[&p.to_ascii_lowercase()].clone());
        let entry = ClassEntry { name, parent, interfaces, is_interface: kind == 1, instantiable: kind == 0 };
        map.insert(name.to_ascii_lowercase(), Rc::new(entry));
    };
    const CLASS: u8 = 0;
    const INTERFACE: u8 = 1;
    const FINAL_INTERNAL: u8 = 2;
    add("Stringable", None, vec![], INTERFACE);
    add("Throwable", None, vec!["Stringable"], INTERFACE);
    add("Exception", None, vec!["Throwable", "Stringable"], CLASS);
    add("Error", None, vec!["Throwable", "Stringable"], CLASS);
    add("ErrorException", Some("Exception"), vec![], CLASS);
    for (name, parent) in [
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
        add(name, Some(parent), vec![], CLASS);
    }
    add("Closure", None, vec![], FINAL_INTERNAL);
    map
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
            cur = obj.throwable().and_then(|t| t.previous.clone());
        }
        false
    };
    if in_chain(ex, &previous) || in_chain(&previous, ex) {
        return;
    }
    let mut last = ex.clone();
    loop {
        let next = last.throwable().and_then(|t| t.previous.clone());
        match next {
            Some(n) => last = n,
            None => break,
        }
    }
    if let ObjectKind::Throwable(t) = &last.kind
        && let Ok(mut t) = t.try_borrow_mut() {
            t.previous = Some(previous);
        }
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

/// `#0 file(line): func(args)` lines followed by `#n {main}`
pub fn trace_string(frames: &[TraceFrame]) -> String {
    let mut out = String::new();
    for (i, f) in frames.iter().enumerate() {
        let args: Vec<String> = f.args.iter().map(trace_arg).collect();
        out.push_str(&format!("#{i} {}({}): {}({})\n", f.file, f.line, f.function, args.join(", ")));
    }
    out.push_str(&format!("#{} {{main}}", frames.len()));
    out
}

/// Exception::__toString: the previous chain first, then each outer exception after "Next"
pub fn to_string(o: &Rc<Object>) -> Option<Vec<u8>> {
    let mut chain = Vec::new();
    let mut cur = Some(o.clone());
    // Bounded so that a cycle through `previous` cannot loop forever
    while let Some(obj) = cur.take() {
        if chain.len() >= 10_000 {
            break;
        }
        let t = obj.throwable()?;
        let mut msg = String::from_utf8_lossy(&t.message).into_owned();
        // Argument errors point at the declaration too
        if matches!(obj.class.name, "TypeError" | "ArgumentCountError") && msg.contains(", called in ") {
            msg.push_str(" and defined");
        }
        let head = if msg.is_empty() { obj.class.name.to_string() } else { format!("{}: {msg}", obj.class.name) };
        chain.push(format!("{head} in {}:{}\nStack trace:\n{}", t.file, t.line, trace_string(&t.trace)));
        cur = t.previous.clone();
    }
    chain.reverse();
    Some(chain.join("\n\nNext ").into_bytes())
}
