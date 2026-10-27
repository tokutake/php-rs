//! serialize() and unserialize(), following php-src's var.c and
//! var_unserializer.re: the same format, back-reference numbering, magic
//! methods (__serialize/__unserialize, __sleep/__wakeup), incomplete classes
//! and error offsets.
//!
//! unserialize() works in two passes: the first parses the input into a tree,
//! numbering the values as php-src's var_hash does and noting which of them an
//! `R:` turns into a reference; the second builds the values, so a value that
//! is referenced later can be made a reference from the start.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::rc::Rc;

use super::{ArgVal, EResult, Fatal, Interp};
use crate::array::{Key, PhpArray};
use crate::ast::ClassRef;
use crate::diag;
use crate::object::{unmangle, ClassEntry, Internal, Object, PropInfo, Visibility};
use crate::value::{format_float_repr, Value};

/// The member of __PHP_Incomplete_Class objects naming the missing class
const INCOMPLETE_NAME: &[u8] = b"__PHP_Incomplete_Class_Name";
const INCOMPLETE_CLASS: &str = "__PHP_Incomplete_Class";

/// Nesting beyond this ends in php-src's stack overflow error rather than
/// overflowing the native stack
const NATIVE_DEPTH: usize = 10_000;
/// php-src's unserialize_max_depth default
const DEFAULT_MAX_DEPTH: i64 = 4096;
/// HT_MAX_SIZE on 64-bit
const HT_MAX_SIZE: i64 = 0x4000_0000;

fn stack_overflow() -> Fatal {
    Fatal::error(
        "Maximum call stack size of 8355840 bytes (zend.max_allowed_stack_size - zend.reserved_stack_size) reached. Infinite recursion?",
    )
}

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    let _ = write!(out, "s:{}:\"", s.len());
    out.extend_from_slice(s);
    out.extend_from_slice(b"\";");
}

/// A class name as php-src prints it: anonymous classes end at the NUL
fn display_name(class: &ClassEntry) -> &str {
    class.name.split('\0').next().unwrap_or(class.name)
}

/// Objects whose classes php-src marks ZEND_ACC_NOT_SERIALIZABLE
fn not_serializable(class: &ClassEntry) -> bool {
    matches!(class.name, "Closure" | "Generator" | "InternalIterator") || class.name.contains("@anonymous")
}

struct Ser {
    out: Vec<u8>,
    /// php-src's var_hash counter: one per value written
    n: i64,
    /// Objects and references already written, by address, with their number
    seen: HashMap<usize, i64>,
    depth: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum SlotKind {
    Object,
    Other,
}

/// A parsed value; `slot` is its number for back references
struct Tree {
    slot: usize,
    node: Node,
}

enum Node {
    Scalar(Value),
    Array(Vec<(Key, Tree)>),
    Object {
        class: Rc<ClassEntry>,
        /// For __PHP_Incomplete_Class: the class named in the data
        incomplete: Option<Vec<u8>>,
        /// Properties by storage key, with the declaration of typed ones
        props: Vec<(Key, Tree, Option<PropInfo>)>,
    },
    /// An object of a class with __unserialize, and the array to pass it
    Unserialize(Rc<ClassEntry>, Vec<(Key, Tree)>),
    /// `C:` data of a class without an unserializer: an empty object
    Custom(Rc<ClassEntry>, Option<Vec<u8>>),
    Enum(Value),
    /// `R:`, a reference to an earlier value
    Ref(usize),
    /// `r:`, the same object as an earlier value
    ObjRef(usize),
}

/// Why parsing stopped: bad data (reported with the offset) or an exception
enum Fail {
    Data,
    Throw(Fatal),
}

impl From<Fatal> for Fail {
    fn from(f: Fatal) -> Self {
        Fail::Throw(f)
    }
}

type PResult<T> = Result<T, Fail>;

struct Parser<'a> {
    s: &'a [u8],
    p: usize,
    slots: Vec<SlotKind>,
    /// Slots that an `R:` refers to
    targets: HashSet<usize>,
    depth: i64,
    max_depth: i64,
    /// Lowercased class names allowed; None allows all
    allowed: Option<HashSet<String>>,
}

