//! Lowering of generator bodies.
//!
//! A generator must be able to stop at a `yield` and continue there later, but
//! the interpreter evaluates expressions by native recursion, which cannot be
//! suspended halfway. So before a generator runs, every `yield` is moved out of
//! the expression it appears in and becomes a statement of its own
//! (Stmt::Yield), whose result lands in a hidden variable that takes its place:
//!
//! ```text
//! echo f() . (yield $x);   =>   $t0 = f(); yield $x -> $t1; echo $t0 . $t1;
//! ```
//!
//! Operands evaluated before the yield are kept in hidden variables so the order
//! of side effects stays the same. Short-circuiting operators, `?:`, `??` and
//! `match` with a yield in a conditionally evaluated part become `if`s, and loop
//! conditions that yield move into the loop body. What is left only yields at
//! statement level, where the generator executor (interp::generator) can save its
//! position. Statements without yields are wrapped in Stmt::Plain and run as
//! usual.

use crate::ast::{ArrayItem, ClassRef, Expr, ExprKind, MatchArm, Name, Stmt};
use crate::parser::ParseError;

pub fn lower(body: Vec<Stmt>) -> Result<Vec<Stmt>, ParseError> {
    Lowerer { next: 0 }.block(body)
}

/// Hidden variables start with a NUL byte, which a PHP variable name cannot contain
pub fn is_hidden(name: &str) -> bool {
    name.starts_with('\0')
}

struct Lowerer {
    next: usize,
}

fn var(name: &str, line: u32) -> Expr {
    Expr::new(ExprKind::Var(name.to_string()), line)
}

fn assign(name: &str, v: Expr) -> Stmt {
    let line = v.line;
    Stmt::Expr(Expr::new(ExprKind::Assign(Box::new(var(name, line)), Box::new(v)), line))
}

fn not(e: Expr) -> Expr {
    let line = e.line;
    Expr::new(ExprKind::Unary("!", Box::new(e)), line)
}

fn bool_cast(e: Expr) -> Expr {
    let line = e.line;
    Expr::new(ExprKind::Unary("(bool)", Box::new(e)), line)
}

fn if_then(cond: Expr, body: Vec<Stmt>) -> Stmt {
    Stmt::If(vec![(cond, body)], None)
}

fn break_unless(cond: Expr) -> Stmt {
    if_then(not(cond), vec![Stmt::Break(1)])
}

fn unsupported(what: &str, line: u32) -> ParseError {
    ParseError { msg: format!("\"yield\" in {what} is not supported yet"), line }
}

/// Whether evaluating the expression later instead of now cannot change its
/// value or side effects (variables and places are read where PHP reads them)
fn keep_in_place(e: &Expr) -> bool {
    e.is_lvalue()
        || matches!(
            e.kind,
            ExprKind::Null
                | ExprKind::Bool(_)
                | ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Str(_)
                | ExprKind::Const(_)
                | ExprKind::NsConst(..)
                | ExprKind::Magic(_)
                | ExprKind::List(_)
                | ExprKind::Hole
        )
}

impl Lowerer {
    fn tmp(&mut self) -> String {
        self.next += 1;
        format!("\0gen{}", self.next)
    }

