//! Type sources attached to aliased properties. Weak owners detach constraints
//! when a property is rebound, unset, or its object is released.
use super::*;
use std::rc::Weak;
#[derive(Clone)]
pub(super) enum SourceOwner {
    Static(Weak<RefCell<Value>>),
    Property(Weak<Object>, Key),
}
#[derive(Clone)]
pub(super) struct RefSource {
    reference: Weak<RefCell<Value>>,
    owner: SourceOwner,
    ty: TypeHint,
    label: String,
    scope: Rc<ClassEntry>,
}
impl RefSource {
    fn active(&self) -> bool {
        let Some(reference) = self.reference.upgrade() else {
            return false;
        };
        let same = |v: &Value| matches!(v,Value::Ref(r) if Rc::ptr_eq(r,&reference));
        match &self.owner {
            SourceOwner::Static(cell) => cell
                .upgrade()
                .is_some_and(|cell| cell.try_borrow().ok().is_some_and(|v| same(&v))),
            SourceOwner::Property(object, key) => object.upgrade().is_some_and(|object| {
                object
                    .props
                    .try_borrow()
                    .ok()
                    .is_some_and(|props| props.get(key).is_some_and(same))
            }),
        }
    }
}
pub(super) fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
        (Value::Str(a), Value::Str(b)) => Rc::ptr_eq(a, b) || (a == b),
        (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
        (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
        (Value::Resource(a), Value::Resource(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}
impl<W: Write> Interp<W> {
    pub(super) fn reference_source(
        &self,
        path: &Path<'_>,
        acquire: bool,
    ) -> EResult<Option<RefSource>> {
        if !path.dims.is_empty() {
            return Ok(None);
        }
        let (owner, ty, label, class, value) = match &path.base {
            Base::Cell(cell, Some((ty, label))) => {
                let class = label.split_once("::$").map(|(c, _)| c).unwrap_or("");
                (
                    SourceOwner::Static(Rc::downgrade(cell)),
                    ty.clone(),
                    label.clone(),
                    class.to_string(),
                    Some(value::with_deref(cell, Value::copy_out)),
                )
            }
            Base::Prop(object, name) => {
                let oop::PropLookup::Declared(p) = self.find_prop(&object.class, name) else {
                    return Ok(None);
                };
                if p.readonly && matches!(object.internal, Internal::Period(_)) {
                    return Err(Fatal::error(format!(
                        "Cannot modify readonly property {}::${name}",
                        p.class
                    )));
                }
                if p.readonly {
                    return Err(Fatal::error(format!(
                        "Cannot acquire reference to readonly property {}::${name}",
                        p.class
                    )));
                }
                let Some(ty) = p.ty else {
                    return Ok(None);
                };
                let value = object.props.borrow().get(&p.key).map(Value::copy_out);
                (
                    SourceOwner::Property(Rc::downgrade(object), p.key),
                    ty,
                    format!("{}::${name}", p.class),
                    p.class.to_string(),
                    value,
                )
            }
            _ => return Ok(None),
        };
        if acquire && value.as_ref().is_none_or(|v| matches!(v, Value::Null)) && !ty.allows("null")
        {
            return Err(Fatal::error(format!(
                "Cannot access uninitialized non-nullable property {label} by reference"
            )));
        }
        let scope = self
            .classes
            .get(&class.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| Fatal::error("Reference property scope is unavailable"))?;
        Ok(Some(RefSource {
            reference: Weak::new(),
            owner,
            ty,
            label,
            scope,
        }))
    }
    pub(super) fn register_reference(&mut self, reference: &RefCellValue, mut source: RefSource) {
        source.reference = Rc::downgrade(reference);
        self.reference_sources.retain(RefSource::active);
        if !self.reference_sources.iter().any(|s| {
            s.label == source.label
                && s.reference.ptr_eq(&source.reference)
                && match (&s.owner, &source.owner) {
                    (SourceOwner::Static(a), SourceOwner::Static(b)) => a.ptr_eq(b),
                    (SourceOwner::Property(a, ka), SourceOwner::Property(b, kb)) => {
                        a.ptr_eq(b) && ka == kb
                    }
                    _ => false,
                }
        }) {
            self.reference_sources.push(source);
        }
    }
    pub(super) fn reference_snapshots(&mut self) -> Vec<(Weak<RefCell<Value>>, Value)> {
        self.reference_sources.retain(RefSource::active);
        let mut references = HashSet::new();
        self.reference_sources
            .iter()
            .filter_map(|s| s.reference.upgrade())
            .filter_map(|r| {
                if references.insert(Rc::as_ptr(&r)) {
                    let value = value::with_deref(&r, Value::clone);
                    Some((Rc::downgrade(&r), value))
                } else {
                    None
                }
            })
            .collect()
    }
    pub(super) fn validate_references(
        &mut self,
        snapshots: &[(Weak<RefCell<Value>>, Value)],
    ) -> EResult<Option<(Value, Value)>> {
        let mut converted = None;
        for (reference, before) in snapshots {
            let Some(reference) = reference.upgrade() else {
                continue;
            };
            let candidate = value::with_deref(&reference, Value::clone);
            if same_value(before, &candidate) {
                continue;
            }
            let sources: Vec<_> = self
                .reference_sources
                .iter()
                .filter(|s| {
                    s.active()
                        && s.reference
                            .upgrade()
                            .is_some_and(|r| Rc::ptr_eq(&r, &reference))
                })
                .cloned()
                .collect();
            let mut accepted = None;
            for source in sources {
                let error = |v: &Value, _: &str| {
                    Fatal::type_error(format!(
                        "Cannot assign {} to reference held by property {} of type {}",
                        v.value_name(),
                        source.label,
                        source.ty.display
                    ))
                };
                let value = self.with_scope(source.scope.clone(), |this| {
                    this.check_param(candidate.clone(), Some(&source.ty), &error)
                });
                let value = match value {
                    Ok(value) => value,
                    Err(error) => {
                        *reference.borrow_mut() = before.clone();
                        return Err(error);
                    }
                };
                if accepted.as_ref().is_some_and(|v| !same_value(v, &value)) {
                    *reference.borrow_mut() = before.clone();
                    return Err(Fatal::type_error(format!(
                        "Cannot assign {} to reference held by property {} of type {}",
                        candidate.value_name(),
                        source.label,
                        source.ty.display
                    )));
                }
                accepted = Some(value);
            }
            if let Some(value) = accepted
                && !same_value(&candidate, &value)
            {
                *reference.borrow_mut() = value.clone();
                converted = Some((candidate, value));
            }
        }
        Ok(converted)
    }
    pub(super) fn check_reference_binding(
        &mut self,
        reference: &RefCellValue,
        source: &RefSource,
    ) -> EResult<()> {
        let candidate = value::with_deref(&reference, Value::clone);
        let error = |v: &Value, _: &str| {
            Fatal::type_error(format!(
                "Cannot assign {} to property {} of type {}",
                v.value_name(),
                source.label,
                source.ty.display
            ))
        };
        let value = self.with_scope(source.scope.clone(), |this| {
            this.check_param(candidate.clone(), Some(&source.ty), &error)
        })?;
        let sources: Vec<_> = self
            .reference_sources
            .iter()
            .filter(|s| {
                s.active()
                    && s.reference
                        .upgrade()
                        .is_some_and(|r| Rc::ptr_eq(&r, reference))
            })
            .cloned()
            .collect();
        for held in sources {
            let error = |v: &Value, _: &str| {
                Fatal::type_error(format!(
                    "Cannot assign {} to reference held by property {} of type {}",
                    v.value_name(),
                    held.label,
                    held.ty.display
                ))
            };
            let accepted = self.with_scope(held.scope.clone(), |this| {
                this.check_param(candidate.clone(), Some(&held.ty), &error)
            })?;
            if !same_value(&accepted, &value) {
                return Err(Fatal::type_error(format!(
                    "Reference with value of type {} held by property {} of type {} is not compatible with property {} of type {}",
                    candidate.value_name(),
                    held.label,
                    held.ty.display,
                    source.label,
                    source.ty.display
                )));
            }
        }
        *reference.borrow_mut() = value;
        Ok(())
    }
}