impl Parser<'_> {
    fn at(&self, i: usize) -> Option<u8> {
        self.s.get(i).copied()
    }

    /// Digits from `i`: their end, or None without any
    fn digits(&self, i: usize) -> Option<usize> {
        let n = self.s[i.min(self.s.len())..].iter().take_while(|c| c.is_ascii_digit()).count();
        (n > 0).then_some(i + n)
    }

    /// An optionally signed integer from `i`
    fn signed(&self, i: usize) -> Option<usize> {
        let i = if matches!(self.at(i), Some(b'+' | b'-')) { i + 1 } else { i };
        self.digits(i)
    }

    /// A number token of `d:` (iv | nv | nvexp) from `i`: its end
    fn number(&self, i: usize) -> Option<usize> {
        let j = if matches!(self.at(i), Some(b'+' | b'-')) { i + 1 } else { i };
        let int_end = self.digits(j);
        let mut end = if self.at(int_end.unwrap_or(j)) == Some(b'.') {
            let after = int_end.unwrap_or(j) + 1;
            match self.digits(after) {
                Some(e) => e,
                None if int_end.is_some() => after,
                None => return None,
            }
        } else {
            int_end?
        };
        if matches!(self.at(end), Some(b'e' | b'E'))
            && let Some(e) = self.signed(end + 1)
        {
            end = e;
        }
        Some(end)
    }

    fn uiv(&self, from: usize, to: usize) -> usize {
        self.s[from..to].iter().fold(0usize, |n, &c| n.wrapping_mul(10).wrapping_add((c - b'0') as usize))
    }

    /// parse_iv2: a signed integer, clamped with a warning when out of range;
    /// the position after its digits
    fn iv(&self, from: usize) -> (i64, usize) {
        let mut i = from;
        let neg = self.at(i) == Some(b'-');
        if matches!(self.at(i), Some(b'+' | b'-')) {
            i += 1;
        }
        while self.at(i) == Some(b'0') {
            i += 1;
        }
        let start = i;
        let mut result: u64 = 0;
        while let Some(c @ b'0'..=b'9') = self.at(i) {
            result = result.wrapping_mul(10).wrapping_add((c - b'0') as u64);
            i += 1;
        }
        if i - start > 19 || result > i64::MAX as u64 + neg as u64 {
            diag::warning("unserialize(): Numerical result out of range");
            return (if neg { i64::MIN } else { i64::MAX }, i);
        }
        (if neg { (result as i64).wrapping_neg() } else { result as i64 }, i)
    }

    fn enter(&mut self) -> PResult<()> {
        if self.max_depth > 0 && self.depth >= self.max_depth {
            diag::warning(format!(
                "unserialize(): Maximum depth of {} exceeded. The depth limit can be changed using the max_depth unserialize() option or the unserialize_max_depth ini setting",
                self.max_depth
            ));
            return Err(Fail::Data);
        }
        if self.depth as usize >= NATIVE_DEPTH {
            return Err(Fail::Throw(stack_overflow()));
        }
        self.depth += 1;
        Ok(())
    }

    /// After an element other than the last, the previous byte must end a value
    fn check_separator(&mut self) -> PResult<()> {
        if !matches!(self.s.get(self.p.wrapping_sub(1)), Some(b';' | b'}')) {
            self.p -= 1;
            return Err(Fail::Data);
        }
        Ok(())
    }

    fn finish_nested(&mut self) -> PResult<()> {
        if self.at(self.p) != Some(b'}') {
            return Err(Fail::Data);
        }
        self.p += 1;
        Ok(())
    }

    /// `"name":` after a length: the name, leaving `p` at the closing quote.
    /// `end` is the byte that must follow the quote
    fn quoted_name(&mut self, start: usize, len_end: usize, end: u8) -> PResult<(usize, usize)> {
        let len = self.uiv(start + 2, len_end);
        let cursor = len_end + 2;
        if self.s.len() - cursor < len || len == 0 {
            self.p = start + 2;
            return Err(Fail::Data);
        }
        let close = cursor + len;
        if self.at(close) != Some(b'"') {
            self.p = close;
            return Err(Fail::Data);
        }
        if self.at(close + 1) != Some(end) {
            self.p = close + 1;
            return Err(Fail::Data);
        }
        Ok((cursor, close))
    }
}

/// zend_is_valid_class_name
fn valid_class_name(s: &[u8]) -> bool {
    s.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80)
}

impl<W: Write> Interp<W> {
    pub(super) fn serialize(&mut self, v: &Value) -> EResult<Value> {
        let mut s = Ser { out: Vec::new(), n: 0, seen: HashMap::new(), depth: 0 };
        let v = v.copy_out();
        self.ser(&mut s, &v)?;
        Ok(Value::str(s.out))
    }

    fn ser(&mut self, s: &mut Ser, v: &Value) -> EResult<()> {
        if s.depth >= NATIVE_DEPTH {
            return Err(stack_overflow());
        }
        // php_add_var_hash: objects and references get numbers to refer back to
        s.n += 1;
        let target = match v {
            Value::Ref(r) => match &*r.borrow() {
                Value::Object(o) => Some(Rc::as_ptr(o) as usize),
                _ => Some(Rc::as_ptr(r) as usize),
            },
            Value::Object(o) => Some(Rc::as_ptr(o) as usize),
            _ => None,
        };
        if let Some(addr) = target {
            if let Some(&id) = s.seen.get(&addr) {
                if id == -1 {
                    s.out.extend_from_slice(b"N;");
                } else if matches!(v, Value::Ref(_)) {
                    // A reference does not take a number of its own
                    s.n -= 1;
                    let _ = write!(s.out, "R:{id};");
                } else {
                    let _ = write!(s.out, "r:{id};");
                }
                return Ok(());
            }
            s.seen.insert(addr, s.n);
        }
        let v = v.copy_out();
        match &v {
            Value::Null => s.out.extend_from_slice(b"N;"),
            Value::Bool(b) => s.out.extend_from_slice(if *b { b"b:1;" } else { b"b:0;" }),
            Value::Int(i) => {
                let _ = write!(s.out, "i:{i};");
            }
            Value::Float(f) => {
                let _ = write!(s.out, "d:{};", format_float_repr(*f));
            }
            Value::Str(st) => put_str(&mut s.out, st),
            Value::Array(a) => {
                let _ = write!(s.out, "a:{}:{{", a.len());
                s.depth += 1;
                let r = self.ser_entries(s, a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(), false);
                s.depth -= 1;
                r?;
                s.out.push(b'}');
            }
            Value::Object(o) => {
                s.depth += 1;
                let r = self.ser_object(s, o);
                s.depth -= 1;
                r?;
            }
            Value::Resource(_) | Value::Ref(_) => s.out.extend_from_slice(b"i:0;"),
        }
        Ok(())
    }