    fn block(&mut self, stmts: Vec<Stmt>) -> Result<Vec<Stmt>, ParseError> {
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            self.stmt(s, &mut out)?;
        }
        Ok(out)
    }

    fn stmt(&mut self, s: Stmt, out: &mut Vec<Stmt>) -> Result<(), ParseError> {
        if !stmt_has_yield(&s) {
            out.push(Stmt::Plain(Box::new(s)));
            return Ok(());
        }
        match s {
            Stmt::Echo(exprs) => {
                // Each operand is written before the next one is evaluated
                for e in exprs {
                    let e = self.expr(e, out)?;
                    out.push(Stmt::Echo(vec![e]));
                }
            }
            Stmt::Expr(e) => match e.kind {
                ExprKind::Yield { key, value } => {
                    let (key, value) = self.yield_operands(key, value, out)?;
                    out.push(Stmt::Yield { key, value, target: None });
                }
                ExprKind::YieldFrom(source) => {
                    let source = self.expr(*source, out)?;
                    out.push(Stmt::YieldFrom { source, target: None });
                }
                kind => {
                    let e = self.expr(Expr::new(kind, e.line), out)?;
                    if !matches!(&e.kind, ExprKind::Var(n) if is_hidden(n)) {
                        out.push(Stmt::Expr(e));
                    }
                }
            },
            Stmt::Block(body) => out.push(Stmt::Block(self.block(body)?)),
            Stmt::If(branches, else_body) => {
                if branches.iter().any(|(c, _)| has_yield(c)) {
                    let chain = self.if_chain(branches.into_iter().collect(), else_body)?;
                    out.extend(chain);
                } else {
                    let mut lowered = Vec::with_capacity(branches.len());
                    for (c, body) in branches {
                        lowered.push((c, self.block(body)?));
                    }
                    let else_body = else_body.map(|b| self.block(b)).transpose()?;
                    out.push(Stmt::If(lowered, else_body));
                }
            }
            Stmt::While(cond, body) => {
                if has_yield(&cond) {
                    // while (true) { <cond>; if (!cond) break; body }
                    let line = cond.line;
                    let mut inner = Vec::new();
                    let cond = self.expr(cond, &mut inner)?;
                    inner.push(break_unless(cond));
                    inner.extend(self.block(body)?);
                    out.push(Stmt::While(Expr::new(ExprKind::Bool(true), line), inner));
                } else {
                    out.push(Stmt::While(cond, self.block(body)?));
                }
            }
            Stmt::DoWhile(body, cond) => {
                if has_yield(&cond) {
                    // A flag skips the condition on the first pass, so `continue` still reaches it
                    let line = cond.line;
                    let first = self.tmp();
                    out.push(assign(&first, Expr::new(ExprKind::Bool(true), line)));
                    let mut check = Vec::new();
                    let cond = self.expr(cond, &mut check)?;
                    check.push(break_unless(cond));
                    let mut inner = vec![
                        if_then(not(var(&first, line)), check),
                        assign(&first, Expr::new(ExprKind::Bool(false), line)),
                    ];
                    inner.extend(self.block(body)?);
                    out.push(Stmt::While(Expr::new(ExprKind::Bool(true), line), inner));
                } else {
                    out.push(Stmt::DoWhile(self.block(body)?, cond));
                }
            }
            Stmt::For(init, cond, step, body) => {
                if init.iter().chain(&cond).chain(&step).any(has_yield) {
                    let line = init.iter().chain(&cond).chain(&step).next().map_or(0, |e| e.line);
                    for e in init {
                        self.stmt(Stmt::Expr(e), out)?;
                    }
                    let first = self.tmp();
                    out.push(assign(&first, Expr::new(ExprKind::Bool(true), line)));
                    let mut steps = Vec::new();
                    for e in step {
                        self.stmt(Stmt::Expr(e), &mut steps)?;
                    }
                    let mut inner = vec![
                        if_then(not(var(&first, line)), steps),
                        assign(&first, Expr::new(ExprKind::Bool(false), line)),
                    ];
                    // With several conditions, the last one decides
                    let n = cond.len();
                    for (i, e) in cond.into_iter().enumerate() {
                        if i + 1 < n {
                            self.stmt(Stmt::Expr(e), &mut inner)?;
                        } else {
                            let e = self.expr(e, &mut inner)?;
                            inner.push(break_unless(e));
                        }
                    }
                    inner.extend(self.block(body)?);
                    out.push(Stmt::While(Expr::new(ExprKind::Bool(true), line), inner));
                } else {
                    out.push(Stmt::For(init, cond, step, self.block(body)?));
                }
            }
            Stmt::Foreach { subject, key, value, by_ref, body } => {
                if key.as_ref().is_some_and(has_yield) || has_yield(&value) {
                    return Err(unsupported("a foreach target", value.line));
                }
                let subject = self.expr(subject, out)?;
                out.push(Stmt::Foreach { subject, key, value, by_ref, body: self.block(body)? });
            }
            Stmt::Switch(subject, cases) => {
                if cases.iter().any(|(c, _)| c.as_ref().is_some_and(has_yield)) {
                    return Err(unsupported("a case expression", subject.line));
                }
                let subject = self.expr(subject, out)?;
                let mut lowered = Vec::with_capacity(cases.len());
                for (c, body) in cases {
                    lowered.push((c, self.block(body)?));
                }
                out.push(Stmt::Switch(subject, lowered));
            }
            Stmt::Try { body, catches, finally } => {
                let body = self.block(body)?;
                let mut lowered = Vec::with_capacity(catches.len());
                for mut c in catches {
                    c.body = self.block(c.body)?;
                    lowered.push(c);
                }
                let finally = finally.map(|f| self.block(f)).transpose()?;
                out.push(Stmt::Try { body, catches: lowered, finally });
            }
            Stmt::Return(Some(e)) => {
                let e = self.expr(e, out)?;
                out.push(Stmt::Return(Some(e)));
            }
            Stmt::Unset(targets) => {
                let mut lowered = Vec::with_capacity(targets.len());
                for t in targets {
                    lowered.push(self.expr(t, out)?);
                }
                out.push(Stmt::Unset(lowered));
            }
            other => out.push(other),
        }
        Ok(())
    }

    /// `if (a) ... elseif (b) ...` where a condition yields: each condition is
    /// evaluated only once the ones before it failed
    fn if_chain(
        &mut self,
        mut branches: std::collections::VecDeque<(Expr, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
    ) -> Result<Vec<Stmt>, ParseError> {
        let (cond, body) = branches.pop_front().expect("if has a branch");
        let mut out = Vec::new();
        let cond = self.expr(cond, &mut out)?;
        let body = self.block(body)?;
        let rest = if branches.is_empty() {
            else_body.map(|b| self.block(b)).transpose()?
        } else {
            Some(self.if_chain(branches, else_body)?)
        };
        out.push(Stmt::If(vec![(cond, body)], rest));
        Ok(out)
    }

    fn yield_operands(
        &mut self,
        key: Option<Box<Expr>>,
        value: Option<Box<Expr>>,
        pre: &mut Vec<Stmt>,
    ) -> Result<(Option<Expr>, Option<Expr>), ParseError> {
        let mut key = key.map(|k| *k);
        if let Some(k) = key.take() {
            let k = self.expr(k, pre)?;
            key = Some(if value.as_deref().is_some_and(has_yield) { self.hoist(k, pre) } else { k });
        }
        let value = value.map(|v| self.expr(*v, pre)).transpose()?;
        Ok((key, value))
    }

    /// Evaluates `e` now into a hidden variable, unless it can wait
    fn hoist(&mut self, e: Expr, pre: &mut Vec<Stmt>) -> Expr {
        if keep_in_place(&e) {
            return e;
        }
        let line = e.line;
        let t = self.tmp();
        pre.push(assign(&t, e));
        var(&t, line)
    }

    /// The expression without yields, with the statements to run before it in `pre`
    fn expr(&mut self, e: Expr, pre: &mut Vec<Stmt>) -> Result<Expr, ParseError> {
        if !has_yield(&e) {
            return Ok(e);
        }
        let line = e.line;
        match e.kind {
            ExprKind::Yield { key, value } => {
                let (key, value) = self.yield_operands(key, value, pre)?;
                let t = self.tmp();
                pre.push(Stmt::Yield { key, value, target: Some(t.clone()) });
                Ok(var(&t, line))
            }
            ExprKind::YieldFrom(source) => {
                let source = self.expr(*source, pre)?;
                let t = self.tmp();
                pre.push(Stmt::YieldFrom { source, target: Some(t.clone()) });
                Ok(var(&t, line))
            }
            ExprKind::Binary(op @ ("&&" | "||"), l, r) if has_yield(&r) => {
                let l = self.expr(*l, pre)?;
                let t = self.tmp();
                pre.push(assign(&t, bool_cast(l)));
                let mut inner = Vec::new();
                let r = self.expr(*r, &mut inner)?;
                inner.push(assign(&t, bool_cast(r)));
                let cond = if op == "&&" { var(&t, line) } else { not(var(&t, line)) };
                pre.push(if_then(cond, inner));
                Ok(var(&t, line))
            }
            ExprKind::Binary("??", l, r) if has_yield(&r) => {
                let l = self.expr(*l, pre)?;
                let t = self.tmp();
                let null = Expr::new(ExprKind::Null, line);
                pre.push(assign(&t, Expr::new(ExprKind::Binary("??", Box::new(l), Box::new(null.clone())), line)));
                let mut inner = Vec::new();
                let r = self.expr(*r, &mut inner)?;
                inner.push(assign(&t, r));
                let is_null = Expr::new(ExprKind::Binary("===", Box::new(var(&t, line)), Box::new(null)), line);
                pre.push(if_then(is_null, inner));
                Ok(var(&t, line))
            }
            ExprKind::CompoundAssign("??", target, r) if has_yield(&r) => {
                let target = self.expr(*target, pre)?;
                let mut inner = Vec::new();
                let r = self.expr(*r, &mut inner)?;
                inner.push(Stmt::Expr(Expr::new(ExprKind::Assign(Box::new(target.clone()), Box::new(r)), line)));
                let unset = not(Expr::new(ExprKind::Isset(vec![target.clone()]), line));
                pre.push(if_then(unset, inner));
                Ok(target)
            }
            ExprKind::Ternary(c, then, otherwise)
                if then.as_deref().is_some_and(has_yield) || has_yield(&otherwise) =>
            {
                let c = self.expr(*c, pre)?;
                let t = self.tmp();
                let mut else_body = Vec::new();
                let otherwise = self.expr(*otherwise, &mut else_body)?;
                else_body.push(assign(&t, otherwise));
                match then {
                    Some(then) => {
                        let mut then_body = Vec::new();
                        let then = self.expr(*then, &mut then_body)?;
                        then_body.push(assign(&t, then));
                        pre.push(Stmt::If(vec![(c, then_body)], Some(else_body)));
                    }
                    None => {
                        pre.push(assign(&t, c));
                        pre.push(if_then(not(var(&t, line)), else_body));
                    }
                }
                Ok(var(&t, line))
            }
            ExprKind::Match(subject, arms) if arms.iter().any(|a| has_yield(&a.body)) => {
                if arms.iter().any(|a| a.conds.iter().flatten().any(has_yield)) {
                    return Err(unsupported("a match condition", line));
                }
                let subject = self.expr(*subject, pre)?;
                let s = self.tmp();
                pre.push(assign(&s, subject));
                let t = self.tmp();
                let (default, arms): (Vec<MatchArm>, Vec<MatchArm>) = arms.into_iter().partition(|a| a.conds.is_none());
                // Nothing matched: a match without arms raises UnhandledMatchError for the subject
                let mut chain = match default.into_iter().next() {
                    Some(arm) => {
                        let mut body = Vec::new();
                        let v = self.expr(arm.body, &mut body)?;
                        body.push(assign(&t, v));
                        body
                    }
                    None => vec![assign(&t, Expr::new(ExprKind::Match(Box::new(var(&s, line)), Vec::new()), line))],
                };
                for arm in arms.into_iter().rev() {
                    let conds = arm.conds.unwrap_or_default();
                    let cond = conds
                        .into_iter()
                        .map(|c| {
                            let cl = c.line;
                            Expr::new(ExprKind::Binary("===", Box::new(var(&s, cl)), Box::new(c)), cl)
                        })
                        .reduce(|a, b| {
                            let l = a.line;
                            Expr::new(ExprKind::Binary("||", Box::new(a), Box::new(b)), l)
                        })
                        .unwrap_or(Expr::new(ExprKind::Bool(false), line));
                    let mut body = Vec::new();
                    let v = self.expr(arm.body, &mut body)?;
                    body.push(assign(&t, v));
                    chain = vec![Stmt::If(vec![(cond, body)], Some(chain))];
                }
                pre.extend(chain);
                Ok(var(&t, line))
            }
            kind => {
                // Children in evaluation order: those before the last one that yields
                // are evaluated now, the yielding one is lowered, the rest can wait
                let mut e = Expr::new(kind, line);
                let mut flags = Vec::new();
                each_child(&e, &mut |c| flags.push(has_yield(c)));
                let last = flags.iter().rposition(|&f| f).unwrap_or(0);
                let mut i = 0;
                let mut err = None;
                each_child_mut(&mut e, &mut |c| {
                    if i <= last && err.is_none() {
                        let taken = std::mem::replace(c, Expr::new(ExprKind::Null, c.line));
                        match self.expr(taken, pre) {
                            Ok(lowered) => *c = if i < last { self.hoist(lowered, pre) } else { lowered },
                            Err(e) => err = Some(e),
                        }
                    }
                    i += 1;
                });
                match err {
                    Some(e) => Err(e),
                    None => Ok(e),
                }
            }
        }
    }
}

