//! Generators and the iteration protocol.
//!
//! A generator's body has been lowered (see genlower) so that it only yields at
//! statement level. It runs on the generator executor below, a variant of
//! Interp::stmt for the statements that contain yields. Statements without
//! yields (Stmt::Plain) run on the ordinary executor.
//!
//! Suspending unwinds the native stack: each construct on the way out records
//! where it was (a Saved entry: the index in a block, the branch of an `if`,
//! the iteration state of a foreach, the phase of a try, ...). Resuming
//! descends the same path, each construct taking its entry back, until the
//! yield that suspended receives the value sent in. Entries are pushed
//! innermost first and taken outermost first, so they form a stack; it is only
//! non-empty while suspended or descending, which is how a construct tells
//! that it is being resumed rather than entered.

use super::*;
use crate::array::PhpArray;

/// What a generator is resumed with
pub(super) enum Resume {
    /// next() or send(): the result of the pending yield
    Value(Value),
    /// throw(): raised where the generator is suspended
    Throw(Fatal),
    /// The generator is being destroyed: finally blocks run, catch blocks do not
    Destroy,
}

/// The executor's outcome for a statement: completed (possibly with a jump or
/// return), or suspended at a yield
pub(super) enum G {
    Flow(Flow),
    Suspend,
}

pub(crate) struct GenState {
    func: Rc<FuncDecl>,
    /// The generator's variables, taken out while it runs. None once finished
    frame: Option<Frame>,
    saved: Vec<Saved>,
    running: bool,
    finished: bool,
    /// Whether it has yielded (or finished): php-src's "initialized"
    started: bool,
    /// Suspended at its first yield, the only point it can be rewound to
    at_first_yield: bool,
    key: Value,
    value: Value,
    /// The largest integer key yielded so far, which auto keys continue from
    auto_key: i64,
    /// Set once the body returned
    ret: Option<Value>,
}

/// Where a suspended construct was
pub(super) enum Saved {
    Block(usize),
    /// The `if` branch taken; usize::MAX for `else`
    Branch(usize),
    /// Inside the body of a while, do-while or for loop
    Loop,
    Foreach(Iter),
    Switch(usize),
    Try(TryPhase),
    /// Suspended at this yield, waiting for its result
    AtYield,
    YieldFrom(Delegate),
}

pub(super) enum TryPhase {
    Body,
    Catch(usize),
    /// Running finally, with the outcome to resume afterwards
    Finally(Outcome),
}

pub(super) enum Outcome {
    Flow(Flow),
    Err(Fatal),
}

/// An iteration in progress (foreach, yield from, iterator_to_array)
pub(super) enum Iter {
    /// A by-value snapshot of an array, at a storage position
    Array(Rc<PhpArray>, usize),
    /// Properties visible in scope, snapshotted
    Props(Vec<(Value, Value)>, usize),
    Generator { obj: Rc<Object>, started: bool, by_ref: bool, _handle: object::Handle },
    RefArray {cell: RefCellValue, position: usize},
    RefProps {obj: Rc<Object>, declared: usize, dynamic: usize},
    /// An object implementing Iterator
    User { obj: Rc<Object>, started: bool, _handle: object::Handle },
}

pub(super) enum Delegate {
    /// `yield from` another generator: sent values and exceptions go to it
    Generator(Rc<Object>),
    /// An array or Iterator, whose values are yielded as they are
    Iter(Iter),
}

/// A generator being run, innermost last
pub(super) struct GenRun {
    obj: Rc<Object>,
    saved: Vec<Saved>,
    input: Option<Resume>,
}

/// The pseudo-exception that unwinds a generator being destroyed
fn generator_exit() -> Fatal {
    Fatal { uncatchable: true, ..Fatal::new(GENERATOR_EXIT, "") }
}

const GENERATOR_EXIT: &str = "\0GeneratorExit";

fn is_exit(e: &Fatal) -> bool {
    e.class == GENERATOR_EXIT
}

fn state_mismatch() -> Fatal {
    Fatal::error("generator state does not match its code")
}

fn exception(msg: &str) -> Fatal {
    Fatal::new("Exception", msg)
}

impl GenState {
    /// Suspended inside a try, so destroying it has finally blocks to run
    pub(crate) fn in_try(&self) -> bool {
        !self.running && self.frame.is_some() && self.saved.iter().any(|s| matches!(s, Saved::Try(_)))
    }

    fn current(&self) -> (Value, Value) {
        (self.key.clone(), self.value.copy_out())
    }
}

impl<W: Write> Interp<W> {
    // ---- Creating and running generators ----