    /// Keys and values of an array or object; object keys are always strings
    fn ser_entries(&mut self, s: &mut Ser, entries: Vec<(Key, Value)>, object: bool) -> EResult<()> {
        for (k, v) in entries {
            match k {
                Key::Int(i) if object => put_str(&mut s.out, i.to_string().as_bytes()),
                Key::Int(i) => {
                    let _ = write!(s.out, "i:{i};");
                }
                Key::Str(k) => put_str(&mut s.out, &k),
            }
            // A reference nobody else holds is just a value
            let v = match &v {
                Value::Ref(r) if Rc::strong_count(r) == 1 => v.copy_out(),
                _ => v,
            };
            self.ser(s, &v)?;
        }
        Ok(())
    }

    fn ser_object(&mut self, s: &mut Ser, o: &Rc<Object>) -> EResult<()> {
        let class = o.class.clone();
        if not_serializable(&class) {
            return Err(Fatal::new("Exception", format!("Serialization of '{}' is not allowed", display_name(&class))));
        }
        let header = |s: &mut Ser, name: &[u8], count: usize| {
            let _ = write!(s.out, "O:{}:\"", name.len());
            s.out.extend_from_slice(name);
            let _ = write!(s.out, "\":{count}:{{");
        };
        if class.is_enum {
            let case = o.get("name").to_bytes();
            let _ = write!(s.out, "E:{}:\"{}:", class.name.len() + 1 + case.len(), class.name);
            s.out.extend_from_slice(&case);
            s.out.extend_from_slice(b"\";");
            return Ok(());
        }
        if class.find_method("__serialize").is_some() {
            let data = self.call_named_method(o, "__serialize")?;
            let Value::Array(data) = data.copy_out() else {
                return Err(Fatal::type_error(format!("{}::__serialize() must return an array", class.name)));
            };
            header(s, class.name.as_bytes(), data.len());
            self.ser_entries(s, data.iter().map(|(k, v)| (k.clone(), v.clone())).collect(), false)?;
            s.out.push(b'}');
            return Ok(());
        }
        if class.name != INCOMPLETE_CLASS && class.find_method("__sleep").is_some() {
            let names = self.call_named_method(o, "__sleep")?;
            let Value::Array(names) = names.copy_out() else {
                diag::warning(format!(
                    "serialize(): {}::__sleep() should return an array only containing the names of instance-variables to serialize",
                    class.name
                ));
                s.out.extend_from_slice(b"N;");
                return Ok(());
            };
            let props = o.export_props();
            let mut picked: Vec<(Key, Value)> = Vec::new();
            for name in names.values() {
                let name = name.copy_out();
                if !matches!(name, Value::Str(_)) {
                    diag::warning(format!(
                        "serialize(): {}::__sleep() should return an array only containing the names of instance-variables to serialize",
                        class.name
                    ));
                }
                let name = self.stringify(&name)?;
                let mut private = vec![0];
                private.extend_from_slice(class.name.as_bytes());
                private.push(0);
                private.extend_from_slice(&name);
                let mut protected = b"\0*\0".to_vec();
                protected.extend_from_slice(&name);
                let mut found = false;
                for key in [name.clone(), private, protected] {
                    let key = Key::Str(key.into());
                    if let Some((_, v)) = props.iter().find(|(k, _)| *k == key) {
                        if picked.iter().any(|(k, _)| *k == key) {
                            diag::warning(format!(
                                "serialize(): \"{}\" is returned from __sleep() multiple times",
                                String::from_utf8_lossy(&name)
                            ));
                        } else {
                            picked.push((key, v.clone()));
                        }
                        found = true;
                        break;
                    }
                    // An uninitialized typed property is left out quietly
                    if class.props.iter().any(|p| p.key == key && p.ty.is_some()) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    diag::warning(format!(
                        "serialize(): \"{}\" returned as member variable from __sleep() but does not exist",
                        String::from_utf8_lossy(&name)
                    ));
                }
            }
            header(s, class.name.as_bytes(), picked.len());
            self.ser_entries(s, picked, true)?;
            s.out.push(b'}');
            return Ok(());
        }
        let mut props = o.export_props();
        let name = if class.name == INCOMPLETE_CLASS {
            let magic = Key::Str(INCOMPLETE_NAME.into());
            match props.iter().position(|(k, _)| *k == magic) {
                Some(i) => {
                    let name = props.remove(i).1.copy_out();
                    match name {
                        Value::Str(n) => n.to_vec(),
                        _ => class.name.as_bytes().to_vec(),
                    }
                }
                None => class.name.as_bytes().to_vec(),
            }
        } else {
            class.name.as_bytes().to_vec()
        };
        header(s, &name, props.len());
        self.ser_entries(s, props, true)?;
        s.out.push(b'}');
        Ok(())
    }

