//! Builds the AST from tokens. Expressions use a Pratt parser with PHP 8 precedence.

use std::rc::Rc;

use crate::ast::{Arg, ArrayItem, Expr, ExprKind, FuncDecl, Param, Stmt, TypeHint};
use crate::lexer::{StrPart, Tok, Token};

#[derive(Debug)]
pub struct ParseError {
    pub msg: String,
    pub line: u32,
}

type PResult<T> = Result<T, ParseError>;

/// Nesting limit for expressions and statements. The parser is recursive, so
/// without it a pathological input would overflow the stack. php-src's bison
/// parser stops at a similar depth with the same message
const MAX_NESTING: u32 = 5000;

// Higher numbers bind tighter
const P_OR: u8 = 1;
const P_XOR: u8 = 2;
const P_AND: u8 = 3;
const P_ASSIGN: u8 = 4;
const P_TERNARY: u8 = 5;
const P_COALESCE: u8 = 6;
const P_LOR: u8 = 7;
const P_LAND: u8 = 8;
const P_BOR: u8 = 9;
const P_BXOR: u8 = 10;
const P_BAND: u8 = 11;
const P_EQUALITY: u8 = 12;
const P_COMPARE: u8 = 13;
const P_CONCAT: u8 = 14;
const P_SHIFT: u8 = 15;
const P_ADD: u8 = 16;
const P_MUL: u8 = 17;
const P_NOT: u8 = 18;
const P_UNARY: u8 = 19;
const P_POW: u8 = 20;

enum Assoc {
    Left,
    Right,
}

fn binary_op(tok: &Tok) -> Option<(&'static str, u8, Assoc)> {
    let op = match tok {
        Tok::Op(op) => *op,
        Tok::Ident(kw) => match kw.to_ascii_lowercase().as_str() {
            "or" => return Some(("||", P_OR, Assoc::Left)),
            "xor" => return Some(("xor", P_XOR, Assoc::Left)),
            "and" => return Some(("&&", P_AND, Assoc::Left)),
            _ => return None,
        },
        _ => return None,
    };
    let (prec, assoc) = match op {
        "??" => (P_COALESCE, Assoc::Right),
        "||" => (P_LOR, Assoc::Left),
        "&&" => (P_LAND, Assoc::Left),
        "|" => (P_BOR, Assoc::Left),
        "^" => (P_BXOR, Assoc::Left),
        "&" => (P_BAND, Assoc::Left),
        "==" | "!=" | "<>" | "===" | "!==" | "<=>" => (P_EQUALITY, Assoc::Left),
        "<" | "<=" | ">" | ">=" => (P_COMPARE, Assoc::Left),
        "." => (P_CONCAT, Assoc::Left),
        "<<" | ">>" => (P_SHIFT, Assoc::Left),
        "+" | "-" => (P_ADD, Assoc::Left),
        "*" | "/" | "%" => (P_MUL, Assoc::Left),
        "**" => (P_POW, Assoc::Right),
        _ => return None,
    };
    let op = if op == "<>" { "!=" } else { op };
    Some((op, prec, assoc))
}

fn assign_op(tok: &Tok) -> Option<&'static str> {
    match tok {
        Tok::Op("=") => Some("="),
        Tok::Op("+=") => Some("+"),
        Tok::Op("-=") => Some("-"),
        Tok::Op("*=") => Some("*"),
        Tok::Op("/=") => Some("/"),
        Tok::Op(".=") => Some("."),
        Tok::Op("%=") => Some("%"),
        Tok::Op("**=") => Some("**"),
        Tok::Op("&=") => Some("&"),
        Tok::Op("|=") => Some("|"),
        Tok::Op("^=") => Some("^"),
        Tok::Op("<<=") => Some("<<"),
        Tok::Op(">>=") => Some(">>"),
        Tok::Op("??=") => Some("??"),
        _ => None,
    }
}

fn cast_op(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "int" | "integer" => "(int)",
        "float" | "double" => "(float)",
        "string" => "(string)",
        "bool" | "boolean" => "(bool)",
        _ => return None,
    })
}

/// Parses the tokens of an interpolated `{$...}` / `$a[k]` inside a string
fn parse_embedded(mut tokens: Vec<Token>) -> PResult<Expr> {
    let line = tokens.last().map_or(0, |t| t.line);
    tokens.push(Token { tok: Tok::Eof, line });
    let mut p = Parser { tokens, pos: 0, depth: 0 };
    let e = p.expr()?;
    if !p.at(&Tok::Eof) {
        return p.unexpected();
    }
    Ok(e)
}