    /// Called instead of running the body of a generator function: its frame,
    /// with the arguments bound, becomes the generator
    pub(super) fn make_generator(&mut self, func: &Rc<FuncDecl>, frame: Frame) -> Value {
        let state = GenState {
            func: func.clone(),
            frame: Some(frame),
            saved: Vec::new(),
            running: false,
            finished: false,
            started: false,
            at_first_yield: false,
            key: Value::Null,
            value: Value::Null,
            auto_key: -1,
            ret: None,
        };
        let class = self.classes["generator"].clone();
        Value::Object(Object::new(class, Internal::Generator(Box::new(RefCell::new(state)))))
    }

    fn gen_state(obj: &Object) -> &RefCell<GenState> {
        match &obj.internal {
            Internal::Generator(g) => g,
            _ => unreachable!("only called on Generator objects"),
        }
    }

    /// Runs the generator until it yields, returns or throws
    pub(super) fn gen_resume(&mut self, obj: &Rc<Object>, input: Resume) -> EResult<()> {
        let cell = Self::gen_state(obj);
        let (func, mut frame, saved) = {
            let mut st = cell.borrow_mut();
            if st.running {
                return Err(Fatal::error("Cannot resume an already running generator"));
            }
            let Some(frame) = st.frame.take() else { return Ok(()) };
            st.running = true;
            st.at_first_yield = false;
            (st.func.clone(), frame, std::mem::take(&mut st.saved))
        };
        if self.frames.len() + self.nesting > MAX_CALL_DEPTH {
            let mut st = cell.borrow_mut();
            st.running = false;
            st.frame = Some(frame);
            st.saved = saved;
            return Err(Fatal::error(
                "Maximum call stack size of 8355840 bytes (zend.max_allowed_stack_size - zend.reserved_stack_size) reached. Infinite recursion?",
            ));
        }
        // In traces the generator shows as called from where it is resumed
        let line = diag::line();
        frame.call_line = line;
        frame.by_builtin =
            self.internal_calls.last().is_some_and(|(depth, _, include)| !include && *depth == self.frames.len());
        let caller_file = self.enter_file(&func.file);
        frame.call_file = caller_file.path.clone();
        self.frames.push(frame);
        self.gen_runs.push(GenRun { obj: obj.clone(), saved, input: Some(input) });
        // Resuming nests deeper natively than a call, so it counts twice towards the limit
        self.nesting += 1;
        let result = self.gblock(&func.body);
        self.nesting -= 1;
        let result = result.map_err(|e| self.frame_error(e));
        let run = self.gen_runs.pop().expect("pushed above");
        let frame = self.frames.pop().expect("pushed above");
        self.enter_file(&caller_file);
        diag::set_line(line);
        let mut st = cell.borrow_mut();
        st.running = false;
        st.started = true;
        let result = match result {
            Ok(G::Suspend) => {
                st.frame = Some(frame);
                st.saved = run.saved;
                return Ok(());
            }
            Ok(G::Flow(flow)) => {
                st.ret = Some(match flow {
                    Flow::Return(v) => v,
                    Flow::Goto(label) => return Err(super::undefined_label(&label)),
                    _ => Value::Null,
                });
                Ok(())
            }
            Err(e) => Err(e),
        };
        st.finished = true;
        st.key = Value::Null;
        st.value = Value::Null;
        drop(st);
        // The finished generator's variables are released like a returning function's
        drop(frame);
        drop(run);
        self.with_destructors(result)
    }

    /// Runs a generator that has not started to its first yield
    fn gen_init(&mut self, obj: &Rc<Object>) -> EResult<()> {
        let st = Self::gen_state(obj).borrow();
        // A running generator that has not yielded yet ends up in gen_resume's error
        if st.started || st.finished {
            return Ok(());
        }
        drop(st);
        self.gen_resume(obj, Resume::Value(Value::Null))?;
        Self::gen_state(obj).borrow_mut().at_first_yield = true;
        Ok(())
    }

    /// foreach by reference over a generator, which cannot yield references
    pub(super) fn generator_by_ref(&self, obj: &Object) -> Fatal {
        exception(if Self::gen_state(obj).borrow().finished {
            "Cannot traverse an already closed generator"
        } else {
            "You can only iterate a generator by-reference if it declared that it yields by-reference"
        })
    }

    fn gen_valid(&mut self, obj: &Rc<Object>) -> EResult<bool> {
        self.gen_init(obj)?;
        Ok(!Self::gen_state(obj).borrow().finished)
    }

    fn gen_current(&mut self, obj: &Rc<Object>) -> EResult<(Value, Value)> {
        self.gen_init(obj)?;
        Ok(Self::gen_state(obj).borrow().current())
    }

    fn gen_next(&mut self, obj: &Rc<Object>) -> EResult<()> {
        self.gen_init(obj)?;
        self.gen_resume(obj, Resume::Value(Value::Null))
    }

    fn gen_rewind(&mut self, obj: &Rc<Object>) -> EResult<()> {
        self.gen_init(obj)?;
        if !Self::gen_state(obj).borrow().at_first_yield {
            return Err(exception("Cannot rewind a generator that was already run"));
        }
        Ok(())
    }