    pub(super) fn unserialize(&mut self, data: &[u8], options: Option<&PhpArray>) -> EResult<Value> {
        let mut allowed = None;
        let mut max_depth = DEFAULT_MAX_DEPTH;
        if let Some(options) = options {
            if let Some(classes) = options.get(&Key::Str((*b"allowed_classes").into())).map(Value::copy_out) {
                match classes {
                    Value::Bool(true) => {}
                    Value::Bool(false) => allowed = Some(HashSet::new()),
                    Value::Array(list) => {
                        let mut set = HashSet::new();
                        for entry in list.values() {
                            let entry = entry.copy_out();
                            if !matches!(entry, Value::Str(_) | Value::Object(_)) {
                                return Err(Fatal::type_error(format!(
                                    "unserialize(): Option \"allowed_classes\" must be an array of class names, {} given",
                                    entry.value_name()
                                )));
                            }
                            let name = self.stringify(&entry)?;
                            if !valid_class_name(&name) || name.is_empty() {
                                return Err(Fatal::value_error(format!(
                                    "unserialize(): Option \"allowed_classes\" must be an array of class names, \"{}\" given",
                                    String::from_utf8_lossy(&name)
                                )));
                            }
                            set.insert(String::from_utf8_lossy(&name).to_ascii_lowercase());
                        }
                        allowed = Some(set);
                    }
                    other => {
                        return Err(Fatal::type_error(format!(
                            "unserialize(): Option \"allowed_classes\" must be of type array|bool, {} given",
                            other.value_name()
                        )));
                    }
                }
            }
            if let Some(depth) = options.get(&Key::Str((*b"max_depth").into())).map(Value::copy_out) {
                let Value::Int(depth) = depth else {
                    return Err(Fatal::type_error(format!(
                        "unserialize(): Option \"max_depth\" must be of type int, {} given",
                        depth.value_name()
                    )));
                };
                if depth < 0 {
                    return Err(Fatal::value_error(
                        "unserialize(): Option \"max_depth\" must be greater than or equal to 0",
                    ));
                }
                max_depth = depth;
            }
        }
        if data.is_empty() {
            return Ok(Value::Bool(false));
        }
        let mut p = Parser { s: data, p: 0, slots: Vec::new(), targets: HashSet::new(), depth: 0, max_depth, allowed };
        let tree = match self.parse(&mut p, true) {
            Ok(t) => t,
            Err(Fail::Data) => {
                diag::warning(format!("unserialize(): Error at offset {} of {} bytes", p.p, data.len()));
                return Ok(Value::Bool(false));
            }
            Err(Fail::Throw(e)) => return Err(e),
        };
        if p.p < data.len() {
            diag::warning(format!("unserialize(): Extra data starting at offset {} of {} bytes", p.p, data.len()));
        }
        let mut b = Builder { targets: p.targets, cells: HashMap::new(), objects: HashMap::new(), pending: Vec::new() };
        let v = self.build(&mut b, &tree)?;
        // Delayed __wakeup and __unserialize calls, in the order the objects ended
        let mut pending = b.pending.into_iter();
        while let Some(call) = pending.next() {
            let result = match &call {
                Pending::Wakeup(o) => self.call_named_method(o, "__wakeup").map(|_| ()),
                Pending::Unserialize(o, data) => self.call_unserialize(o, data.clone()),
            };
            if let Err(e) = result {
                // The objects left behind are not destructed
                for call in std::iter::once(call).chain(pending) {
                    let (Pending::Wakeup(o) | Pending::Unserialize(o, _)) = call;
                    o.destructed.set(true);
                }
                return Err(e);
            }
        }
        Ok(v.copy_out())
    }

    fn call_unserialize(&mut self, o: &Rc<Object>, data: Value) -> EResult<()> {
        let Some(m) = o.class.find_method("__unserialize").cloned() else { return Ok(()) };
        let line = diag::line();
        let r = self.call_method(Some(o.clone()), &m, vec![ArgVal::Val(data)], line, o.class.clone());
        diag::set_line(line);
        r.map(|_| ())
    }