pub fn has_yield(e: &Expr) -> bool {
    if matches!(e.kind, ExprKind::Yield { .. } | ExprKind::YieldFrom(_)) {
        return true;
    }
    let mut found = false;
    each_child(e, &mut |c| found = found || has_yield(c));
    found
}

fn block_has_yield(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_has_yield)
}

/// Whether the statement yields itself. Nested functions, classes and closures
/// are separate functions
pub fn stmt_has_yield(s: &Stmt) -> bool {
    match s {
        Stmt::Echo(exprs) | Stmt::Unset(exprs) => exprs.iter().any(has_yield),
        Stmt::Expr(e) | Stmt::Return(Some(e)) => has_yield(e),
        Stmt::If(branches, else_body) => {
            branches.iter().any(|(c, b)| has_yield(c) || block_has_yield(b))
                || else_body.as_deref().is_some_and(block_has_yield)
        }
        Stmt::While(c, body) | Stmt::DoWhile(body, c) => has_yield(c) || block_has_yield(body),
        Stmt::For(init, cond, step, body) => {
            init.iter().chain(cond).chain(step).any(has_yield) || block_has_yield(body)
        }
        Stmt::Foreach { subject, key, value, body, .. } => {
            has_yield(subject) || key.as_ref().is_some_and(has_yield) || has_yield(value) || block_has_yield(body)
        }
        Stmt::Block(body) => block_has_yield(body),
        Stmt::Switch(subject, cases) => {
            has_yield(subject) || cases.iter().any(|(c, b)| c.as_ref().is_some_and(has_yield) || block_has_yield(b))
        }
        Stmt::Try { body, catches, finally } => {
            block_has_yield(body)
                || catches.iter().any(|c| block_has_yield(&c.body))
                || finally.as_deref().is_some_and(block_has_yield)
        }
        Stmt::Yield { .. } | Stmt::YieldFrom { .. } => true,
        Stmt::Return(None)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Function(_)
        | Stmt::Class(_)
        | Stmt::Global(_)
        | Stmt::StaticVar(_)
        | Stmt::Const(_)
        | Stmt::StrictTypes
        | Stmt::Plain(_) => false,
    }
}