    /// Destroying a generator suspended inside a try runs its finally blocks
    pub(super) fn gen_destroy(&mut self, obj: &Rc<Object>) -> EResult<()> {
        if !Self::gen_state(obj).borrow().in_try() {
            return Ok(());
        }
        match self.gen_resume(obj, Resume::Destroy) {
            Err(e) if is_exit(&e) => Ok(()),
            other => other,
        }
    }

    /// The Generator class's methods
    pub(super) fn generator_method(&mut self, n: Native, this: &Rc<Object>, vals: Vec<Value>) -> EResult<Value> {
        let name = match n {
            Native::GenCurrent => "current",
            Native::GenKey => "key",
            Native::GenNext => "next",
            Native::GenValid => "valid",
            Native::GenRewind => "rewind",
            Native::GenSend => "send",
            Native::GenThrow => "throw",
            _ => "getReturn",
        };
        let takes = usize::from(matches!(n, Native::GenSend | Native::GenThrow));
        if vals.len() != takes {
            return Err(Fatal::new(
                "ArgumentCountError",
                format!(
                    "Generator::{name}() expects exactly {takes} argument{}, {} given",
                    if takes == 1 { "" } else { "s" },
                    vals.len()
                ),
            ));
        }
        Ok(match n {
            Native::GenCurrent => self.gen_current(this)?.1,
            Native::GenKey => self.gen_current(this)?.0,
            Native::GenNext => {
                self.gen_next(this)?;
                Value::Null
            }
            Native::GenValid => Value::Bool(self.gen_valid(this)?),
            Native::GenRewind => {
                self.gen_rewind(this)?;
                Value::Null
            }
            Native::GenSend => {
                self.gen_init(this)?;
                let v = vals.into_iter().next().unwrap_or(Value::Null);
                self.gen_resume(this, Resume::Value(v))?;
                self.gen_state_value(this)
            }
            Native::GenThrow => {
                let ex = match vals.into_iter().next() {
                    Some(Value::Object(o)) if o.class.is_throwable => o,
                    other => {
                        return Err(Fatal::type_error(format!(
                            "Generator::throw(): Argument #1 ($exception) must be of type Throwable, {} given",
                            other.unwrap_or(Value::Null).value_name()
                        )))
                    }
                };
                self.gen_init(this)?;
                if Self::gen_state(this).borrow().finished {
                    // A closed generator throws in the caller's context
                    return Err(Fatal::thrown(ex));
                }
                self.gen_resume(this, Resume::Throw(Fatal::thrown(ex)))?;
                self.gen_state_value(this)
            }
            _ => {
                self.gen_init(this)?;
                match &Self::gen_state(this).borrow().ret {
                    Some(v) => v.clone(),
                    None => return Err(exception("Cannot get return value of a generator that hasn't returned")),
                }
            }
        })
    }

    fn gen_state_value(&self, obj: &Rc<Object>) -> Value {
        Self::gen_state(obj).borrow().value.clone()
    }

    // ---- The generator executor ----

    fn g_take(&mut self) -> Option<Saved> {
        self.gen_runs.last_mut()?.saved.pop()
    }

    fn g_push(&mut self, s: Saved) {
        if let Some(run) = self.gen_runs.last_mut() {
            run.saved.push(s);
        }
    }

    fn gblock(&mut self, stmts: &[Stmt]) -> EResult<G> {
        let start = match self.g_take() {
            None => 0,
            Some(Saved::Block(i)) => i,
            Some(_) => return Err(state_mismatch()),
        };
        let mut i = start;
        while let Some(s) = stmts.get(i) {
            let r = self.gstmt(s)?;
            if let G::Suspend = r {
                self.g_push(Saved::Block(i));
                return Ok(G::Suspend);
            }
            self.run_destructors()?;
            i += 1;
            match r {
                G::Flow(Flow::Normal) => {}
                G::Flow(Flow::Goto(label)) => match super::label_index(stmts, &label) {
                    Some(at) => i = at + 1,
                    None => return Ok(G::Flow(Flow::Goto(label))),
                },
                r => return Ok(r),
            }
        }
        Ok(G::Flow(Flow::Normal))
    }