    /// php_var_unserialize_internal. `values` is false for array keys, which
    /// take no slot and cannot be compound
    fn parse(&mut self, p: &mut Parser, values: bool) -> PResult<Tree> {
        let start = p.p;
        let Some(c) = p.at(start) else { return Err(Fail::Data) };
        let slot = p.slots.len();
        if values && c != b'R' {
            p.slots.push(SlotKind::Other);
        }
        let tree = |node| Ok(Tree { slot, node });
        let colon = p.at(start + 1) == Some(b':');
        match c {
            b'R' | b'r' if colon => {
                let Some(end) = p.digits(start + 2).filter(|&e| p.at(e) == Some(b';')) else { return Err(Fail::Data) };
                p.p = end + 1;
                if !values {
                    return Err(Fail::Data);
                }
                let id = p.uiv(start + 2, end).wrapping_sub(1);
                if id >= p.slots.len() || (c == b'R' && id == usize::MAX) {
                    return Err(Fail::Data);
                }
                if c == b'R' {
                    p.targets.insert(id);
                    return Ok(Tree { slot: usize::MAX, node: Node::Ref(id) });
                }
                if p.slots[id] != SlotKind::Object {
                    return Err(Fail::Data);
                }
                p.slots[slot] = SlotKind::Object;
                tree(Node::ObjRef(id))
            }
            b'N' if p.at(start + 1) == Some(b';') => {
                p.p = start + 2;
                tree(Node::Scalar(Value::Null))
            }
            b'b' if colon && matches!(p.at(start + 2), Some(b'0' | b'1')) && p.at(start + 3) == Some(b';') => {
                p.p = start + 4;
                tree(Node::Scalar(Value::Bool(p.at(start + 2) == Some(b'1'))))
            }
            b'i' if colon => {
                let Some(end) = p.signed(start + 2).filter(|&e| p.at(e) == Some(b';')) else { return Err(Fail::Data) };
                p.p = end + 1;
                tree(Node::Scalar(Value::Int(p.iv(start + 2).0)))
            }
            b'd' if colon => {
                let rest = &p.s[start + 2..];
                for (word, f) in [(&b"NAN;"[..], f64::NAN), (b"INF;", f64::INFINITY), (b"-INF;", f64::NEG_INFINITY)] {
                    if rest.starts_with(word) {
                        p.p = start + 2 + word.len();
                        return tree(Node::Scalar(Value::Float(f)));
                    }
                }
                let Some(end) = p.number(start + 2).filter(|&e| p.at(e) == Some(b';')) else { return Err(Fail::Data) };
                p.p = end + 1;
                let text = std::str::from_utf8(&p.s[start + 2..end]).unwrap_or("0");
                let text = text.strip_prefix('+').unwrap_or(text);
                tree(Node::Scalar(Value::Float(text.parse().unwrap_or(0.0))))
            }
            b's' | b'S' if colon => {
                let Some(len_end) = p.digits(start + 2).filter(|&e| p.at(e) == Some(b':') && p.at(e + 1) == Some(b'"'))
                else {
                    return Err(Fail::Data);
                };
                let len = p.uiv(start + 2, len_end);
                let cursor = len_end + 2;
                if p.s.len() - cursor < len {
                    p.p = start + 2;
                    return Err(Fail::Data);
                }
                let (bytes, end) = if c == b's' {
                    (p.s[cursor..cursor + len].to_vec(), cursor + len)
                } else {
                    // The escaped form: `\` and two hex digits stand for a byte
                    let mut out = Vec::with_capacity(len);
                    let mut i = cursor;
                    let hex = |c: Option<u8>| c.and_then(|c| (c as char).to_digit(16));
                    for _ in 0..len {
                        match p.at(i) {
                            None => return Err(Fail::Data),
                            Some(b'\\') => match (hex(p.at(i + 1)), hex(p.at(i + 2))) {
                                (Some(h), Some(l)) => {
                                    out.push((h * 16 + l) as u8);
                                    i += 3;
                                }
                                _ => return Err(Fail::Data),
                            },
                            Some(b) => {
                                out.push(b);
                                i += 1;
                            }
                        }
                    }
                    (out, i)
                };
                if p.at(end) != Some(b'"') {
                    p.p = end;
                    return Err(Fail::Data);
                }
                if p.at(end + 1) != Some(b';') {
                    p.p = end + 1;
                    return Err(Fail::Data);
                }
                p.p = end + 2;
                if c == b'S' {
                    diag::deprecated("unserialize(): Unserializing the 'S' format is deprecated");
                }
                tree(Node::Scalar(Value::str(bytes)))
            }
            b'a' if colon => {
                let Some(len_end) = p.digits(start + 2).filter(|&e| p.at(e) == Some(b':') && p.at(e + 1) == Some(b'{'))
                else {
                    return Err(Fail::Data);
                };
                let elements = p.iv(start + 2).0;
                p.p = len_end + 2;
                if !values || !(0..HT_MAX_SIZE).contains(&elements) || elements as usize > (p.s.len() - p.p) / 2 {
                    return Err(Fail::Data);
                }
                if elements == 0 {
                    p.finish_nested()?;
                    return tree(Node::Array(Vec::new()));
                }
                let items = self.parse_array_items(p, elements)?;
                p.finish_nested()?;
                tree(Node::Array(items))
            }
            b'O' | b'C' if colon => {
                let Some(len_end) = p.digits(start + 2).filter(|&e| p.at(e) == Some(b':') && p.at(e + 1) == Some(b'"'))
                else {
                    return Err(Fail::Data);
                };
                if !values {
                    return Err(Fail::Data);
                }
                let (name_start, close) = p.quoted_name(start, len_end, b':')?;
                let name = &p.s[name_start..close];
                if name[0] == 0 || name[0] == b'\\' {
                    return Err(Fail::Data);
                }
                let name = name.to_vec();
                let lname = String::from_utf8_lossy(&name).to_ascii_lowercase();
                let allowed = p.allowed.as_ref().is_none_or(|a| a.contains(&lname));
                let class = if allowed && valid_class_name(&name) {
                    self.find_class(&String::from_utf8_lossy(&name))?
                } else if allowed {
                    self.classes.get(&lname).cloned()
                } else {
                    None
                };
                if class.is_none() && !valid_class_name(&name) {
                    return Err(Fail::Data);
                }
                let incomplete = class.is_none().then_some(name);
                let class = match class {
                    Some(c) => c,
                    None => self.classes.get(&INCOMPLETE_CLASS.to_ascii_lowercase()).cloned().expect("built-in class"),
                };
                p.p = close;
                if not_serializable(&class) {
                    return Err(Fail::Throw(Fatal::new(
                        "Exception",
                        format!("Unserialization of '{}' is not allowed", display_name(&class)),
                    )));
                }
                if c == b'C' {
                    // object_custom: no class here has an unserializer of its own
                    let (datalen, after) = p.iv(close + 2);
                    p.p = after;
                    if p.s.len() - p.p < 2 || p.at(p.p) != Some(b':') {
                        return Err(Fail::Data);
                    }
                    if p.at(p.p + 1) != Some(b'{') {
                        p.p += 1;
                        return Err(Fail::Data);
                    }
                    p.p += 2;
                    let present = (p.s.len() - p.p) as i64;
                    if datalen < 0 || present <= datalen {
                        diag::warning(format!(
                            "Insufficient data for unserializing - {datalen} required, {present} present"
                        ));
                        return Err(Fail::Data);
                    }
                    if p.at(p.p + datalen as usize) != Some(b'}') {
                        p.p += datalen as usize;
                        return Err(Fail::Data);
                    }
                    diag::warning(format!("Class {} has no unserializer", class.name));
                    p.p += datalen as usize + 1;
                    p.slots[slot] = SlotKind::Object;
                    return tree(Node::Custom(class, incomplete));
                }
                if p.p + 2 >= p.s.len() {
                    diag::warning("Bad unserialize data");
                    return Err(Fail::Data);
                }
                let (elements, after) = p.iv(p.p + 2);
                let remaining = p.s.len() - close;
                p.p = after;
                if elements < 0 || elements as usize > remaining / 2 {
                    return Err(Fail::Data);
                }
                if p.at(p.p) != Some(b':') {
                    return Err(Fail::Data);
                }
                if p.at(p.p + 1) != Some(b'{') {
                    p.p += 1;
                    return Err(Fail::Data);
                }
                p.p += 2;
                for (cond, what) in [
                    (class.is_interface, "interface"),
                    (class.is_trait, "trait"),
                    (class.is_enum, "enum"),
                    (class.is_abstract, "abstract class"),
                ] {
                    if cond {
                        return Err(Fail::Throw(Fatal::error(format!("Cannot instantiate {what} {}", class.name))));
                    }
                }
                p.slots[slot] = SlotKind::Object;
                if incomplete.is_none() && class.find_method("__unserialize").is_some() {
                    if elements >= HT_MAX_SIZE {
                        return Err(Fail::Data);
                    }
                    let items = self.parse_array_items(p, elements)?;
                    p.finish_nested()?;
                    return tree(Node::Unserialize(class, items));
                }
                let props = self.parse_props(p, &class, elements)?;
                p.finish_nested()?;
                tree(Node::Object { class, incomplete, props })
            }
            b'E' if colon => {
                let Some(len_end) = p.digits(start + 2).filter(|&e| p.at(e) == Some(b':') && p.at(e + 1) == Some(b'"'))
                else {
                    return Err(Fail::Data);
                };
                if !values {
                    return Err(Fail::Data);
                }
                let (name_start, close) = p.quoted_name(start, len_end, b';')?;
                let full = &p.s[name_start..close];
                let Some(colon_at) = full.iter().position(|&c| c == b':') else {
                    diag::warning(format!(
                        "unserialize(): Invalid enum name '{}' (missing colon)",
                        String::from_utf8_lossy(full)
                    ));
                    return Err(Fail::Data);
                };
                let enum_name = String::from_utf8_lossy(&full[..colon_at]).into_owned();
                let case = String::from_utf8_lossy(&full[colon_at + 1..]).into_owned();
                if !valid_class_name(enum_name.as_bytes()) || enum_name.is_empty() {
                    return Err(Fail::Data);
                }
                let Some(class) = self.find_class(&enum_name)? else {
                    diag::warning(format!("unserialize(): Class '{enum_name}' not found"));
                    return Err(Fail::Data);
                };
                if !class.is_enum {
                    diag::warning(format!("unserialize(): Class '{enum_name}' is not an enum"));
                    return Err(Fail::Data);
                }
                p.p = close + 2;
                if !class.consts.contains_key(&case) {
                    diag::warning(format!("unserialize(): Undefined constant {enum_name}::{case}"));
                    return Err(Fail::Data);
                }
                if !class.cases.contains(&case) {
                    diag::warning(format!("unserialize(): {enum_name}::{case} is not an enum case"));
                    return Err(Fail::Data);
                }
                let v = self.class_const(&ClassRef::Named(class.name.to_string()), &case)?;
                p.slots[slot] = SlotKind::Object;
                tree(Node::Enum(v))
            }
            b'}' => {
                diag::warning("unserialize(): Unexpected end of serialized data");
                Err(Fail::Data)
            }
            _ => Err(Fail::Data),
        }
    }