pub fn parse(tokens: Vec<Token>) -> PResult<Vec<Stmt>> {
    let mut p = Parser { tokens, pos: 0, depth: 0 };
    let mut stmts = Vec::new();
    while !p.at(&Tok::Eof) {
        stmts.push(p.statement()?);
    }
    Ok(stmts)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: u32,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn peek_at(&self, off: usize) -> &Tok {
        let i = (self.pos + off).min(self.tokens.len() - 1);
        &self.tokens[i].tok
    }

    fn line(&self) -> u32 {
        self.tokens[self.pos].line
    }

    fn next(&mut self) -> Tok {
        let tok = self.tokens[self.pos].tok.clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn at(&self, tok: &Tok) -> bool {
        self.peek() == tok
    }

    fn at_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case(kw))
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.at(tok) {
            self.next();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: Tok) -> PResult<()> {
        if self.eat(&tok) {
            Ok(())
        } else {
            self.unexpected()
        }
    }

    fn unexpected<T>(&self) -> PResult<T> {
        let what = match self.peek() {
            Tok::Eof => "end of file".to_string(),
            Tok::Variable(v) => format!("variable \"${v}\""),
            Tok::Ident(s) => format!("identifier \"{s}\""),
            Tok::Op(op) => format!("token \"{op}\""),
            Tok::Semicolon => "token \";\"".to_string(),
            Tok::Int(_) | Tok::Float(_) => "number".to_string(),
            Tok::Str(_) | Tok::Template(_) => "string".to_string(),
            Tok::InlineHtml(_) => "inline html".to_string(),
        };
        Err(ParseError { msg: format!("syntax error, unexpected {what}"), line: self.line() })
    }

    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        if self.depth >= MAX_NESTING {
            return Err(ParseError { msg: "memory exhausted".into(), line: self.line() });
        }
        self.depth += 1;
        let out = f(self);
        self.depth -= 1;
        out
    }

    fn statement(&mut self) -> PResult<Stmt> {
        self.nested(Self::statement_inner)
    }

    fn statement_inner(&mut self) -> PResult<Stmt> {
        match self.peek().clone() {
            Tok::InlineHtml(text) => {
                let line = self.line();
                self.next();
                Ok(Stmt::Echo(vec![Expr::new(ExprKind::Str(Rc::new(text)), line)]))
            }
            Tok::Semicolon => {
                self.next();
                Ok(Stmt::Block(Vec::new()))
            }
            Tok::Op("{") => {
                self.next();
                let mut body = Vec::new();
                while !self.eat(&Tok::Op("}")) {
                    if self.at(&Tok::Eof) {
                        return self.unexpected();
                    }
                    body.push(self.statement()?);
                }
                Ok(Stmt::Block(body))
            }
            Tok::Ident(kw) => match kw.to_ascii_lowercase().as_str() {
                "echo" => {
                    self.next();
                    let mut args = vec![self.expr()?];
                    while self.eat(&Tok::Op(",")) {
                        args.push(self.expr()?);
                    }
                    self.end_statement()?;
                    Ok(Stmt::Echo(args))
                }
                "if" => self.if_statement(),
                "while" => {
                    self.next();
                    let cond = self.paren_expr()?;
                    let body = self.body()?;
                    Ok(Stmt::While(cond, body))
                }
                "do" => {
                    self.next();
                    let body = self.body()?;
                    if !self.eat_kw("while") {
                        return self.unexpected();
                    }
                    let cond = self.paren_expr()?;
                    self.end_statement()?;
                    Ok(Stmt::DoWhile(body, cond))
                }
                "for" => self.for_statement(),
                "foreach" => self.foreach_statement(),
                "function" if matches!(self.peek_at(1), Tok::Ident(_)) || matches!(self.peek_at(1), Tok::Op("&")) => {
                    self.function_decl()
                }
                "return" => {
                    self.next();
                    let value = if self.at(&Tok::Semicolon) { None } else { Some(self.expr()?) };
                    self.end_statement()?;
                    Ok(Stmt::Return(value))
                }
                "global" => {
                    self.next();
                    let mut names = Vec::new();
                    loop {
                        match self.next() {
                            Tok::Variable(name) => names.push(name),
                            _ => {
                                self.pos -= 1;
                                return self.unexpected();
                            }
                        }
                        if !self.eat(&Tok::Op(",")) {
                            break;
                        }
                    }
                    self.end_statement()?;
                    Ok(Stmt::Global(names))
                }
                "static" if matches!(self.peek_at(1), Tok::Variable(_)) => {
                    self.next();
                    let mut vars = Vec::new();
                    loop {
                        let Tok::Variable(name) = self.next() else {
                            self.pos -= 1;
                            return self.unexpected();
                        };
                        let init = if self.eat(&Tok::Op("=")) { Some(self.expr()?) } else { None };
                        vars.push((name, init));
                        if !self.eat(&Tok::Op(",")) {
                            break;
                        }
                    }
                    self.end_statement()?;
                    Ok(Stmt::StaticVar(vars))
                }
                "unset" => {
                    self.next();
                    self.expect(Tok::Op("("))?;
                    let targets = self.expr_list(&Tok::Op(")"))?;
                    self.expect(Tok::Op(")"))?;
                    self.end_statement()?;
                    Ok(Stmt::Unset(targets))
                }
                "break" | "continue" => {
                    let is_break = kw.eq_ignore_ascii_case("break");
                    self.next();
                    let level = match self.peek() {
                        Tok::Int(n) if *n >= 1 => {
                            let n = *n;
                            self.next();
                            u32::try_from(n).map_err(|_| ParseError {
                                msg: "Cannot 'break' that many levels".into(),
                                line: self.line(),
                            })?
                        }
                        Tok::Int(_) => {
                            return Err(ParseError {
                                msg: format!("'{}' operator accepts only positive integers", kw.to_ascii_lowercase()),
                                line: self.line(),
                            })
                        }
                        _ => 1,
                    };
                    self.end_statement()?;
                    Ok(if is_break { Stmt::Break(level) } else { Stmt::Continue(level) })
                }
                _ => self.expr_statement(),
            },
            _ => self.expr_statement(),
        }
    }

    fn expr_statement(&mut self) -> PResult<Stmt> {
        let e = self.expr()?;
        self.end_statement()?;
        Ok(Stmt::Expr(e))
    }

    /// Statements end with `;`. A `?>` also acts as `;`, so the lexer emits a Semicolon for it
    fn end_statement(&mut self) -> PResult<()> {
        if self.eat(&Tok::Semicolon) {
            return Ok(());
        }
        self.unexpected()
    }

    fn paren_expr(&mut self) -> PResult<Expr> {
        self.expect(Tok::Op("("))?;
        let e = self.expr()?;
        self.expect(Tok::Op(")"))?;
        Ok(e)
    }

    /// Body of a control structure, normalized to Vec<Stmt> for both blocks and single statements
    fn body(&mut self) -> PResult<Vec<Stmt>> {
        match self.statement()? {
            Stmt::Block(stmts) => Ok(stmts),
            s => Ok(vec![s]),
        }
    }

    fn if_statement(&mut self) -> PResult<Stmt> {
        self.next();
        let mut branches = vec![(self.paren_expr()?, self.body()?)];
        let mut else_body = None;
        loop {
            if self.eat_kw("elseif") {
                branches.push((self.paren_expr()?, self.body()?));
            } else if self.at_kw("else") && matches!(self.peek_at(1), Tok::Ident(s) if s.eq_ignore_ascii_case("if")) {
                self.next();
                self.next();
                branches.push((self.paren_expr()?, self.body()?));
            } else if self.eat_kw("else") {
                else_body = Some(self.body()?);
                break;
            } else {
                break;
            }
        }
        Ok(Stmt::If(branches, else_body))
    }

    fn for_statement(&mut self) -> PResult<Stmt> {
        self.next();
        self.expect(Tok::Op("("))?;
        let init = self.expr_list(&Tok::Semicolon)?;
        self.expect(Tok::Semicolon)?;
        let cond = self.expr_list(&Tok::Semicolon)?;
        self.expect(Tok::Semicolon)?;
        let step = self.expr_list(&Tok::Op(")"))?;
        self.expect(Tok::Op(")"))?;
        let body = self.body()?;
        Ok(Stmt::For(init, cond, step, body))
    }

    fn foreach_statement(&mut self) -> PResult<Stmt> {
        self.next();
        self.expect(Tok::Op("("))?;
        let subject = self.expr()?;
        if !self.eat_kw("as") {
            return self.unexpected();
        }
        let mut by_ref = self.eat(&Tok::Op("&"));
        let mut value = self.foreach_target()?;
        let mut key = None;
        if self.eat(&Tok::Op("=>")) {
            if by_ref {
                return Err(ParseError { msg: "Key element cannot be a reference".into(), line: self.line() });
            }
            key = Some(value);
            by_ref = self.eat(&Tok::Op("&"));
            value = self.foreach_target()?;
        }
        if by_ref && !subject.is_lvalue() {
            return Err(ParseError {
                msg: "Cannot create references to elements of a temporary array expression".into(),
                line: self.line(),
            });
        }
        self.expect(Tok::Op(")"))?;
        let body = self.body()?;
        Ok(Stmt::Foreach { subject, key, value, by_ref, body })
    }

    fn function_decl(&mut self) -> PResult<Stmt> {
        let line = self.line();
        self.next();
        // Returning by reference is accepted but treated as by value
        self.eat(&Tok::Op("&"));
        let Tok::Ident(name) = self.next() else {
            self.pos -= 1;
            return self.unexpected();
        };
        self.expect(Tok::Op("("))?;
        let mut params: Vec<Param> = Vec::new();
        while !self.eat(&Tok::Op(")")) {
            let ty = if matches!(self.peek(), Tok::Variable(_) | Tok::Op("&" | "...")) { None } else { Some(self.type_hint()?) };
            let by_ref = self.eat(&Tok::Op("&"));
            let variadic = self.eat(&Tok::Op("..."));
            let Tok::Variable(pname) = self.next() else {
                self.pos -= 1;
                return self.unexpected();
            };
            if params.iter().any(|p| p.name == pname) {
                return Err(ParseError { msg: format!("Redefinition of parameter ${pname}"), line: self.line() });
            }
            let default = if self.eat(&Tok::Op("=")) { Some(self.expr()?) } else { None };
            if variadic && default.is_some() {
                return Err(ParseError { msg: "Variadic parameter cannot have a default value".into(), line: self.line() });
            }
            params.push(Param { name: pname, ty, default, by_ref, variadic });
            if variadic && !self.at(&Tok::Op(")")) && !(self.at(&Tok::Op(",")) && self.peek_at(1) == &Tok::Op(")")) {
                return Err(ParseError { msg: "Only the last parameter can be variadic".into(), line: self.line() });
            }
            if !self.eat(&Tok::Op(",")) {
                self.expect(Tok::Op(")"))?;
                break;
            }
        }
        let ret = if self.eat(&Tok::Op(":")) { Some(self.type_hint()?) } else { None };
        self.expect(Tok::Op("{"))?;
        let mut body = Vec::new();
        while !self.eat(&Tok::Op("}")) {
            if self.at(&Tok::Eof) {
                return self.unexpected();
            }
            body.push(self.statement()?);
        }
        Ok(Stmt::Function(Rc::new(FuncDecl { name, params, ret, body, line })))
    }

    /// `int`, `?string`, `int|float|null`
    fn type_hint(&mut self) -> PResult<TypeHint> {
        let mut display = String::new();
        let mut types = Vec::new();
        if self.eat(&Tok::Op("?")) {
            display.push('?');
            types.push("null".to_string());
        }
        loop {
            let Tok::Ident(name) = self.next() else {
                self.pos -= 1;
                return self.unexpected();
            };
            display.push_str(&name);
            types.push(name.to_ascii_lowercase());
            if !self.eat(&Tok::Op("|")) {
                break;
            }
            display.push('|');
        }
        Ok(TypeHint { types, display })
    }

    fn foreach_target(&mut self) -> PResult<Expr> {
        let target = self.postfix_expr()?;
        if !target.is_lvalue() {
            return Err(ParseError { msg: "syntax error, foreach target must be a variable".into(), line: self.line() });
        }
        Ok(target)
    }

    /// Array literal items up to `end`. Trailing commas are allowed
    fn array_items(&mut self, end: &Tok) -> PResult<Vec<ArrayItem>> {
        let mut items = Vec::new();
        while !self.eat(end) {
            if self.at(&Tok::Op(",")) {
                return Err(ParseError { msg: "Cannot use empty array elements in arrays".into(), line: self.line() });
            }
            let first = self.expr()?;
            let item = if self.eat(&Tok::Op("=>")) {
                ArrayItem { key: Some(first), value: self.expr()? }
            } else {
                ArrayItem { key: None, value: first }
            };
            items.push(item);
            if !self.eat(&Tok::Op(",")) {
                self.expect(end.clone())?;
                break;
            }
        }
        Ok(items)
    }

    /// Call arguments after `(`, through the closing `)`. Trailing commas are allowed
    fn call_args(&mut self) -> PResult<Vec<Arg>> {
        let mut args = Vec::new();
        while !self.eat(&Tok::Op(")")) {
            let spread = self.eat(&Tok::Op("..."));
            if !spread && args.last().is_some_and(|a: &Arg| a.spread) {
                return Err(ParseError {
                    msg: "Cannot use positional argument after argument unpacking".into(),
                    line: self.line(),
                });
            }
            args.push(Arg { value: self.expr()?, spread });
            if !self.eat(&Tok::Op(",")) {
                self.expect(Tok::Op(")"))?;
                break;
            }
        }
        Ok(args)
    }

    fn expr_list(&mut self, end: &Tok) -> PResult<Vec<Expr>> {
        let mut list = Vec::new();
        if self.at(end) {
            return Ok(list);
        }
        list.push(self.expr()?);
        while self.eat(&Tok::Op(",")) {
            list.push(self.expr()?);
        }
        Ok(list)
    }

    fn expr(&mut self) -> PResult<Expr> {
        self.expr_bp(0)
    }

    fn expr_bp(&mut self, min_prec: u8) -> PResult<Expr> {
        self.nested(|p| p.expr_bp_inner(min_prec))
    }

    fn expr_bp_inner(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.prefix()?;
        loop {
            // Assignment applies only when the LHS is a variable, and takes the whole RHS regardless of precedence (e.g. `!$a = f()`)
            if lhs.is_lvalue() {
                if self.at(&Tok::Op("=")) && self.peek_at(1) == &Tok::Op("&") {
                    self.next();
                    self.next();
                    let source = self.postfix_expr()?;
                    if !source.is_lvalue() {
                        return Err(ParseError {
                            msg: "Cannot assign reference to non referenceable value".into(),
                            line: self.line(),
                        });
                    }
                    let line = lhs.line;
                    lhs = Expr::new(ExprKind::AssignRef(Box::new(lhs), Box::new(source)), line);
                    continue;
                }
                if let Some(op) = assign_op(self.peek()) {
                    self.next();
                    let rhs = Box::new(self.expr_bp(P_ASSIGN)?);
                    let line = lhs.line;
                    let target = Box::new(lhs);
                    let kind = if op == "=" { ExprKind::Assign(target, rhs) } else { ExprKind::CompoundAssign(op, target, rhs) };
                    lhs = Expr::new(kind, line);
                    continue;
                }
                if matches!(self.peek(), Tok::Op("++" | "--")) {
                    let inc = self.next() == Tok::Op("++");
                    let line = lhs.line;
                    lhs = Expr::new(ExprKind::IncDec { target: Box::new(lhs), inc, prefix: false }, line);
                    continue;
                }
            }

            if self.at(&Tok::Op("?")) {
                if P_TERNARY < min_prec {
                    break;
                }
                self.next();
                let then = if self.eat(&Tok::Op(":")) {
                    None
                } else {
                    let e = self.expr_bp(P_ASSIGN)?;
                    self.expect(Tok::Op(":"))?;
                    Some(Box::new(e))
                };
                let otherwise = self.expr_bp(P_TERNARY + 1)?;
                let line = lhs.line;
                lhs = Expr::new(ExprKind::Ternary(Box::new(lhs), then, Box::new(otherwise)), line);
                continue;
            }

            let Some((op, prec, assoc)) = binary_op(self.peek()) else { break };
            if prec < min_prec {
                break;
            }
            self.next();
            let next_min = match assoc {
                Assoc::Left => prec + 1,
                Assoc::Right => prec,
            };
            let rhs = self.expr_bp(next_min)?;
            let line = lhs.line;
            lhs = Expr::new(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), line);
        }
        Ok(lhs)
    }

    /// A primary expression followed by any `[...]` offsets
    fn postfix_expr(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        while self.eat(&Tok::Op("[")) {
            let index = if self.at(&Tok::Op("]")) { None } else { Some(Box::new(self.expr()?)) };
            self.expect(Tok::Op("]"))?;
            let line = e.line;
            e = Expr::new(ExprKind::Index(Box::new(e), index), line);
        }
        Ok(e)
    }

    fn prefix(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let line = self.line();
        let kind = match self.peek().clone() {
            Tok::Op("(") if matches!((self.peek_at(1), self.peek_at(2)), (Tok::Ident(name), Tok::Op(")")) if cast_op(name).is_some()) => {
                self.next();
                let Tok::Ident(name) = self.next() else { unreachable!() };
                self.next();
                ExprKind::Unary(cast_op(&name).unwrap(), Box::new(self.expr_bp(P_UNARY)?))
            }
            Tok::Op(op @ ("-" | "+" | "~")) => {
                self.next();
                ExprKind::Unary(op, Box::new(self.expr_bp(P_UNARY)?))
            }
            Tok::Op("!") => {
                self.next();
                ExprKind::Unary("!", Box::new(self.expr_bp(P_NOT)?))
            }
            Tok::Op("@") => {
                self.next();
                return self.expr_bp(P_UNARY);
            }
            Tok::Op(op @ ("++" | "--")) => {
                self.next();
                let target = self.postfix_expr()?;
                if !target.is_lvalue() {
                    self.pos = start;
                    return self.unexpected();
                }
                ExprKind::IncDec { target: Box::new(target), inc: op == "++", prefix: true }
            }
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("print") => {
                self.next();
                ExprKind::Call("print".into(), vec![Arg { value: self.expr_bp(P_ASSIGN)?, spread: false }])
            }
            _ => return self.postfix_expr(),
        };
        Ok(Expr::new(kind, line))
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let line = self.line();
        let tok = self.next();
        let kind = match tok {
            Tok::Int(n) => ExprKind::Int(n),
            Tok::Float(f) => ExprKind::Float(f),
            Tok::Str(s) => ExprKind::Str(Rc::new(s)),
            Tok::Template(parts) => {
                let mut exprs = Vec::with_capacity(parts.len());
                for part in parts {
                    exprs.push(match part {
                        StrPart::Lit(s) => Expr::new(ExprKind::Str(Rc::new(s)), line),
                        StrPart::Code(tokens) => parse_embedded(tokens)?,
                    });
                }
                ExprKind::Interp(exprs)
            }
            Tok::Variable(name) => ExprKind::Var(name),
            Tok::Op("[") => ExprKind::Array(self.array_items(&Tok::Op("]"))?),
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect(Tok::Op(")"))?;
                return Ok(e);
            }
            Tok::Ident(name) => {
                let lname = name.to_ascii_lowercase();
                if self.at(&Tok::Op("(")) && matches!(lname.as_str(), "array" | "isset" | "empty") {
                    self.next();
                    let kind = match lname.as_str() {
                        "array" => ExprKind::Array(self.array_items(&Tok::Op(")"))?),
                        "isset" => {
                            let args = self.expr_list(&Tok::Op(")"))?;
                            self.expect(Tok::Op(")"))?;
                            if args.is_empty() || !args.iter().all(Expr::is_variable) {
                                return Err(ParseError {
                                    msg: "Cannot use isset() on the result of an expression".into(),
                                    line: self.line(),
                                });
                            }
                            ExprKind::Isset(args)
                        }
                        _ => {
                            let e = self.expr()?;
                            self.expect(Tok::Op(")"))?;
                            ExprKind::Empty(Box::new(e))
                        }
                    };
                    return Ok(Expr::new(kind, line));
                }
                if self.eat(&Tok::Op("(")) {
                    let args = self.call_args()?;
                    return Ok(Expr::new(ExprKind::Call(name, args), line));
                }
                match name.to_ascii_lowercase().as_str() {
                    "true" => ExprKind::Bool(true),
                    "false" => ExprKind::Bool(false),
                    "null" => ExprKind::Null,
                    _ => ExprKind::Const(name),
                }
            }
            _ => {
                self.pos = start;
                return self.unexpected();
            }
        };
        Ok(Expr::new(kind, line))
    }
}
