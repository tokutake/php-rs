//! PHP's ordered hash map.
//!
//! Layout mirrors php-src's HashTable: entries live in insertion order in a Vec,
//! deletions leave tombstones, and a hash index maps keys to slots. This keeps
//! `unset` O(1) (a shifting remove would make unset-in-a-loop O(n^2)).
//! The index uses std's randomly seeded SipHash, so attacker-chosen keys cannot
//! force collisions (cf. the 2011 hash-flooding DoS against PHP).

use std::collections::HashMap;
use std::rc::Rc;

use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Int(i64),
    Str(Rc<[u8]>),
}

impl Key {
    /// Strings that look like canonical decimal integers become int keys:
    /// "1" -> 1, but "01", "+1", "-0" and " 1" stay strings.
    pub fn from_bytes(s: &[u8]) -> Key {
        if is_canonical_int(s)
            && let Some(i) = std::str::from_utf8(s).ok().and_then(|t| t.parse().ok()) {
                return Key::Int(i);
            }
        Key::Str(s.into())
    }

    pub fn to_value(&self) -> Value {
        match self {
            Key::Int(i) => Value::Int(*i),
            Key::Str(s) => Value::str(s.to_vec()),
        }
    }
}

fn is_canonical_int(s: &[u8]) -> bool {
    let digits = s.strip_prefix(b"-").unwrap_or(s);
    match digits {
        [] => false,
        [b'0'] => s.len() == 1,
        [first, rest @ ..] => (b'1'..=b'9').contains(first) && rest.iter().all(u8::is_ascii_digit),
    }
}

#[derive(Debug, Clone, Default)]
pub struct PhpArray {
    slots: Vec<Option<(Key, Value)>>,
    index: HashMap<Key, usize>,
    /// Next key for `$a[] = ...`; None until an int key has been used
    next_free: Option<i64>,
}

#[derive(Debug)]
pub struct NextElementOccupied;

impl PhpArray {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn get(&self, key: &Key) -> Option<&Value> {
        let &slot = self.index.get(key)?;
        self.slots[slot].as_ref().map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Value> {
        let &slot = self.index.get(key)?;
        self.slots[slot].as_mut().map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &Key) -> bool {
        self.index.contains_key(key)
    }

    /// Returns the value for `key`, inserting null first if it is missing
    pub fn entry(&mut self, key: Key) -> &mut Value {
        let slot = match self.index.get(&key) {
            Some(&slot) => slot,
            None => self.insert_new(key, Value::Null),
        };
        &mut self.slots[slot].as_mut().unwrap().1
    }

    pub fn insert(&mut self, key: Key, value: Value) {
        match self.index.get(&key) {
            Some(&slot) => self.slots[slot].as_mut().unwrap().1 = value,
            None => {
                self.insert_new(key, value);
            }
        }
    }

    fn insert_new(&mut self, key: Key, value: Value) -> usize {
        if let Key::Int(i) = key {
            // On overflow next_free stays saturated, and push() reports it as occupied
            let next = i.saturating_add(1);
            self.next_free = Some(self.next_free.map_or(next, |n| n.max(next)));
        }
        let slot = self.slots.len();
        self.slots.push(Some((key.clone(), value)));
        self.index.insert(key, slot);
        slot
    }

    /// `$a[] = value`. Returns the slot so callers can write through it
    pub fn push(&mut self, value: Value) -> Result<&mut Value, NextElementOccupied> {
        let key = self.next_free.unwrap_or(0);
        if self.contains_key(&Key::Int(key)) {
            return Err(NextElementOccupied);
        }
        let slot = self.insert_new(Key::Int(key), value);
        Ok(&mut self.slots[slot].as_mut().unwrap().1)
    }

    pub fn remove(&mut self, key: &Key) -> Option<Value> {
        let slot = self.index.remove(key)?;
        let (_, value) = self.slots[slot].take().unwrap();
        // Compact once tombstones dominate so iteration stays proportional to len()
        if self.slots.len() > 8 && self.index.len() < self.slots.len() / 2 {
            self.compact();
        }
        Some(value)
    }

    fn compact(&mut self) {
        self.slots.retain(Option::is_some);
        for (slot, entry) in self.slots.iter().enumerate() {
            let (key, _) = entry.as_ref().unwrap();
            *self.index.get_mut(key).unwrap() = slot;
        }
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&Key, &Value)> {
        self.slots.iter().filter_map(|e| e.as_ref().map(|(k, v)| (k, v)))
    }

    /// The first entry at or after storage position `pos`, with the position after
    /// it, for iteration that has to pause. Positions stay valid while the array
    /// is not modified
    pub fn entry_from(&self, pos: usize) -> Option<(usize, &Key, &Value)> {
        self.slots.iter().enumerate().skip(pos).find_map(|(i, e)| e.as_ref().map(|(k, v)| (i + 1, k, v)))
    }

    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.iter().map(|(_, v)| v)
    }

    pub fn pop(&mut self) -> Option<Value> {
        let key = self.iter().next_back()?.0.clone();
        let value = self.remove(&key);
        // Like php-src, array_pop gives back the index only if it was the last one handed out
        if let Key::Int(i) = key
            && self.next_free == i.checked_add(1) {
                self.next_free = Some(i);
            }
        value
    }

    pub fn from_values(values: impl IntoIterator<Item = Value>) -> Self {
        let mut arr = PhpArray::new();
        for v in values {
            // A fresh list cannot run out of indexes before running out of memory
            arr.push(v).expect("list index overflow");
        }
        arr
    }
}

impl FromIterator<(Key, Value)> for PhpArray {
    fn from_iter<I: IntoIterator<Item = (Key, Value)>>(iter: I) -> Self {
        let mut arr = PhpArray::new();
        for (k, v) in iter {
            arr.insert(k, v);
        }
        arr
    }
}
