//! Evaluate unpacked/named native arguments once, in source order, retaining
//! storage cells for reference parameters before putting them in call order.
use super::*;

impl<W: Write> Interp<W> {
    pub(super) fn read_argument_path(&mut self, path: &Path<'_>) -> EResult<Value> {
        let mut value = match &path.base {
            Base::Var(name) => self.lookup(name),
            Base::Global(name) => self.frames[0]
                .vars
                .get(name)
                .map(Value::copy_out)
                .unwrap_or(Value::Null),
            Base::Prop(object, name) => self.read_prop(object, name)?,
            Base::Cell(cell, _) => value::with_deref(cell, Value::copy_out),
            Base::Offset(object, dim) => self.offset_get(object, dim.as_ref())?,
        };
        for dim in &path.dims {
            let Some(dim) = dim else {
                return Err(Fatal::error("Cannot use [] for reading"));
            };
            value = if let Value::Object(object) = value {
                self.offset_get(&object, Some(dim))?
            } else {
                read_dim(&value, dim)?
            };
        }
        Ok(value)
    }
    pub(super) fn unpack_entries(&mut self, value: Value) -> EResult<Vec<(Key, Value)>> {
        match value {
            Value::Array(array) => Ok(array.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            Value::Object(obj) if obj.class.is_subclass_of("Traversable") => {
                let Some(mut iterator) = self.iter_start(Value::Object(obj), true)? else {
                    return Ok(Vec::new());
                };
                let mut out = Vec::new();
                while let Some((key, value)) = self.iter_next(&mut iterator, true)? {
                    let key = match key {
                        Value::Int(i) => Key::Int(i),
                        Value::Str(s) => Key::Str(s.as_slice().into()),
                        _ => {
                            return Err(Fatal::error(
                                "Keys must be of type int|string during argument unpacking",
                            ));
                        }
                    };
                    out.push((key, value));
                }
                Ok(out)
            }
            other => Err(Fatal::type_error(format!(
                "Only arrays and Traversables can be unpacked, {} given",
                other.value_name()
            ))),
        }
    }
    pub(super) fn order_builtin_args(
        &mut self,
        fname: &str,
        args: &[Arg],
    ) -> EResult<Option<Vec<Arg>>> {
        if !args.iter().any(|arg| arg.spread || arg.name.is_some()) {
            return Ok(None);
        }
        let sig = builtins::signature(&fname.to_ascii_lowercase()).unwrap_or(&[]);
        let mut slots: Vec<Option<Arg>> = Vec::new();
        let mut positional = 0;
        for arg in args {
            if arg.spread {
                let mut named = false;
                let source = if arg.value.is_lvalue() {
                    Some(self.path(&arg.value)?)
                } else {
                    None
                };
                let value = match &source {
                    Some(path) => self.read_argument_path(path)?,
                    None => self.expr(&arg.value)?,
                };
                let traversable = matches!(value, Value::Object(_));
                let entries = self.unpack_entries(value)?;
                for (key, value) in entries {
                    let name = match &key {
                        Key::Int(_) => None,
                        Key::Str(s) => Some(String::from_utf8_lossy(s).into_owned()),
                    };
                    if name.is_none() && named {
                        return Err(Fatal::error(
                            "Cannot use positional argument after named argument during unpacking",
                        ));
                    }
                    let index = name
                        .as_ref()
                        .and_then(|n| sig.iter().position(|p| p.name == n && !p.variadic))
                        .unwrap_or(if name.is_some() {
                            sig.len()
                        } else {
                            positional
                        });
                    let by_ref = sig.get(index).is_some_and(|p| p.by_ref);
                    let cell = if by_ref && !traversable {
                        if let Some(path) = &source {
                            let mut element = path.clone();
                            element.dims.push(Some(key.to_value()));
                            self.make_ref(&element)?
                        } else if let Value::Ref(reference) = value {
                            reference
                        } else {
                            Rc::new(RefCell::new(value.copy_out()))
                        }
                    } else {
                        if by_ref {
                            diag::warning(format!(
                                "Cannot pass by-reference argument {} of {fname}() by unpacking a Traversable, passing by-value instead",
                                index + 1
                            ));
                        }
                        Rc::new(RefCell::new(value.copy_out()))
                    };
                    self.place_builtin_arg(
                        fname,
                        sig,
                        &mut slots,
                        index,
                        name.as_deref(),
                        cell,
                        arg.value.line,
                    )?;
                    if name.is_some() {
                        named = true;
                    }
                    positional = slots.len();
                }
            } else {
                let index = arg
                    .name
                    .as_ref()
                    .and_then(|n| sig.iter().position(|p| p.name == n && !p.variadic))
                    .unwrap_or(if arg.name.is_some() {
                        sig.len()
                    } else {
                        positional
                    });
                let by_ref = sig.get(index).is_some_and(|p| p.by_ref);
                let cell = if by_ref {
                    if !arg.value.is_lvalue() {
                        let parameter = sig[index].name;
                        return Err(Fatal::error(format!(
                            "{fname}(): Argument #{} (${parameter}) could not be passed by reference",
                            index + 1
                        )));
                    }
                    let path = self.path(&arg.value)?;
                    self.make_ref(&path)?
                } else {
                    Rc::new(RefCell::new(self.expr(&arg.value)?))
                };
                self.place_builtin_arg(
                    fname,
                    sig,
                    &mut slots,
                    index,
                    arg.name.as_deref(),
                    cell,
                    arg.value.line,
                )?;
                positional = slots.len();
            }
        }
        let mut out = Vec::with_capacity(slots.len());
        for (index, arg) in slots.into_iter().enumerate() {
            if let Some(arg) = arg {
                out.push(arg);
            } else {
                let default = self.builtin_default(fname, index, &sig[index])?;
                let value = if sig[index].by_ref {
                    let v = self.expr(&default)?;
                    Expr::new(ExprKind::Argument(Rc::new(RefCell::new(v))), default.line)
                } else {
                    (*default).clone()
                };
                out.push(Arg {
                    value,
                    spread: false,
                    name: None,
                });
            }
        }
        Ok(Some(out))
    }
    fn place_builtin_arg(
        &self,
        fname: &str,
        sig: &[builtins::Param],
        slots: &mut Vec<Option<Arg>>,
        index: usize,
        name: Option<&str>,
        cell: RefCellValue,
        line: u32,
    ) -> EResult<()> {
        if let Some(name) = name
            && index == sig.len()
        {
            if sig.last().is_some_and(|p| p.variadic) {
                return Err(Fatal::new(
                    "ArgumentCountError",
                    format!("{fname}() does not accept unknown named parameters"),
                ));
            }
            return Err(Fatal::error(format!("Unknown named parameter ${name}")));
        }
        if slots.len() <= index {
            slots.resize_with(index + 1, || None);
        }
        if slots[index].is_some() {
            return Err(Fatal::error(format!(
                "Named parameter ${} overwrites previous argument",
                name.unwrap_or("")
            )));
        }
        slots[index] = Some(Arg {
            value: Expr::new(ExprKind::Argument(cell), line),
            spread: false,
            name: None,
        });
        Ok(())
    }
    pub(super) fn eval_named_values(
        &mut self,
        args: &[Arg],
    ) -> EResult<(Vec<Value>, Vec<(String, Value)>)> {
        let mut positional = Vec::new();
        let mut named = Vec::new();
        for arg in args {
            let value = self.expr(&arg.value)?;
            if arg.spread {
                for (key, value) in self.unpack_entries(value)? {
                    match key {
                        Key::Int(_) if !named.is_empty() => {
                            return Err(Fatal::error(
                                "Cannot use positional argument after named argument during unpacking",
                            ));
                        }
                        Key::Int(_) => positional.push(value.copy_out()),
                        Key::Str(name) => named.push((
                            String::from_utf8_lossy(&name).into_owned(),
                            value.copy_out(),
                        )),
                    }
                }
            } else if let Some(name) = &arg.name {
                named.push((name.clone(), value));
            } else {
                positional.push(value);
            }
        }
        Ok((positional, named))
    }
}