    /// process_nested_array_data
    fn parse_array_items(&mut self, p: &mut Parser, elements: i64) -> PResult<Vec<(Key, Tree)>> {
        p.enter()?;
        let mut items: Vec<(Key, Tree)> = Vec::new();
        let result = (|| {
            for left in (0..elements).rev() {
                let key = match self.parse(p, false)?.node {
                    Node::Scalar(Value::Int(i)) => Key::Int(i),
                    Node::Scalar(Value::Str(s)) => Key::from_bytes(&s),
                    _ => return Err(Fail::Data),
                };
                let value = self.parse(p, true)?;
                // A repeated key replaces the earlier value
                match items.iter().position(|(k, _)| *k == key) {
                    Some(i) => items[i].1 = value,
                    None => items.push((key, value)),
                }
                if left > 0 {
                    p.check_separator()?;
                }
            }
            Ok(())
        })();
        p.depth -= 1;
        result.map(|_| items)
    }

    /// process_nested_object_data: keys become property storage keys, with
    /// php-src's handling of changed visibility and dynamic properties
    fn parse_props(&mut self, p: &mut Parser, class: &Rc<ClassEntry>, elements: i64) -> PResult<Vec<(Key, Tree, Option<PropInfo>)>> {
        p.enter()?;
        let mut props: Vec<(Key, Tree, Option<PropInfo>)> = Vec::new();
        let result = (|| {
            for left in (0..elements).rev() {
                let key: Vec<u8> = match self.parse(p, false)?.node {
                    Node::Scalar(Value::Int(i)) => i.to_string().into_bytes(),
                    Node::Scalar(Value::Str(s)) => s.to_vec(),
                    _ => return Err(Fail::Data),
                };
                let mut key = Key::Str(key.into());
                let mut info = class.props.iter().find(|p| p.key == key).cloned();
                if info.is_none() && !props.iter().any(|(k, _, _)| *k == key) {
                    match visibility_changed(class, &key) {
                        Err(()) => {
                            diag::notice("Illegal member variable name");
                            return Err(Fail::Data);
                        }
                        Ok(Some(p)) => {
                            key = p.key.clone();
                            info = Some(p);
                        }
                        Ok(None) => {
                            let name = unmangle(&key).0;
                            if class.is_readonly {
                                return Err(Fail::Throw(Fatal::error(format!(
                                    "Cannot create dynamic property {}::${name}",
                                    class.name
                                ))));
                            }
                            if !class.allow_dynamic && !native_state(class).contains(&name.as_str()) {
                                diag::deprecated(format!("Creation of dynamic property {}::${name} is deprecated", class.name));
                            }
                        }
                    }
                }
                let value = self.parse(p, true)?;
                let info = info.filter(|i| i.ty.is_some());
                match props.iter().position(|(k, _, _)| *k == key) {
                    Some(i) => props[i] = (key, value, info),
                    None => props.push((key, value, info)),
                }
                if left > 0 {
                    p.check_separator()?;
                }
            }
            Ok(())
        })();
        p.depth -= 1;
        result.map(|_| props)
    }