/// The direct subexpressions in evaluation order, not looking into closures
fn each_child(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    let class = |c: &ClassRef, f: &mut dyn FnMut(&Expr)| {
        if let ClassRef::Dynamic(e) = c {
            f(e);
        }
    };
    let name = |n: &Name, f: &mut dyn FnMut(&Expr)| {
        if let Name::Dyn(e) = n {
            f(e);
        }
    };
    let items = |items: &[ArrayItem], f: &mut dyn FnMut(&Expr)| {
        for item in items {
            if let Some(k) = &item.key {
                f(k);
            }
            f(&item.value);
        }
    };
    match &e.kind {
        ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Var(_)
        | ExprKind::Const(_)
        | ExprKind::NsConst(..)
        | ExprKind::Closure(_)
        | ExprKind::Hole
        | ExprKind::Magic(_) => {}
        ExprKind::Interp(parts) | ExprKind::Isset(parts) => parts.iter().for_each(f),
        ExprKind::Array(it) => items(it, f),
        ExprKind::Index(base, index) => {
            f(base);
            if let Some(i) = index {
                f(i);
            }
        }
        ExprKind::Assign(a, b) | ExprKind::AssignRef(a, b) | ExprKind::CompoundAssign(_, a, b) | ExprKind::Binary(_, a, b) => {
            f(a);
            f(b);
        }
        ExprKind::IncDec { target: x, .. }
        | ExprKind::Unary(_, x)
        | ExprKind::Empty(x)
        | ExprKind::Throw(x)
        | ExprKind::Clone(x)
        | ExprKind::Silence(x)
        | ExprKind::Include(_, x)
        | ExprKind::CallableFrom(x)
        | ExprKind::YieldFrom(x) => f(x),
        ExprKind::Exit(x) => {
            if let Some(x) = x {
                f(x);
            }
        }
        ExprKind::Ternary(c, t, o) => {
            f(c);
            if let Some(t) = t {
                f(t);
            }
            f(o);
        }
        ExprKind::Call(_, args) | ExprKind::NsCall(_, _, args) => args.iter().for_each(|a| f(&a.value)),
        ExprKind::CallValue(callee, args) => {
            f(callee);
            args.iter().for_each(|a| f(&a.value));
        }
        ExprKind::New(c, args) | ExprKind::StaticCall { class: c, args, .. } => {
            class(c, f);
            args.iter().for_each(|a| f(&a.value));
        }
        ExprKind::MethodCall { object, name: n, args, .. } => {
            f(object);
            name(n, f);
            args.iter().for_each(|a| f(&a.value));
        }
        ExprKind::Prop { object, name: n, .. } => {
            f(object);
            name(n, f);
        }
        ExprKind::StaticProp { class: c, .. } | ExprKind::ClassConst { class: c, .. } => class(c, f),
        ExprKind::InstanceOf(x, c) => {
            f(x);
            class(c, f);
        }
        ExprKind::Match(subject, arms) => {
            f(subject);
            for arm in arms {
                arm.conds.iter().flatten().for_each(&mut *f);
                f(&arm.body);
            }
        }
        ExprKind::List(list) => {
            for item in list.iter().flatten() {
                if let Some(k) = &item.key {
                    f(k);
                }
                f(&item.target);
            }
        }
        ExprKind::Yield { key, value } => {
            if let Some(k) = key {
                f(k);
            }
            if let Some(v) = value {
                f(v);
            }
        }
    }
}

