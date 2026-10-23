//! Builds the AST from tokens. Expressions use a Pratt parser with PHP 8 precedence.

use std::rc::Rc;

use crate::ast::{
    Arg, ArrayItem, Catch, ClassDecl, ClassRef, Expr, ExprKind, FuncDecl, FuncKind, Member, Name, Param, Stmt, TypeHint,
};
use crate::object::Visibility;
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
const P_INSTANCEOF: u8 = 19;
const P_UNARY: u8 = 20;
const P_POW: u8 = 21;

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
        "array" => "(array)",
        "object" => "(object)",
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
                "try" => self.try_statement(),
                "class" | "interface" | "abstract" | "final" | "readonly"
                    if matches!(self.peek_at(1), Tok::Ident(_)) =>
                {
                    self.class_decl()
                }
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
        let params = self.params()?;
        let ret = if self.eat(&Tok::Op(":")) { Some(self.type_hint()?) } else { None };
        let body = self.brace_body()?;
        Ok(Stmt::Function(Rc::new(FuncDecl { kind: FuncKind::Named, name, uses: Vec::new(), params, ret, body, line })))
    }

    /// `function (...) use (...) { ... }`, after the `function` keyword
    fn closure(&mut self, line: u32) -> PResult<ExprKind> {
        self.eat(&Tok::Op("&"));
        let params = self.params()?;
        let mut uses = Vec::new();
        if self.eat_kw("use") {
            self.expect(Tok::Op("("))?;
            while !self.eat(&Tok::Op(")")) {
                let by_ref = self.eat(&Tok::Op("&"));
                let Tok::Variable(name) = self.next() else {
                    self.pos -= 1;
                    return self.unexpected();
                };
                if params.iter().any(|p| p.name == name) {
                    return Err(ParseError {
                        msg: format!("Cannot use lexical variable ${name} as a parameter name"),
                        line: self.line(),
                    });
                }
                uses.push((name, by_ref));
                if !self.eat(&Tok::Op(",")) {
                    self.expect(Tok::Op(")"))?;
                    break;
                }
            }
        }
        let ret = if self.eat(&Tok::Op(":")) { Some(self.type_hint()?) } else { None };
        let body = self.brace_body()?;
        let name = "{closure}".to_string();
        Ok(ExprKind::Closure(Rc::new(FuncDecl { kind: FuncKind::Closure, name, uses, params, ret, body, line })))
    }

    /// `fn (...) => expr`, after the `fn` keyword
    fn arrow_fn(&mut self, line: u32) -> PResult<ExprKind> {
        self.eat(&Tok::Op("&"));
        let params = self.params()?;
        let ret = if self.eat(&Tok::Op(":")) { Some(self.type_hint()?) } else { None };
        self.expect(Tok::Op("=>"))?;
        let body = self.expr_bp(P_ASSIGN)?;
        // Every variable the body mentions is captured by value, except the parameters
        let mut used = Vec::new();
        free_vars(&body, &mut used);
        let uses = used
            .into_iter()
            .filter(|v| !params.iter().any(|p| &p.name == v) && v != "this")
            .map(|v| (v, false))
            .collect();
        let name = "{closure}".to_string();
        let body = vec![Stmt::Return(Some(body))];
        Ok(ExprKind::Closure(Rc::new(FuncDecl { kind: FuncKind::Arrow, name, uses, params, ret, body, line })))
    }

    fn brace_body(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(Tok::Op("{"))?;
        let mut body = Vec::new();
        while !self.eat(&Tok::Op("}")) {
            if self.at(&Tok::Eof) {
                return self.unexpected();
            }
            body.push(self.statement()?);
        }
        Ok(body)
    }

    /// A parameter list including its parentheses
    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect(Tok::Op("("))?;
        let mut params: Vec<Param> = Vec::new();
        while !self.eat(&Tok::Op(")")) {
            // Constructor promotion modifiers
            let mut vis = None;
            let mut readonly = false;
            while let Tok::Ident(kw) = self.peek() {
                match kw.to_ascii_lowercase().as_str() {
                    "public" => vis = Some(Visibility::Public),
                    "protected" => vis = Some(Visibility::Protected),
                    "private" => vis = Some(Visibility::Private),
                    "readonly" => readonly = true,
                    _ => break,
                }
                self.next();
            }
            let promote = (vis.is_some() || readonly).then(|| (vis.unwrap_or(Visibility::Public), readonly));
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
            params.push(Param { promote, name: pname, ty, default, by_ref, variadic });
            if variadic && !self.at(&Tok::Op(")")) && !(self.at(&Tok::Op(",")) && self.peek_at(1) == &Tok::Op(")")) {
                return Err(ParseError { msg: "Only the last parameter can be variadic".into(), line: self.line() });
            }
            if !self.eat(&Tok::Op(",")) {
                self.expect(Tok::Op(")"))?;
                break;
            }
        }
        Ok(params)
    }

    fn class_decl(&mut self) -> PResult<Stmt> {
        let line = self.line();
        let (mut is_abstract, mut is_final, mut is_readonly) = (false, false, false);
        loop {
            if self.eat_kw("abstract") {
                is_abstract = true;
            } else if self.eat_kw("final") {
                is_final = true;
            } else if self.eat_kw("readonly") {
                is_readonly = true;
            } else {
                break;
            }
        }
        let is_interface = self.eat_kw("interface");
        if !is_interface && !self.eat_kw("class") {
            return self.unexpected();
        }
        let Tok::Ident(name) = self.next() else {
            self.pos -= 1;
            return self.unexpected();
        };
        let mut parent = None;
        let mut interfaces = Vec::new();
        if self.eat_kw("extends") {
            if is_interface {
                interfaces = self.name_list()?;
            } else {
                parent = Some(self.class_name()?);
            }
        }
        if !is_interface && self.eat_kw("implements") {
            interfaces = self.name_list()?;
        }
        self.expect(Tok::Op("{"))?;
        let mut members = Vec::new();
        while !self.eat(&Tok::Op("}")) {
            if self.at(&Tok::Eof) {
                return self.unexpected();
            }
            self.class_member(&mut members, is_interface)?;
        }
        let decl = ClassDecl { name, is_interface, is_abstract, is_final, is_readonly, parent, interfaces, members, line };
        Ok(Stmt::Class(Rc::new(decl)))
    }

    fn class_name(&mut self) -> PResult<String> {
        match self.next() {
            Tok::Ident(n) => Ok(n.trim_start_matches('\\').to_string()),
            _ => {
                self.pos -= 1;
                self.unexpected()
            }
        }
    }

    fn name_list(&mut self) -> PResult<Vec<String>> {
        let mut names = vec![self.class_name()?];
        while self.eat(&Tok::Op(",")) {
            names.push(self.class_name()?);
        }
        Ok(names)
    }

    fn class_member(&mut self, members: &mut Vec<Member>, in_interface: bool) -> PResult<()> {
        let line = self.line();
        let (mut vis, mut is_static, mut is_abstract, mut is_final, mut readonly) = (None, false, false, false, false);
        while let Tok::Ident(kw) = self.peek() {
            match kw.to_ascii_lowercase().as_str() {
                "public" | "var" => vis = Some(Visibility::Public),
                "protected" => vis = Some(Visibility::Protected),
                "private" => vis = Some(Visibility::Private),
                "static" => is_static = true,
                "abstract" => is_abstract = true,
                "final" => is_final = true,
                "readonly" => readonly = true,
                _ => break,
            }
            self.next();
        }
        let vis = vis.unwrap_or(Visibility::Public);
        if self.eat_kw("use") {
            return Err(ParseError { msg: "Traits are not supported yet".into(), line });
        }
        if self.eat_kw("const") {
            // An optional type before the name (PHP 8.3)
            if matches!((self.peek(), self.peek_at(1)), (Tok::Ident(_), Tok::Ident(_))) {
                self.type_hint()?;
            }
            loop {
                let name = self.class_name()?;
                self.expect(Tok::Op("="))?;
                members.push(Member::Const { name, value: self.expr()?, vis });
                if !self.eat(&Tok::Op(",")) {
                    break;
                }
            }
            return self.end_statement();
        }
        if self.eat_kw("function") {
            self.eat(&Tok::Op("&"));
            let name = self.class_name()?;
            let params = self.params()?;
            let ret = if self.eat(&Tok::Op(":")) { Some(self.type_hint()?) } else { None };
            let is_abstract = is_abstract || in_interface;
            let body = if self.eat(&Tok::Semicolon) {
                if !is_abstract {
                    return Err(ParseError { msg: format!("Non-abstract method {name}() must contain body"), line });
                }
                Vec::new()
            } else {
                if is_abstract {
                    return Err(ParseError { msg: format!("Abstract function {name}() cannot contain body"), line });
                }
                self.brace_body()?
            };
            if params.iter().any(|p| p.promote.is_some()) && !name.eq_ignore_ascii_case("__construct") {
                return Err(ParseError { msg: "Cannot declare promoted property outside a constructor".into(), line });
            }
            let func = Rc::new(FuncDecl { kind: FuncKind::Method, name, uses: Vec::new(), params, ret, body, line });
            members.push(Member::Method { func, vis, is_static, is_abstract, is_final });
            return Ok(());
        }
        if in_interface {
            return Err(ParseError { msg: "Interfaces may not include properties".into(), line });
        }
        let ty = if matches!(self.peek(), Tok::Variable(_)) { None } else { Some(self.type_hint()?) };
        loop {
            let Tok::Variable(name) = self.next() else {
                self.pos -= 1;
                return self.unexpected();
            };
            let default = if self.eat(&Tok::Op("=")) { Some(self.expr()?) } else { None };
            members.push(Member::Prop { name, ty: ty.clone(), default, vis, is_static, readonly });
            if !self.eat(&Tok::Op(",")) {
                break;
            }
        }
        self.end_statement()
    }

    /// After `X::`: a static property, static call, constant or `::class`
    fn static_member(&mut self, class: ClassRef) -> PResult<ExprKind> {
        Ok(match self.next() {
            Tok::Variable(name) => ExprKind::StaticProp { class, name },
            Tok::Ident(name) if self.eat(&Tok::Op("(")) => ExprKind::StaticCall { class, name, args: self.call_args()? },
            Tok::Ident(name) => ExprKind::ClassConst { class, name },
            _ => {
                self.pos -= 1;
                return self.unexpected();
            }
        })
    }

    /// The class after `new` or `instanceof`
    fn class_ref(&mut self) -> PResult<ClassRef> {
        let line = self.line();
        match self.next() {
            Tok::Ident(name) => Ok(ClassRef::from_name(&name)),
            Tok::Variable(v) => {
                let mut e = Expr::new(ExprKind::Var(v), line);
                // `new $this->cls` uses a property as the class name
                while self.at(&Tok::Op("->")) {
                    self.next();
                    let name = self.class_name()?;
                    e = Expr::new(ExprKind::Prop { object: Box::new(e), name: Name::Lit(name), nullsafe: false }, line);
                }
                Ok(ClassRef::Dynamic(Box::new(e)))
            }
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect(Tok::Op(")"))?;
                Ok(ClassRef::Dynamic(Box::new(e)))
            }
            _ => {
                self.pos -= 1;
                self.unexpected()
            }
        }
    }

    fn try_statement(&mut self) -> PResult<Stmt> {
        self.next();
        let body = self.brace_body()?;
        let mut catches = Vec::new();
        while self.eat_kw("catch") {
            self.expect(Tok::Op("("))?;
            let mut types = Vec::new();
            loop {
                let Tok::Ident(name) = self.next() else {
                    self.pos -= 1;
                    return self.unexpected();
                };
                types.push(name.trim_start_matches('\\').to_string());
                if !self.eat(&Tok::Op("|")) {
                    break;
                }
            }
            let var = match self.peek().clone() {
                Tok::Variable(v) => {
                    self.next();
                    Some(v)
                }
                _ => None,
            };
            self.expect(Tok::Op(")"))?;
            catches.push(Catch { types, var, body: self.brace_body()? });
        }
        let finally = if self.eat_kw("finally") { Some(self.brace_body()?) } else { None };
        if catches.is_empty() && finally.is_none() {
            return Err(ParseError { msg: "Cannot use try without catch or finally".into(), line: self.line() });
        }
        Ok(Stmt::Try { body, catches, finally })
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

            if matches!(self.peek(), Tok::Ident(kw) if kw.eq_ignore_ascii_case("instanceof")) {
                if P_INSTANCEOF < min_prec {
                    break;
                }
                self.next();
                let class = self.class_ref()?;
                let line = lhs.line;
                lhs = Expr::new(ExprKind::InstanceOf(Box::new(lhs), class), line);
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
        loop {
            let line = e.line;
            let kind = match self.peek() {
                Tok::Op("[") => {
                    self.next();
                    let index = if self.at(&Tok::Op("]")) { None } else { Some(Box::new(self.expr()?)) };
                    self.expect(Tok::Op("]"))?;
                    ExprKind::Index(Box::new(e), index)
                }
                Tok::Op(op @ ("->" | "?->")) => {
                    let nullsafe = *op == "?->";
                    self.next();
                    let name = match self.next() {
                        Tok::Ident(n) => Name::Lit(n),
                        Tok::Variable(v) => Name::Dyn(Box::new(Expr::new(ExprKind::Var(v), self.line()))),
                        Tok::Op("{") => {
                            let e = self.expr()?;
                            self.expect(Tok::Op("}"))?;
                            Name::Dyn(Box::new(e))
                        }
                        _ => {
                            self.pos -= 1;
                            return self.unexpected();
                        }
                    };
                    if self.eat(&Tok::Op("(")) {
                        let args = self.call_args()?;
                        ExprKind::MethodCall { object: Box::new(e), name, args, nullsafe }
                    } else {
                        ExprKind::Prop { object: Box::new(e), name, nullsafe }
                    }
                }
                Tok::Op("(") => {
                    self.next();
                    ExprKind::CallValue(Box::new(e), self.call_args()?)
                }
                Tok::Op("::") => {
                    self.next();
                    self.static_member(ClassRef::Dynamic(Box::new(e)))?
                }
                _ => return Ok(e),
            };
            e = Expr::new(kind, line);
        }
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
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("clone") => {
                self.next();
                ExprKind::Clone(Box::new(self.expr_bp(P_UNARY)?))
            }
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("throw") => {
                self.next();
                ExprKind::Throw(Box::new(self.expr_bp(0)?))
            }
            // `static function` / `static fn`: there is no $this to leave unbound yet
            Tok::Ident(kw)
                if kw.eq_ignore_ascii_case("static")
                    && matches!(self.peek_at(1), Tok::Ident(f) if f.eq_ignore_ascii_case("function") || f.eq_ignore_ascii_case("fn")) =>
            {
                self.next();
                return self.prefix();
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
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("function") => self.closure(line)?,
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("fn") => self.arrow_fn(line)?,
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("new") => {
                if self.at_kw("class") {
                    return Err(ParseError { msg: "Anonymous classes are not supported yet".into(), line });
                }
                let class = self.class_ref()?;
                let args = if self.eat(&Tok::Op("(")) { self.call_args()? } else { Vec::new() };
                ExprKind::New(class, args)
            }
            Tok::Ident(name) if self.at(&Tok::Op("::")) => {
                self.next();
                self.static_member(ClassRef::from_name(&name))?
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

/// Variable names an arrow function body refers to, in first-use order
fn free_vars(e: &Expr, out: &mut Vec<String>) {
    let add = |name: &str, out: &mut Vec<String>| {
        if !out.iter().any(|v| v == name) {
            out.push(name.to_string());
        }
    };
    let args = |args: &[Arg], out: &mut Vec<String>| args.iter().for_each(|a| free_vars(&a.value, out));
    match &e.kind {
        ExprKind::Null | ExprKind::Bool(_) | ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Const(_) => {}
        ExprKind::Var(name) => add(name, out),
        ExprKind::Interp(parts) => parts.iter().for_each(|p| free_vars(p, out)),
        ExprKind::Array(items) => {
            for item in items {
                if let Some(k) = &item.key {
                    free_vars(k, out);
                }
                free_vars(&item.value, out);
            }
        }
        ExprKind::Index(base, index) => {
            free_vars(base, out);
            if let Some(i) = index {
                free_vars(i, out);
            }
        }
        ExprKind::Assign(a, b) | ExprKind::AssignRef(a, b) | ExprKind::CompoundAssign(_, a, b) | ExprKind::Binary(_, a, b) => {
            free_vars(a, out);
            free_vars(b, out);
        }
        ExprKind::IncDec { target, .. } => free_vars(target, out),
        ExprKind::Unary(_, x) | ExprKind::Empty(x) | ExprKind::Throw(x) | ExprKind::Clone(x) => free_vars(x, out),
        ExprKind::InstanceOf(x, class) => {
            free_vars(x, out);
            class_vars(class, out);
        }
        ExprKind::StaticProp { class, .. } | ExprKind::ClassConst { class, .. } => class_vars(class, out),
        ExprKind::StaticCall { class, args: a, .. } => {
            class_vars(class, out);
            args(a, out);
        }
        ExprKind::Prop { object, name, .. } => {
            free_vars(object, out);
            name_vars(name, out);
        }
        ExprKind::Ternary(c, t, f) => {
            free_vars(c, out);
            if let Some(t) = t {
                free_vars(t, out);
            }
            free_vars(f, out);
        }
        ExprKind::Call(_, a) => args(a, out),
        ExprKind::New(class, a) => {
            class_vars(class, out);
            args(a, out);
        }
        ExprKind::CallValue(f, a) => {
            free_vars(f, out);
            args(a, out);
        }
        ExprKind::MethodCall { object, name, args: a, .. } => {
            free_vars(object, out);
            name_vars(name, out);
            args(a, out);
        }
        ExprKind::Isset(list) => list.iter().for_each(|x| free_vars(x, out)),
        // A nested closure only needs what it captures itself
        ExprKind::Closure(f) => f.uses.iter().for_each(|(name, _)| add(name, out)),
    }
}

fn name_vars(name: &Name, out: &mut Vec<String>) {
    if let Name::Dyn(e) = name {
        free_vars(e, out);
    }
}

fn class_vars(class: &ClassRef, out: &mut Vec<String>) {
    if let ClassRef::Dynamic(e) = class {
        free_vars(e, out);
    }
}