    fn build(&mut self, b: &mut Builder, t: &Tree) -> EResult<Value> {
        // A value an `R:` refers to is a reference from the start
        let cell = b.targets.contains(&t.slot).then(|| b.cell(t.slot));
        let v = match &t.node {
            Node::Scalar(v) | Node::Enum(v) => v.clone(),
            Node::Ref(id) => return Ok(Value::Ref(b.cell(*id))),
            Node::ObjRef(id) => b.objects.get(id).cloned().map(Value::Object).unwrap_or(Value::Null),
            Node::Array(items) => {
                let mut arr = PhpArray::new();
                for (k, item) in items {
                    let v = self.build(b, item)?;
                    arr.insert(k.clone(), v);
                }
                Value::Array(Rc::new(arr))
            }
            Node::Unserialize(class, items) => {
                let obj = Object::new(class.clone(), Internal::None);
                b.objects.insert(t.slot, obj.clone());
                let mut arr = PhpArray::new();
                for (k, item) in items {
                    let v = self.build(b, item)?;
                    arr.insert(k.clone(), v);
                }
                b.pending.push(Pending::Unserialize(obj.clone(), Value::Array(Rc::new(arr))));
                Value::Object(obj)
            }
            Node::Custom(class, incomplete) => {
                let obj = Object::new(class.clone(), Internal::None);
                if let Some(name) = incomplete {
                    obj.set_key(Key::Str(INCOMPLETE_NAME.into()), Value::str(name.clone()));
                }
                b.objects.insert(t.slot, obj.clone());
                Value::Object(obj)
            }
            Node::Object { class, incomplete, props } => {
                let obj = Object::new(class.clone(), Internal::None);
                if let Some(name) = incomplete {
                    obj.set_key(Key::Str(INCOMPLETE_NAME.into()), Value::str(name.clone()));
                }
                b.objects.insert(t.slot, obj.clone());
                let state_names = crate::builtins::date::state_properties(&obj);
                let mut state = Vec::new();
                for (key, item, info) in props {
                    let mut v = self.build(b, item)?;
                    if let Key::Str(k) = key
                        && let Some(name) = state_names.iter().find(|n| n.as_bytes() == &k[..])
                    {
                        state.push((*name, v));
                        continue;
                    }
                    if let Some(info) = info
                        && let Some(ty) = &info.ty
                    {
                        let err = |v: &Value, _: &str| {
                            Fatal::type_error(format!(
                                "Cannot assign {} to property {}::${} of type {}",
                                v.value_name(),
                                info.class,
                                info.name,
                                ty.display
                            ))
                        };
                        match &v {
                            Value::Ref(r) => {
                                let inner = r.borrow().clone();
                                let checked = self.check_type(inner, Some(ty), &err, true)?;
                                *r.borrow_mut() = checked;
                            }
                            _ => v = self.check_type(v, Some(ty), &err, true)?,
                        }
                    }
                    obj.props.borrow_mut().insert(key.clone(), v);
                }
                if !state_names.is_empty() {
                    crate::builtins::date::restore(&obj, &state)?;
                }
                if class.name != INCOMPLETE_CLASS && class.find_method("__wakeup").is_some() {
                    b.pending.push(Pending::Wakeup(obj.clone()));
                }
                Value::Object(obj)
            }
        };
        if let Value::Object(o) = &v {
            b.objects.entry(t.slot).or_insert_with(|| o.clone());
        }
        Ok(match cell {
            Some(cell) => {
                *cell.borrow_mut() = v;
                Value::Ref(cell)
            }
            None => v,
        })
    }
}