    fn gstmt(&mut self, s: &Stmt) -> EResult<G> {
        match s {
            Stmt::Plain(inner) => Ok(G::Flow(self.stmt(inner)?)),
            Stmt::Yield { key, value, target } => self.g_yield(key.as_ref(), value.as_ref(), target.as_deref()),
            Stmt::YieldFrom { source, target } => self.g_yield_from(source, target.as_deref()),
            Stmt::Block(body) => self.gblock(body),
            Stmt::If(branches, else_body) => {
                let branch = match self.g_take() {
                    Some(Saved::Branch(i)) => i,
                    Some(_) => return Err(state_mismatch()),
                    None => {
                        let mut taken = None;
                        for (i, (cond, _)) in branches.iter().enumerate() {
                            if self.expr(cond)?.to_bool() {
                                taken = Some(i);
                                break;
                            }
                        }
                        match taken {
                            Some(i) => i,
                            None if else_body.is_some() => usize::MAX,
                            None => return Ok(G::Flow(Flow::Normal)),
                        }
                    }
                };
                let body = match branch {
                    usize::MAX => else_body.as_deref().unwrap_or_default(),
                    i => &branches[i].1,
                };
                let r = self.gblock(body)?;
                if let G::Suspend = r {
                    self.g_push(Saved::Branch(branch));
                }
                Ok(r)
            }
            Stmt::While(cond, body) => {
                let mut resumed = self.g_take_loop()?;
                loop {
                    if !resumed && !self.expr(cond)?.to_bool() {
                        break;
                    }
                    resumed = false;
                    if let Some(r) = self.g_loop_body(body)? {
                        return Ok(r);
                    }
                }
                Ok(G::Flow(Flow::Normal))
            }
            Stmt::DoWhile(body, cond) => {
                self.g_take_loop()?;
                loop {
                    if let Some(r) = self.g_loop_body(body)? {
                        return Ok(r);
                    }
                    if !self.expr(cond)?.to_bool() {
                        break;
                    }
                }
                Ok(G::Flow(Flow::Normal))
            }
            Stmt::For(init, cond, step, body) => {
                let mut resumed = self.g_take_loop()?;
                if !resumed {
                    for e in init {
                        self.expr(e)?;
                    }
                }
                loop {
                    if !resumed {
                        let mut ok = true;
                        for e in cond {
                            ok = self.expr(e)?.to_bool();
                        }
                        if !ok {
                            break;
                        }
                    }
                    resumed = false;
                    if let Some(r) = self.g_loop_body(body)? {
                        return Ok(r);
                    }
                    for e in step {
                        self.expr(e)?;
                    }
                }
                Ok(G::Flow(Flow::Normal))
            }
            Stmt::Foreach { subject, key, value, by_ref, body } => {
                let (mut it, mut resumed) = match self.g_take() {
                    Some(Saved::Foreach(it)) => (it, true),
                    Some(_) => return Err(state_mismatch()),
                    None => {
                        let iterator = if *by_ref {self.iter_start_reference(subject)?}else{let v=self.expr(subject)?;self.iter_start(v,false)?};
                        match iterator {
                            Some(it) => (it, false),
                            None => return Ok(G::Flow(Flow::Normal)),
                        }
                    }
                };
                loop {
                    if !resumed {
                        let Some((k, v)) = self.iter_next(&mut it, key.is_some())? else { break };
                        if let Some(key_target) = key {
                            self.assign(key_target, k)?;
                        }
                        if *by_ref {let Value::Ref(reference)=v else{return Err(state_mismatch());};self.bind_ref(value,reference)?;}else{self.assign(value,v)?;}
                    }
                    resumed = false;
                    match self.gblock(body)? {
                        G::Suspend => {
                            self.g_push(Saved::Foreach(it));
                            return Ok(G::Suspend);
                        }
                        G::Flow(flow) => {
                            if let Some(flow) = loop_flow(flow) {
                                return Ok(G::Flow(flow));
                            }
                        }
                    }
                }
                Ok(G::Flow(Flow::Normal))
            }
            Stmt::Switch(subject, cases) => {
                let start = match self.g_take() {
                    Some(Saved::Switch(i)) => i,
                    Some(_) => return Err(state_mismatch()),
                    None => {
                        let v = self.expr(subject)?;
                        let mut start = None;
                        for (i, (cond, _)) in cases.iter().enumerate() {
                            if let Some(c) = cond {
                                let cv = self.expr(c)?;
                                if binary("==", &v, &cv)?.to_bool() {
                                    start = Some(i);
                                    break;
                                }
                            }
                        }
                        match start.or_else(|| cases.iter().position(|(c, _)| c.is_none())) {
                            Some(i) => i,
                            None => return Ok(G::Flow(Flow::Normal)),
                        }
                    }
                };
                for (i, (_, body)) in cases.iter().enumerate().skip(start) {
                    match self.gblock(body)? {
                        G::Suspend => {
                            self.g_push(Saved::Switch(i));
                            return Ok(G::Suspend);
                        }
                        G::Flow(Flow::Normal) => {}
                        G::Flow(Flow::Break(1) | Flow::Continue(1)) => break,
                        G::Flow(Flow::Break(n)) => return Ok(G::Flow(Flow::Break(n - 1))),
                        G::Flow(Flow::Continue(n)) => return Ok(G::Flow(Flow::Continue(n - 1))),
                        G::Flow(ret @ (Flow::Return(_) | Flow::Goto(_))) => return Ok(G::Flow(ret)),
                    }
                }
                Ok(G::Flow(Flow::Normal))
            }
            Stmt::Try { body, catches, finally } => self.g_try(body, catches, finally.as_deref()),
            other => Ok(G::Flow(self.stmt(other)?)),
        }
    }