/// each_child, mutably (same order)
fn each_child_mut(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    fn class(c: &mut ClassRef, f: &mut dyn FnMut(&mut Expr)) {
        if let ClassRef::Dynamic(e) = c {
            f(e);
        }
    }
    fn name(n: &mut Name, f: &mut dyn FnMut(&mut Expr)) {
        if let Name::Dyn(e) = n {
            f(e);
        }
    }
    match &mut e.kind {
        ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Var(_)
        | ExprKind::Const(_)
        | ExprKind::NsConst(..)
        | ExprKind::Closure(_)
        | ExprKind::Hole
        | ExprKind::Magic(_) => {}
        ExprKind::Interp(parts) | ExprKind::Isset(parts) => parts.iter_mut().for_each(f),
        ExprKind::Array(items) => {
            for item in items {
                if let Some(k) = &mut item.key {
                    f(k);
                }
                f(&mut item.value);
            }
        }
        ExprKind::Index(base, index) => {
            f(base);
            if let Some(i) = index {
                f(i);
            }
        }
        ExprKind::Assign(a, b) | ExprKind::AssignRef(a, b) | ExprKind::CompoundAssign(_, a, b) | ExprKind::Binary(_, a, b) => {
            f(a);
            f(b);
        }
        ExprKind::IncDec { target: x, .. }
        | ExprKind::Unary(_, x)
        | ExprKind::Empty(x)
        | ExprKind::Throw(x)
        | ExprKind::Clone(x)
        | ExprKind::Silence(x)
        | ExprKind::Include(_, x)
        | ExprKind::CallableFrom(x)
        | ExprKind::YieldFrom(x) => f(x),
        ExprKind::Exit(x) => {
            if let Some(x) = x {
                f(x);
            }
        }
        ExprKind::Ternary(c, t, o) => {
            f(c);
            if let Some(t) = t {
                f(t);
            }
            f(o);
        }
        ExprKind::Call(_, args) | ExprKind::NsCall(_, _, args) => args.iter_mut().for_each(|a| f(&mut a.value)),
        ExprKind::CallValue(callee, args) => {
            f(callee);
            args.iter_mut().for_each(|a| f(&mut a.value));
        }
        ExprKind::New(c, args) | ExprKind::StaticCall { class: c, args, .. } => {
            class(c, f);
            args.iter_mut().for_each(|a| f(&mut a.value));
        }
        ExprKind::MethodCall { object, name: n, args, .. } => {
            f(object);
            name(n, f);
            args.iter_mut().for_each(|a| f(&mut a.value));
        }
        ExprKind::Prop { object, name: n, .. } => {
            f(object);
            name(n, f);
        }
        ExprKind::StaticProp { class: c, .. } | ExprKind::ClassConst { class: c, .. } => class(c, f),
        ExprKind::InstanceOf(x, c) => {
            f(x);
            class(c, f);
        }
        ExprKind::Match(subject, arms) => {
            f(subject);
            for arm in arms {
                arm.conds.iter_mut().flatten().for_each(&mut *f);
                f(&mut arm.body);
            }
        }
        ExprKind::List(list) => {
            for item in list.iter_mut().flatten() {
                if let Some(k) = &mut item.key {
                    f(k);
                }
                f(&mut item.target);
            }
        }
        ExprKind::Yield { key, value } => {
            if let Some(k) = key {
                f(k);
            }
            if let Some(v) = value {
                f(v);
            }
        }
    }
}