/// Properties that carry the native state of date objects, not dynamic ones
fn native_state(class: &ClassEntry) -> &'static [&'static str] {
    if class.is_subclass_of("DateTimeInterface") {
        &["date", "timezone_type", "timezone"]
    } else if class.is_subclass_of("DateTimeZone") {
        &["timezone_type", "timezone"]
    } else {
        &[]
    }
}

/// is_property_visibility_changed: a key naming a declared property with
/// another visibility is stored under that declaration. Err for a malformed
/// mangled name
fn visibility_changed(class: &ClassEntry, key: &Key) -> Result<Option<PropInfo>, ()> {
    if class.props.is_empty() {
        return Ok(None);
    }
    let Key::Str(k) = key else { return Ok(None) };
    let (scope, name): (Option<&[u8]>, &[u8]) = if k.first() == Some(&0) {
        if k.len() < 3 || k[1] == 0 {
            return Err(());
        }
        let Some(sep) = k[1..].iter().position(|&c| c == 0) else { return Err(()) };
        (Some(&k[1..1 + sep]), &k[2 + sep..])
    } else {
        (None, k)
    };
    if let Some(scope) = scope
        && scope != b"*"
        && !scope.eq_ignore_ascii_case(class.name.as_bytes())
    {
        return Ok(None);
    }
    let named = |p: &&PropInfo| p.name.as_bytes() == name;
    let visible = class.props.iter().filter(named).find(|p| p.vis != Visibility::Private || p.class == class.name);
    Ok(visible.cloned())
}

enum Pending {
    Wakeup(Rc<Object>),
    Unserialize(Rc<Object>, Value),
}

struct Builder {
    targets: HashSet<usize>,
    cells: HashMap<usize, Rc<RefCell<Value>>>,
    /// The objects made so far, by slot, for `r:`
    objects: HashMap<usize, Rc<Object>>,
    pending: Vec<Pending>,
}

impl Builder {
    fn cell(&mut self, slot: usize) -> Rc<RefCell<Value>> {
        self.cells.entry(slot).or_insert_with(|| Rc::new(RefCell::new(Value::Null))).clone()
    }
}
