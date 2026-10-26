//! Objects and classes.
//!
//! Objects are handles: copying a value shares the object, so they live behind
//! an Rc and keep their properties in a RefCell. Handle numbers (`#3` in
//! var_dump) are recycled like php-src's object store: a freed number is handed
//! out again before a new one is allocated.
//!
//! `__destruct` runs when the last handle goes away. Drop cannot call into the
//! interpreter, so it moves the object's contents into a fresh Rc under the same
//! handle and queues that; the interpreter calls the queued destructors at its
//! next safe point (see `take_pending_destruct`). An object is destructed at most
//! once, and objects still alive at shutdown are reached through a registry of
//! the live objects that have a destructor.
//!
//! Properties are stored under php-src's mangled names: `"\0A\0x"` for a
//! private `$x` declared in class A, `"\0*\0x"` for a protected one and `"x"`
//! for public and dynamic ones. A parent's private property and a child's
//! property of the same name therefore stay distinct, and `(array)` casts and
//! var_dump come out exactly as in PHP.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

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

#[derive(Debug, Clone)]
pub enum ConstState {
    Unevaluated(Expr),
    Evaluating,
    Done(Value),
    /// An enum case with its backing value expression. Its object is created on
    /// first access, as php-src does, which decides its handle number
    Case(Option<Expr>),
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
    DateFactory,
    Interval,
    IntervalFactory,
    DateTime,
    TimeZone,
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
    ClosureFromCallable,
    ClosureBind,
    ClosureBindTo,
    ClosureCall,
    EnumCases,
    EnumFrom,
    EnumTryFrom,
    GenCurrent,
    GenKey,
    GenNext,
    GenValid,
    GenRewind,
    GenSend,
    GenThrow,
    GenGetReturn,
    /// An interface method of a built-in interface, which classes implement
    Abstract,
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
    pub is_trait: bool,
    pub is_enum: bool,
    /// The backing type of an enum
    pub backing: Option<TypeHint>,
    /// Enum case names in declaration order
    pub cases: Vec<String>,
    pub is_abstract: bool,
    pub is_final: bool,
    pub is_readonly: bool,
    /// Traits used directly by this class, for class_uses()
    pub traits: Vec<&'static str>,
    /// Built-ins like Closure that `new` refuses
    pub internal_only: bool,
    /// A built-in class, whose scope closures cannot be bound to
    pub is_internal: bool,
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
    /// No `__destruct` (left) to call: the class has none, or it already ran
    pub destructed: Cell<bool>,
}

pub enum Internal {
    Interval(RefCell<Option<crate::datetime::interval::Interval>>),
    DateTime(RefCell<Option<crate::datetime::DateTime>>),
    TimeZone(RefCell<Option<Rc<crate::datetime::Zone>>>),
    None,
    Closure(Closure),
    Generator(Box<RefCell<crate::interp::generator::GenState>>),
}

#[derive(Debug, Clone)]
pub struct TraceFrame {
    /// Empty for a call made by a built-in function (`[internal function]`)
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
    /// For a closure made from a callable (`strlen(...)`, Closure::fromCallable),
    /// what it calls. `func` then only describes the parameters
    pub forward: Option<Forward>,
}

/// What a closure made from a callable calls
#[derive(Debug, Clone)]
pub enum Forward {
    /// A user or built-in function, by name
    Function(String),
    /// A method, with the class `static` refers to in it. The object is the closure's `this`
    Method(Rc<Method>, Rc<ClassEntry>),
    /// A method that is missing or not accessible, so `__call` (with `this`) or
    /// `__callStatic` of the class gets its name
    Magic(String, Rc<ClassEntry>),
}

impl std::fmt::Debug for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "object({})#{}", self.class.name, self.id)
    }
}