    fn g_take_loop(&mut self) -> EResult<bool> {
        match self.g_take() {
            None => Ok(false),
            Some(Saved::Loop) => Ok(true),
            Some(_) => Err(state_mismatch()),
        }
    }

    /// One pass through a loop body: Some when the loop is left (or suspended)
    fn g_loop_body(&mut self, body: &[Stmt]) -> EResult<Option<G>> {
        match self.gblock(body)? {
            G::Suspend => {
                self.g_push(Saved::Loop);
                Ok(Some(G::Suspend))
            }
            G::Flow(flow) => Ok(loop_flow(flow).map(G::Flow)),
        }
    }

    fn g_try(&mut self, body: &[Stmt], catches: &[crate::ast::Catch], finally: Option<&[Stmt]>) -> EResult<G> {
        let mut phase = match self.g_take() {
            None => TryPhase::Body,
            Some(Saved::Try(p)) => p,
            Some(_) => return Err(state_mismatch()),
        };
        loop {
            phase = match phase {
                TryPhase::Body => match self.gblock(body) {
                    Ok(G::Suspend) => {
                        self.g_push(Saved::Try(TryPhase::Body));
                        return Ok(G::Suspend);
                    }
                    Ok(G::Flow(flow)) => TryPhase::Finally(Outcome::Flow(flow)),
                    Err(e) if is_exit(&e) => TryPhase::Finally(Outcome::Err(e)),
                    Err(e) if e.uncatchable => return Err(e),
                    Err(e) => {
                        let ex = self.exception_object(e);
                        match catches.iter().position(|c| c.types.iter().any(|t| self.is_instance_of(&ex.class, t))) {
                            Some(i) => {
                                if let Some(var) = &catches[i].var {
                                    self.with_place(&Path::var(var), false, true, |slot| {
                                        *slot = Value::Object(ex.clone());
                                        Ok(())
                                    })?;
                                }
                                TryPhase::Catch(i)
                            }
                            None => TryPhase::Finally(Outcome::Err(Fatal::thrown(ex))),
                        }
                    }
                },
                TryPhase::Catch(i) => match self.gblock(&catches[i].body) {
                    Ok(G::Suspend) => {
                        self.g_push(Saved::Try(TryPhase::Catch(i)));
                        return Ok(G::Suspend);
                    }
                    Ok(G::Flow(flow)) => TryPhase::Finally(Outcome::Flow(flow)),
                    Err(e) if e.uncatchable && !is_exit(&e) => return Err(e),
                    Err(e) => TryPhase::Finally(Outcome::Err(e)),
                },
                TryPhase::Finally(pending) => {
                    let Some(fin) = finally else {
                        return match pending {
                            Outcome::Flow(flow) => Ok(G::Flow(flow)),
                            Outcome::Err(e) => Err(e),
                        };
                    };
                    return match self.gblock(fin) {
                        Ok(G::Suspend) => {
                            self.g_push(Saved::Try(TryPhase::Finally(pending)));
                            Ok(G::Suspend)
                        }
                        Ok(G::Flow(Flow::Normal)) => match pending {
                            Outcome::Flow(flow) => Ok(G::Flow(flow)),
                            Outcome::Err(e) => Err(e),
                        },
                        // A return, break or new exception in finally replaces the pending outcome
                        Ok(G::Flow(flow)) => Ok(G::Flow(flow)),
                        Err(new) if new.uncatchable => Err(new),
                        Err(new) => match pending {
                            Outcome::Err(p) if !p.uncatchable => {
                                let new = self.exception_object(new);
                                let p = self.exception_object(p);
                                object::chain_previous(&new, p);
                                Err(Fatal::thrown(new))
                            }
                            _ => Err(new),
                        },
                    };
                }
            };
        }
    }

    /// The result a suspended yield receives, from the run's input
    fn g_received(&mut self) -> EResult<Value> {
        match self.gen_runs.last_mut().and_then(|r| r.input.take()) {
            Some(Resume::Value(v)) => Ok(v),
            Some(Resume::Throw(e)) => Err(e),
            Some(Resume::Destroy) => Err(generator_exit()),
            None => Ok(Value::Null),
        }
    }

    fn g_yield(&mut self, key: Option<&Expr>, value: Option<&Expr>, target: Option<&str>) -> EResult<G> {
        match self.g_take() {
            Some(Saved::AtYield) => {
                let v = self.g_received()?;
                if let Some(t) = target {
                    self.frame().vars.insert(t.to_string(), v);
                }
                return Ok(G::Flow(Flow::Normal));
            }
            Some(_) => return Err(state_mismatch()),
            None => {}
        }
        let key = key.map(|k| self.expr(k)).transpose()?;
        let by_ref=Self::gen_state(&self.gen_runs.last().expect("yield runs in a generator").obj).borrow().func.returns_ref;
        let value = if by_ref {
            if let Some(value)=value && value.is_lvalue() {let path=self.path(value)?;Value::Ref(self.make_ref(&path)?)}
            else {if let Some(value)=value {diag::set_line(value.line);diag::notice("Only variable references should be yielded by reference");}Value::Ref(Rc::new(RefCell::new(value.map(|v|self.expr(v)).transpose()?.unwrap_or(Value::Null))))}
        } else {value.map(|v|self.expr(v)).transpose()?.unwrap_or(Value::Null)};
        let obj = self.gen_runs.last().expect("yield runs in a generator").obj.clone();
        let mut st = Self::gen_state(&obj).borrow_mut();
        st.key = match key {
            Some(k) => {
                if let Value::Int(i) = k
                    && i > st.auto_key
                {
                    st.auto_key = i;
                }
                k
            }
            None => {
                st.auto_key = st.auto_key.saturating_add(1);
                Value::Int(st.auto_key)
            }
        };
        st.value = value;
        drop(st);
        self.g_push(Saved::AtYield);
        Ok(G::Suspend)
    }

    fn g_yield_from(&mut self, source: &Expr, target: Option<&str>) -> EResult<G> {
        let obj = self.gen_runs.last().expect("yield from runs in a generator").obj.clone();
        let (delegate, resumed) = match self.g_take() {
            Some(Saved::YieldFrom(d)) => (d, true),
            Some(_) => return Err(state_mismatch()),
            None => {
                let v = self.expr(source)?;
                let d = match v {
                    Value::Object(inner) if matches!(inner.internal, Internal::Generator(_)) => {
                        if Rc::ptr_eq(&inner, &obj) {
                            return Err(Fatal::error("Impossible to yield from the Generator being currently run"));
                        }
                        Delegate::Generator(inner)
                    }
                    v @ (Value::Array(_) | Value::Object(_)) if self.is_traversable(&v) => match self.iter_start(v, true)? {
                        Some(it) => Delegate::Iter(it),
                        None => return Ok(G::Flow(Flow::Normal)),
                    },
                    _ => return Err(Fatal::error("Can use \"yield from\" only with arrays and Traversables")),
                };
                (d, false)
            }
        };
        let current = match delegate {
            Delegate::Generator(inner) => {
                if resumed {
                    match self.gen_runs.last_mut().and_then(|r| r.input.take()) {
                        Some(Resume::Value(v)) => self.gen_resume(&inner, Resume::Value(v))?,
                        Some(Resume::Throw(e)) => self.gen_resume(&inner, Resume::Throw(e))?,
                        Some(Resume::Destroy) => return Err(generator_exit()),
                        None => {}
                    }
                } else {
                    let st = Self::gen_state(&inner).borrow();
                    if st.finished && st.ret.is_none() {
                        drop(st);
                        return Err(Fatal::new(
                            "ClosedGeneratorException",
                            "Generator yielded from aborted, no return value available",
                        ));
                    }
                    drop(st);
                    self.gen_init(&inner)?;
                }
                let st = Self::gen_state(&inner).borrow();
                if st.finished {
                    let ret = st.ret.clone().unwrap_or(Value::Null);
                    drop(st);
                    if let Some(t) = target {
                        self.frame().vars.insert(t.to_string(), ret);
                    }
                    return Ok(G::Flow(Flow::Normal));
                }
                let current = st.current();
                drop(st);
                (current, Delegate::Generator(inner))
            }
            Delegate::Iter(mut it) => {
                if resumed {
                    // Values sent while delegating to an array or iterator are dropped
                    self.g_received()?;
                }
                match self.iter_next(&mut it, true)? {
                    Some(current) => (current, Delegate::Iter(it)),
                    None => {
                        if let Some(t) = target {
                            self.frame().vars.insert(t.to_string(), Value::Null);
                        }
                        return Ok(G::Flow(Flow::Normal));
                    }
                }
            }
        };
        let ((k, v), delegate) = current;
        let mut st = Self::gen_state(&obj).borrow_mut();
        st.key = k;
        st.value = v;
        drop(st);
        self.g_push(Saved::YieldFrom(delegate));
        Ok(G::Suspend)
    }

    // ---- Iteration ----