thread_local! {
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    static FREE_IDS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// Live objects with a pending destructor, indexed by handle
    static DESTRUCTIBLE: RefCell<Vec<Weak<Object>>> = const { RefCell::new(Vec::new()) };
    /// Released objects whose destructor is still to be called
    static PENDING: RefCell<VecDeque<Pending>> = const { RefCell::new(VecDeque::new()) };
    /// Cleared after a fatal error: php-src then calls no more destructors
    static DESTRUCTORS_ENABLED: Cell<bool> = const { Cell::new(true) };
}

pub enum Pending {
    Destruct(Rc<Object>),
    /// A handle freed after the destructors queued before it, as php-src runs
    /// them while releasing the object's contents, before the handle is free
    FreeId(u32),
}

fn free_id(id: u32) {
    // try_with: during thread teardown the free list may already be gone
    let _ = FREE_IDS.try_with(|f| f.borrow_mut().push(id));
}

fn register_destructible(obj: &Rc<Object>) {
    let _ = DESTRUCTIBLE.try_with(|d| {
        let mut d = d.borrow_mut();
        let i = obj.id as usize;
        if d.len() <= i {
            d.resize_with(i + 1, Weak::new);
        }
        d[i] = Rc::downgrade(obj);
    });
}

fn alloc_id() -> u32 {
    FREE_IDS.with(|f| f.borrow_mut().pop()).unwrap_or_else(|| {
        NEXT_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        })
    })
}

/// A handle number taken without an object of its own, like the iterator that
/// php-src creates when a foreach starts on a Traversable. Holding it keeps the
/// handle numbers of other objects as in PHP
pub struct Handle(u32);

impl Handle {
    pub fn new() -> Handle {
        Handle(alloc_id())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        release_id(self.0);
    }
}

/// The next released object whose `__destruct` is due (already marked as
/// destructed), with the rest of the queue taken aside: what its destructor
/// releases comes first, as php-src would release it right there
pub fn take_pending_destruct() -> Option<(Rc<Object>, VecDeque<Pending>)> {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        loop {
            match p.pop_front()? {
                Pending::Destruct(obj) => return Some((obj, std::mem::take(&mut *p))),
                Pending::FreeId(id) => free_id(id),
            }
        }
    })
}

/// Puts back what take_pending_destruct took aside, after anything queued since
pub fn restore_pending(mut rest: VecDeque<Pending>) {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        // What was queued meanwhile is usually short; the rest can be long
        for item in std::mem::take(&mut *p).into_iter().rev() {
            rest.push_front(item);
        }
        *p = rest;
    });
}

/// Live objects whose destructor has not run, in handle order (for shutdown)
pub fn destructible_objects() -> Vec<Rc<Object>> {
    DESTRUCTIBLE.with(|d| d.borrow().iter().filter_map(Weak::upgrade).filter(|o| !o.destructed.get()).collect())
}

/// After a fatal error no destructor runs any more
pub fn disable_destructors() {
    DESTRUCTORS_ENABLED.with(|e| e.set(false));
    let pending = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for p in pending {
        if let Pending::FreeId(id) = p {
            free_id(id);
        }
    }
}