    pub(super) fn iter_start_reference(&mut self, subject:&Expr) -> EResult<Option<Iter>> {
        let path=if subject.is_lvalue() && !matches!(&subject.kind,ExprKind::Var(name) if name=="this"){Some(self.path(subject)?)}else{None};
        let value=if let Some(path)=&path {self.with_place(path,true,true,|v|Ok(v.copy_out()))?}else{self.expr(subject)?};
        match value {
            Value::Array(array)=>{let cell=if let Some(path)=path {self.make_ref(&path)?}else{Rc::new(RefCell::new(Value::Array(array)))};Ok(Some(Iter::RefArray{cell,position:0}))}
            Value::Object(mut obj)=>{
                let mut depth=0;
                while obj.class.is_subclass_of("IteratorAggregate")&&!obj.class.is_subclass_of("Iterator") {
                    let class=obj.class.name;
                    let next=self.call_named_method(&obj,"getIterator")?;depth+=1;
                    if let Value::Object(next)=next && next.class.is_subclass_of("Traversable") && depth<64 {obj=next;}
                    else{return Err(exception(&format!("Objects returned by {class}::getIterator() must be traversable or implement interface Iterator")));}
                }
                if matches!(obj.internal,Internal::Generator(_)) {
                    if !Self::gen_state(&obj).borrow().func.returns_ref || Self::gen_state(&obj).borrow().finished {return Err(self.generator_by_ref(&obj));}
                    Ok(Some(Iter::Generator{obj,started:false,by_ref:true,_handle:object::Handle::new()}))
                }else if obj.class.is_subclass_of("Traversable") {Err(Fatal::error("An iterator cannot be used with foreach by reference"))}
                else{Ok(Some(Iter::RefProps{obj,declared:0,dynamic:0}))}
            }
            other=>{foreach_type_warning(&other);Ok(None)}
        }
    }

    pub(super) fn is_traversable(&self, v: &Value) -> bool {
        match v {
            Value::Array(_) => true,
            Value::Object(o) => o.class.is_subclass_of("Traversable"),
            _ => false,
        }
    }

    /// Starts iterating a value. For anything but arrays and objects, warns
    /// (unless `quiet`) and gives None
    pub(super) fn iter_start(&mut self, v: Value, quiet: bool) -> EResult<Option<Iter>> {
        Ok(Some(match v {
            Value::Array(arr) => Iter::Array(arr, 0),
            Value::Object(o) => {
                let mut o = o;
                // IteratorAggregate hands out the iterator to use
                let mut depth = 0;
                while !matches!(o.internal, Internal::Generator(_))
                    && !o.class.is_subclass_of("Iterator")
                    && o.class.is_subclass_of("IteratorAggregate")
                {
                    let class = o.class.name;
                    let it = self.call_named_method(&o, "getIterator")?;
                    match it {
                        Value::Object(next) if next.class.is_subclass_of("Traversable") && depth < 64 => o = next,
                        _ => {
                            return Err(exception(&format!(
                                "Objects returned by {class}::getIterator() must be traversable or implement interface Iterator"
                            )))
                        }
                    }
                    depth += 1;
                }
                if matches!(o.internal, Internal::Generator(_)) {
                    if Self::gen_state(&o).borrow().finished {
                        return Err(exception("Cannot traverse an already closed generator"));
                    }
                    Iter::Generator { obj: o, started: false, by_ref: false, _handle: object::Handle::new() }
                } else if o.class.is_subclass_of("Iterator") {
                    Iter::User { obj: o, started: false, _handle: object::Handle::new() }
                } else {
                    Iter::Props(self.visible_props(&o), 0)
                }
            }
            other => {
                if !quiet {
                    foreach_type_warning(&other);
                }
                return Ok(None);
            }
        }))
    }

    /// The next key and value. The key is only asked for when `want_key`
    pub(super) fn iter_next(&mut self, it: &mut Iter, want_key: bool) -> EResult<Option<(Value, Value)>> {
        self.iter_step(it, want_key, true)
    }

    /// iter_next, also leaving out the value unless `want_value`
    fn iter_step(&mut self, it: &mut Iter, want_key: bool, want_value: bool) -> EResult<Option<(Value, Value)>> {
        match it {
            Iter::RefArray {cell,position} => {
                let mut root=cell.try_borrow_mut().map_err(|_|recursive_reference())?;
                let Value::Array(array)=&mut *root else{return Ok(None);};
                let Some((next,key,_))=array.entry_from(*position) else{return Ok(None);};
                let key=key.clone();*position=next;
                let value=Rc::make_mut(array).get_mut(&key).expect("entry exists");
                Ok(Some((key.to_value(),Value::Ref(make_ref_slot(value)))))
            }
            Iter::RefProps {obj,declared,dynamic} => {
                loop {
                    let (key,name)=if *declared<obj.class.props.len() {
                        let prop=obj.class.props[*declared].clone();*declared+=1;
                        if !self.can_access(prop.vis,prop.class)||!obj.props.borrow().contains_key(&prop.key){continue;}
                        (prop.key,prop.name)
                    }else{
                        let entry=obj.props.borrow().entry_from(*dynamic).map(|(next,key,_)|(next,key.clone()));
                        let Some((next,key))=entry else{return Ok(None);};*dynamic=next;
                        if obj.class.props.iter().any(|p|p.key==key){continue;}
                        let name=object::unmangle(&key).0;
                        if !builtins::date_interval::visible(obj,&name){continue;}
                        (key,name)
                    };
                    let path=Path{base:Base::Prop(obj.clone(),name.clone()),dims:Vec::new()};
                    let reference=self.make_ref(&path)?;
                    let _=key;
                    return Ok(Some((Value::str(name),Value::Ref(reference))));
                }
            }
            Iter::Array(arr, pos) => Ok(arr.entry_from(*pos).map(|(next, k, v)| {
                *pos = next;
                (k.to_value(), v.copy_out())
            })),
            Iter::Props(entries, pos) => {
                let entry = entries.get(*pos).cloned();
                *pos += 1;
                Ok(entry)
            }
            Iter::Generator { obj, started, by_ref, .. } => {
                let obj = obj.clone();
                if std::mem::replace(started, true) {
                    self.gen_next(&obj)?;
                } else {
                    self.gen_rewind(&obj)?;
                }
                if !self.gen_valid(&obj)? {
                    return Ok(None);
                }
                if *by_ref {let state=Self::gen_state(&obj).borrow();Ok(Some((state.key.clone(),state.value.clone())))}else{self.gen_current(&obj).map(Some)}
            }
            Iter::User { obj, started, .. } => {
                let obj = obj.clone();
                let first = !std::mem::replace(started, true);
                self.call_named_method(&obj, if first { "rewind" } else { "next" })?;
                if !self.call_named_method(&obj, "valid")?.to_bool() {
                    return Ok(None);
                }
                let v = if want_value { self.call_named_method(&obj, "current")? } else { Value::Null };
                let k = if want_key { self.call_named_method(&obj, "key")? } else { Value::Null };
                Ok(Some((k, v)))
            }
        }
    }

    pub(super) fn call_named_method(&mut self, obj: &Rc<Object>, name: &str) -> EResult<Value> {
        let Some(m) = obj.class.find_method(name).cloned() else {
            return Err(Fatal::error(format!("Call to undefined method {}::{name}()", obj.class.name)));
        };
        let line = diag::line();
        let v = self.call_method(Some(obj.clone()), &m, Vec::new(), line, obj.class.clone());
        diag::set_line(line);
        v
    }

    /// foreach over a Traversable object, outside generators
    pub(super) fn foreach_iter(&mut self, mut it: Iter, key: Option<&Expr>, value: &Expr, body: &[Stmt], by_ref:bool) -> EResult<Flow> {
        while let Some((k, v)) = self.iter_next(&mut it, key.is_some())? {
            if let Some(key_target) = key {
                self.assign(key_target, k)?;
            }
            if by_ref {let Value::Ref(reference)=v else{return Err(state_mismatch());};self.bind_ref(value,reference)?;}else{self.assign(value,v)?;}
            if let Some(flow) = loop_flow(self.block(body)?) {
                return Ok(flow);
            }
        }
        Ok(Flow::Normal)
    }

    /// iterator_to_array() and iterator_count()
    pub(super) fn iterator_builtin(&mut self, lname: &str, vals: Vec<Value>) -> EResult<Value> {
        let max = if lname == "iterator_count" { 1 } else { 2 };
        if vals.is_empty() || vals.len() > max {
            let msg = if vals.is_empty() {
                format!("{lname}() expects at least 1 argument, 0 given")
            } else {
                format!("{lname}() expects at most {max} arguments, {} given", vals.len())
            };
            return Err(Fatal::new("ArgumentCountError", msg));
        }
        let mut vals = vals.into_iter();
        let subject = vals.next().unwrap();
        let preserve_keys = vals.next().is_none_or(|v| v.to_bool());
        if !self.is_traversable(&subject) {
            return Err(Fatal::type_error(format!(
                "{lname}(): Argument #1 ($iterator) must be of type Traversable|array, {} given",
                subject.value_name()
            )));
        }
        let Some(mut it) = self.iter_start(subject, true)? else { unreachable!("checked traversable") };
        let mut count = 0;
        let mut out = PhpArray::new();
        let to_array = lname == "iterator_to_array";
        while let Some((k, v)) = self.iter_step(&mut it, to_array && preserve_keys, to_array)? {
            count += 1;
            if lname == "iterator_count" {
                continue;
            }
            if preserve_keys {
                let key = match &k {
                    Value::Null => Key::Str(Default::default()),
                    Value::Bool(b) => Key::Int(i64::from(*b)),
                    Value::Float(f) => Key::Int(float_to_int_implicit(*f, &k)),
                    Value::Int(_) | Value::Str(_) => to_key(&k, "array")?,
                    other => {
                        return Err(Fatal::type_error(format!("Cannot access offset of type {} on array", other.type_name())))
                    }
                };
                out.insert(key, v);
            } else {
                out.push(v).map_err(|_| next_element_occupied())?;
            }
        }
        if lname == "iterator_count" {
            return Ok(Value::Int(count));
        }
        Ok(Value::Array(Rc::new(out)))
    }
}