impl Object {
    /// A new object with the class's default property values
    pub fn new(class: Rc<ClassEntry>, internal: Internal) -> Rc<Object> {
        let internal = if matches!(internal, Internal::None) && class.is_subclass_of("DateTimeInterface") {
            Internal::DateTime(RefCell::new(None))
        } else if matches!(internal, Internal::None) && class.is_subclass_of("DateTimeZone") {
            Internal::TimeZone(RefCell::new(None))
        } else if matches!(internal, Internal::None) && class.is_subclass_of("DateInterval") {
            Internal::Interval(RefCell::new(None))
        } else { internal };
        let id = alloc_id();
        let mut props = PhpArray::new();
        for p in &class.props {
            match (&p.default, &p.ty) {
                (Some(v), _) => props.insert(p.key.clone(), v.clone()),
                (None, None) => props.insert(p.key.clone(), Value::Null),
                // Typed without a default: uninitialized, i.e. absent
                (None, Some(_)) => {}
            }
        }
        // A suspended generator may still have finally blocks to run
        let has_destructor = class.find_method("__destruct").is_some() || matches!(internal, Internal::Generator(_));
        let obj = Rc::new(Object { id, class, props: RefCell::new(props), internal, destructed: Cell::new(!has_destructor) });
        if has_destructor {
            register_destructible(&obj);
        }
        obj
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

    pub fn set_key(&self, key: Key, v: Value) {
        if let Ok(mut p) = self.props.try_borrow_mut() {
            p.insert(key, v);
        }
    }

    /// Native debug/serialization properties, followed by user properties.
    /// Native date properties are absent from foreach/get_object_vars.
    pub fn export_props(&self) -> Vec<(Key, Value)> {
        let Some(native) = crate::builtins::date::debug_properties(self) else { return self.ordered_props() };
        let mut out: Vec<_> = native.into_iter().map(|(name, value)| (Key::from_bytes(name.as_bytes()), value)).collect();
        let native_count = out.len();
        for (key, value) in self.ordered_props() {
            if let Some((_, slot)) = out[..native_count].iter_mut().find(|(k, _)| *k == key) { *slot = value; }
            else { out.push((key, value)); }
        }
        out
    }

    /// Initialized properties in slot order: declared ones in class layout order
    /// (even if re-set later), then dynamic ones in creation order
    pub fn ordered_props(&self) -> Vec<(Key, Value)> {
        let Ok(stored) = self.props.try_borrow() else { return Vec::new() };
        let mut out: Vec<(Key, Value)> =
            self.class.props.iter().filter_map(|p| stored.get(&p.key).map(|v| (p.key.clone(), v.clone()))).collect();
        for (k, v) in stored.iter() {
            if !self.class.props.iter().any(|p| &p.key == k) {
                if !matches!(self.internal, Internal::Interval(_)) || crate::builtins::date_interval::visible(self, &unmangle(k).0) {
                    out.push((k.clone(), v.clone()));
                }
            }
        }
        out
    }

    pub fn closure(&self) -> Option<&Closure> {
        match &self.internal {
            Internal::Closure(c) => Some(c),
            Internal::None | Internal::Generator(_) | Internal::DateTime(_) | Internal::TimeZone(_) | Internal::Interval(_) => None,
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
    // Freeing nested objects recurses through here, so the frame is kept small:
    // the rarer work happens out of line
    fn drop(&mut self) {
        if !self.destructed.get() && revive_for_destructor(self) {
            return;
        }
        // Like php-src, the object's contents are released before its handle is,
        // so handles of objects it held are recycled first
        drop(std::mem::take(self.props.get_mut()));
        drop(std::mem::replace(&mut self.internal, Internal::None));
        release_id(self.id);
    }
}

/// Moves a dying object into a fresh Rc under the same handle (which stays
/// taken) and queues it for its destructor. False once destructors are off
#[inline(never)]
fn revive_for_destructor(obj: &mut Object) -> bool {
    if !DESTRUCTORS_ENABLED.try_with(Cell::get).unwrap_or(false) {
        return false;
    }
    if let Internal::Generator(g) = &obj.internal
        && !g.borrow().in_try()
    {
        return false;
    }
    let revived = Rc::new(Object {
        id: obj.id,
        class: obj.class.clone(),
        props: RefCell::new(std::mem::take(obj.props.get_mut())),
        internal: std::mem::replace(&mut obj.internal, Internal::None),
        destructed: Cell::new(true),
    });
    let _ = PENDING.try_with(|p| p.borrow_mut().push_back(Pending::Destruct(revived)));
    true
}

#[inline(never)]
fn release_id(id: u32) {
    let queued = PENDING.try_with(|p| {
        let mut p = p.borrow_mut();
        if p.is_empty() {
            return false;
        }
        p.push_back(Pending::FreeId(id));
        true
    });
    if queued != Ok(true) {
        free_id(id);
    }
}

/// A natively implemented method of an enum: cases(), from(), tryFrom()
pub fn enum_method(name: &str, class: &'static str, body: Native) -> Method {
    Method {
        name: name.to_string(),
        class,
        vis: Visibility::Public,
        is_static: true,
        is_abstract: false,
        is_final: false,
        body: MethodBody::Native(body),
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

fn static_native(name: &str, class: &'static str, body: Native) -> (String, Rc<Method>) {
    let (key, m) = native(name, class, body, false);
    let mut m = Rc::try_unwrap(m).expect("just created");
    m.is_static = true;
    (key, Rc::new(m))
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
    let mut put = |mut c: ClassEntry| {
        c.is_internal = true;
        let c = Rc::new(c);
        map.insert(c.name.to_ascii_lowercase(), c.clone());
        c
    };
    put(ClassEntry { name: "Stringable", is_interface: true, ..Default::default() });
    put(ClassEntry { name: "Throwable", is_interface: true, interfaces: vec!["Stringable"], ..Default::default() });
    put(ClassEntry { name: "stdClass", allow_dynamic: true, ..Default::default() });
    put(ClassEntry { name: "UnitEnum", is_interface: true, ..Default::default() });
    put(ClassEntry { name: "BackedEnum", is_interface: true, interfaces: vec!["UnitEnum"], ..Default::default() });
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
        ("ClosedGeneratorException", "Exception"),
        ("DateError", "Error"),
        ("DateRangeError", "DateError"),
        ("DateException", "Exception"),
        ("DateMalformedStringException", "DateException"),
        ("DateInvalidTimeZoneException", "DateException"),
        ("DateMalformedIntervalStringException", "DateException"),
        ("DateInvalidOperationException", "DateException"),
    ] {
        let c = put(plain_subclass(name, &by_name[parent]));
        by_name.insert(name, c);
    }
    put(ClassEntry { name: "Traversable", is_interface: true, ..Default::default() });
    let abstract_method = |name: &str, class: &'static str| {
        let (key, m) = native(name, class, Native::Abstract, false);
        let mut m = Rc::try_unwrap(m).expect("just created");
        m.is_abstract = true;
        (key, Rc::new(m))
    };
    let iterator_methods = ["current", "next", "key", "valid", "rewind"];
    put(ClassEntry {
        name: "Iterator",
        is_interface: true,
        interfaces: vec!["Traversable"],
        methods: iterator_methods.iter().map(|m| abstract_method(m, "Iterator")).collect(),
        ..Default::default()
    });
    put(ClassEntry {
        name: "IteratorAggregate",
        is_interface: true,
        interfaces: vec!["Traversable"],
        methods: [abstract_method("getIterator", "IteratorAggregate")].into_iter().collect(),
        ..Default::default()
    });
    put(ClassEntry {
        name: "ArrayAccess",
        is_interface: true,
        methods: ["offsetExists", "offsetGet", "offsetSet", "offsetUnset"]
            .iter()
            .map(|m| abstract_method(m, "ArrayAccess"))
            .collect(),
        ..Default::default()
    });
    put(ClassEntry {
        name: "Countable",
        is_interface: true,
        methods: [abstract_method("count", "Countable")].into_iter().collect(),
        ..Default::default()
    });
    put(ClassEntry {
        name: "JsonSerializable",
        is_interface: true,
        methods: [abstract_method("jsonSerialize", "JsonSerializable")].into_iter().collect(),
        ..Default::default()
    });
    let generator_methods = [
        ("current", Native::GenCurrent),
        ("key", Native::GenKey),
        ("next", Native::GenNext),
        ("valid", Native::GenValid),
        ("rewind", Native::GenRewind),
        ("send", Native::GenSend),
        ("throw", Native::GenThrow),
        ("getReturn", Native::GenGetReturn),
    ];
    put(ClassEntry {
        name: "Generator",
        is_final: true,
        interfaces: vec!["Iterator", "Traversable"],
        methods: generator_methods.iter().map(|(m, n)| native(m, "Generator", *n, true)).collect(),
        ..Default::default()
    });
    let methods = [
        native("__invoke", "Closure", Native::ClosureInvoke, false),
        static_native("fromCallable", "Closure", Native::ClosureFromCallable),
        static_native("bind", "Closure", Native::ClosureBind),
        native("bindTo", "Closure", Native::ClosureBindTo, false),
        native("call", "Closure", Native::ClosureCall, false),
    ];
    put(ClassEntry {
        name: "Closure",
        is_final: true,
        internal_only: true,
        methods: methods.into_iter().collect(),
        ..Default::default()
    });
    let date_constants = || crate::builtins::date::class_constants().map(|(name, value)| {
        (name.to_string(), Rc::new(ClassConst { vis: Visibility::Public, class: "DateTimeInterface", state: RefCell::new(ConstState::Done(value)) }))
    }).collect();
    put(ClassEntry { name: "DateTimeInterface", is_interface: true, consts: date_constants(), ..Default::default() });
    for name in ["DateTime", "DateTimeImmutable"] {
        let methods = ["__construct", "format", "modify", "getTimestamp", "setTimestamp", "getTimezone", "setTimezone", "getOffset", "setDate", "setTime", "setISODate", "getMicrosecond", "setMicrosecond", "add", "sub", "diff"];
        let mut methods: HashMap<_, _> = methods.iter().map(|m| native(m, name, Native::DateTime, false)).collect();
        for m in ["createFromFormat", "getLastErrors"] { let (key, method) = static_native(m, name, Native::DateFactory); methods.insert(key, method); }
        put(ClassEntry { name, interfaces: vec!["DateTimeInterface"], consts: date_constants(), methods, ..Default::default() });
    }
    put(ClassEntry { name: "DateTimeZone", methods: ["__construct", "getName", "getOffset"].iter().map(|m| native(m, "DateTimeZone", Native::TimeZone, false)).collect(), ..Default::default() });
    let mut methods: HashMap<_, _> = ["__construct", "format"].iter().map(|m| native(m, "DateInterval", Native::Interval, false)).collect();
    let (key, method) = static_native("createFromDateString", "DateInterval", Native::IntervalFactory);
    methods.insert(key, method);
    put(ClassEntry { name: "DateInterval", methods, ..Default::default() });
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
        Value::Resource(r) => format!("Resource id #{}", r.id),
        Value::Ref(r) => crate::value::with_deref(r, trace_arg),
    }
}

/// `#0 file(line): Class->func(args)` lines followed by `#n {main}`
pub fn trace_string(frames: &[TraceFrame]) -> String {
    let mut out = String::new();
    for (i, f) in frames.iter().enumerate() {
        let args: Vec<String> = f.args.iter().map(trace_arg).collect();
        let class = f.class.as_ref().map_or(String::new(), |(c, t)| format!("{c}{t}"));
        let place = if f.file.is_empty() { "[internal function]".to_string() } else { format!("{}({})", f.file, f.line) };
        out.push_str(&format!("#{i} {place}: {class}{}({})\n", f.function, args.join(", ")));
    }
    out.push_str(&format!("#{} {{main}}", frames.len()));
    out
}

/// getTrace(): one array per frame
pub fn trace_array(frames: &[TraceFrame]) -> Value {
    let rows = frames.iter().map(|f| {
        let mut a = PhpArray::new();
        let mut put = |k: &str, v: Value| a.insert(Key::from_bytes(k.as_bytes()), v);
        if !f.file.is_empty() {
            put("file", Value::str(f.file.clone()));
            put("line", Value::Int(f.line as i64));
        }
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
