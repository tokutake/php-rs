//! Builds the AST from tokens. Expressions use a Pratt parser with PHP 8 precedence.

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{
    Arg, ArrayItem, Catch, ClassDecl, ClassRef, Expr, ExprKind, FuncDecl, FuncKind, ListItem, Magic, MatchArm, Member, Name,
    Param, Stmt, TraitRule, TypeHint,
};
use crate::object::Visibility;
use crate::lexer::{StrPart, Tok, Token};

#[derive(Debug)]
pub struct ParseError {
    pub msg: String,
    pub line: u32,
}

impl ParseError {
    /// How php-cli labels the error: grammar errors are parse errors, the
    /// checks php-src does while compiling are fatal errors
    pub fn label(&self) -> &'static str {
        if self.msg.starts_with("syntax error") || self.msg == "memory exhausted" {
            "Parse error"
        } else {
            "Fatal error"
        }
    }
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
/// An expression interpolated into a string, in the context of the code around it
fn parse_embedded(outer: &Parser, mut tokens: Vec<Token>) -> PResult<Expr> {
    let line = tokens.last().map_or(0, |t| t.line);
    tokens.push(Token { tok: Tok::Eof, line });
    let mut p = Parser::new(tokens);
    p.class = outer.class.clone();
    p.in_trait = outer.in_trait;
    p.func = outer.func.clone();
    p.names = outer.names.clone();
    p.depth = outer.depth;
    let e = p.expr()?;
    if !p.at(&Tok::Eof) {
        return p.unexpected();
    }
    Ok(e)
}

/// Parses a script. Names are resolved against the namespace and imports here,
/// as php-src does while compiling; the statements of braced namespaces are
/// flattened into the top level
pub fn parse(tokens: Vec<Token>) -> PResult<Vec<Stmt>> {
    let mut p = Parser::new(tokens);
    let mut stmts = Vec::new();
    while !p.at(&Tok::Eof) {
        if p.at_namespace_decl() {
            p.namespace_decl(&mut stmts)?;
            continue;
        }
        if p.braced_ns == Some(true) && !p.at_kw("declare") {
            return Err(ParseError { msg: "No code may exist outside of namespace {}".into(), line: p.line() });
        }
        let s = p.statement()?;
        p.seen_code |= !matches!(s, Stmt::StrictTypes(_) | Stmt::Block(_));
        stmts.push(s);
        p.seen_stmt = true;
    }
    Ok(stmts)
}

/// The current namespace and its `use` imports. Class and function aliases are
/// keyed in lowercase, constant aliases as written
#[derive(Default, Clone)]
struct Names {
    ns: String,
    classes: HashMap<String, String>,
    functions: HashMap<String, String>,
    consts: HashMap<String, String>,
}

#[derive(Clone, Copy, PartialEq)]
enum UseKind {
    Class,
    Function,
    Const,
}

/// How a name was written: `\A\B`, `namespace\A\B`, or relative to the imports
enum NameForm<'a> {
    Full(&'a str),
    Relative(&'a str),
    Plain(&'a str),
}

fn name_form(raw: &str) -> NameForm<'_> {
    if let Some(rest) = raw.strip_prefix('\\') {
        return NameForm::Full(rest);
    }
    if raw.len() > 10 && raw.as_bytes()[9] == b'\\' && raw[..9].eq_ignore_ascii_case("namespace") {
        return NameForm::Relative(&raw[10..]);
    }
    NameForm::Plain(raw)
}

/// Reserved words, which syntax errors call tokens rather than identifiers
const KEYWORDS: &[&str] = &[
    "abstract", "and", "array", "as", "break", "callable", "case", "catch", "class", "clone", "const", "continue",
    "declare", "default", "do", "echo", "else", "elseif", "empty", "enddeclare", "endfor", "endforeach", "endif",
    "endswitch", "endwhile", "eval", "exit", "die", "extends", "final", "finally", "fn", "for", "foreach", "function",
    "global", "goto", "if", "implements", "include", "include_once", "instanceof", "insteadof", "interface", "isset",
    "list", "match", "namespace", "new", "or", "print", "private", "protected", "public", "readonly", "require",
    "require_once", "return", "static", "switch", "throw", "trait", "try", "unset", "use", "var", "while", "xor",
    "yield",
];

/// Type names that are not classes
const BUILTIN_TYPES: &[&str] = &[
    "int", "float", "string", "bool", "false", "true", "null", "void", "never", "iterable", "mixed", "object", "array",
    "callable", "static", "self", "parent",
];

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: u32,
    /// The enclosing class and function, for `__CLASS__`, `__FUNCTION__` and `__METHOD__`
    class: Option<String>,
    /// Whether `class` is a trait, whose `__CLASS__` is only known at runtime
    in_trait: bool,
    func: Option<FuncCtx>,
    /// Inside the items of `[...]` / `list(...)`, where a skipped element may
    /// belong to a list pattern that is only recognized at the outer `=`
    in_items: u32,
    /// Whether a top-level statement has been parsed, for `declare(strict_types=1)`
    seen_stmt: bool,
    /// Whether a top-level statement other than `declare` has been parsed, which
    /// a first namespace declaration must not follow
    seen_code: bool,
    /// Whether the namespace declarations so far were braced, None before the first
    braced_ns: Option<bool>,
    names: Names,
    /// The line of the first `yield` in the function being parsed
    yield_line: Option<u32>,
}

/// What kind of declaration a class member belongs to
#[derive(Clone, Copy)]
enum MemberCtx<'a> {
    Class,
    Interface,
    Enum { name: &'a str, backed: bool },
}

#[derive(Clone)]
enum FuncCtx {
    Named(String),
    Method(String),
    Closure,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            pos: 0,
            depth: 0,
            class: None,
            in_trait: false,
            func: None,
            in_items: 0,
            seen_stmt: false,
            seen_code: false,
            braced_ns: None,
            names: Names::default(),
            yield_line: None,
        }
    }

    /// `namespace A;`, `namespace A { ... }` or `namespace { ... }`, as opposed to
    /// a `namespace\name` reference (lexed as a single name)
    fn at_namespace_decl(&self) -> bool {
        self.at_kw("namespace") && matches!(self.peek_at(1), Tok::Ident(_) | Tok::Op("{"))
    }

    fn namespace_decl(&mut self, stmts: &mut Vec<Stmt>) -> PResult<()> {
        let line = self.line();
        let fatal = |msg: &str| Err(ParseError { msg: msg.into(), line });
        self.next();
        let name = match self.peek().clone() {
            Tok::Ident(n) => {
                self.next();
                Some(n)
            }
            _ => None,
        };
        let braced = self.at(&Tok::Op("{"));
        if name.is_none() && !braced {
            return self.unexpected();
        }
        match self.braced_ns {
            Some(b) if b != braced => {
                return fatal("Cannot mix bracketed namespace declarations with unbracketed namespace declarations")
            }
            None if self.seen_code => {
                return fatal(
                    "Namespace declaration statement has to be the very first statement or after any declare call in the script",
                )
            }
            _ => {}
        }
        if let Some(n) = &name {
            if n.eq_ignore_ascii_case("namespace") {
                return fatal(&format!("Cannot use '{n}' as namespace name"));
            }
            if n.starts_with('\\') {
                return self.unexpected_at(self.pos - 1);
            }
        }
        self.braced_ns = Some(braced);
        self.seen_code = true;
        self.names = Names { ns: name.unwrap_or_default(), ..Names::default() };
        if braced {
            self.next();
            while !self.eat(&Tok::Op("}")) {
                if self.at(&Tok::Eof) {
                    return self.unexpected();
                }
                if self.at_namespace_decl() {
                    return Err(ParseError { msg: "Namespace declarations cannot be nested".into(), line: self.line() });
                }
                stmts.push(self.statement()?);
            }
            self.names = Names::default();
        } else {
            self.end_statement()?;
        }
        Ok(())
    }

    /// `use A\B as C, ...;`, `use function ...`, `use const ...` and group uses
    /// `use A\{B, function c, const D}`
    fn use_decl(&mut self) -> PResult<()> {
        self.next();
        let kind = if self.eat_kw("function") {
            UseKind::Function
        } else if self.eat_kw("const") {
            UseKind::Const
        } else {
            UseKind::Class
        };
        loop {
            let line = self.line();
            let name = self.class_name()?;
            if self.eat(&Tok::Op("\\")) {
                self.expect(Tok::Op("{"))?;
                while !self.eat(&Tok::Op("}")) {
                    let item_kind = if kind != UseKind::Class {
                        kind
                    } else if self.eat_kw("function") {
                        UseKind::Function
                    } else if self.eat_kw("const") {
                        UseKind::Const
                    } else {
                        UseKind::Class
                    };
                    let line = self.line();
                    let item = self.class_name()?;
                    let alias = if self.eat_kw("as") { Some(self.class_name()?) } else { None };
                    self.import(item_kind, format!("{name}\\{item}"), alias, line)?;
                    if !self.eat(&Tok::Op(",")) {
                        self.expect(Tok::Op("}"))?;
                        break;
                    }
                }
            } else {
                let alias = if self.eat_kw("as") { Some(self.class_name()?) } else { None };
                self.import(kind, name, alias, line)?;
            }
            if !self.eat(&Tok::Op(",")) {
                break;
            }
        }
        self.end_statement()
    }

    fn import(&mut self, kind: UseKind, name: String, alias: Option<String>, line: u32) -> PResult<()> {
        let alias = alias.unwrap_or_else(|| name.rsplit('\\').next().unwrap_or(&name).to_string());
        let what = match kind {
            UseKind::Class => "",
            UseKind::Function => " function",
            UseKind::Const => " const",
        };
        if kind == UseKind::Class && matches!(alias.to_ascii_lowercase().as_str(), "self" | "parent" | "static") {
            let msg = format!("Cannot use {name} as {alias} because '{alias}' is a special class name");
            return Err(ParseError { msg, line });
        }
        let (map, key) = match kind {
            UseKind::Class => (&mut self.names.classes, alias.to_ascii_lowercase()),
            UseKind::Function => (&mut self.names.functions, alias.to_ascii_lowercase()),
            UseKind::Const => (&mut self.names.consts, alias.clone()),
        };
        if map.contains_key(&key) {
            let msg = format!("Cannot use{what} {name} as {alias} because the name is already in use");
            return Err(ParseError { msg, line });
        }
        map.insert(key, name);
        Ok(())
    }

    /// A declared name in the current namespace
    fn prefix_ns(&self, name: &str) -> String {
        if self.names.ns.is_empty() {
            name.to_string()
        } else {
            format!("{}\\{name}", self.names.ns)
        }
    }

    /// A qualified name whose first part may be an imported class or namespace alias
    fn resolve_qualified(&self, name: &str) -> String {
        if let Some((first, rest)) = name.split_once('\\')
            && let Some(full) = self.names.classes.get(&first.to_ascii_lowercase())
        {
            return format!("{full}\\{rest}");
        }
        self.prefix_ns(name)
    }

    /// The fully qualified name of a class reference (`self`, `parent` and
    /// `static` stay as they are)
    fn resolve_class(&self, raw: &str) -> String {
        match name_form(raw) {
            NameForm::Full(n) => n.to_string(),
            NameForm::Relative(n) => self.prefix_ns(n),
            NameForm::Plain(n) if matches!(n.to_ascii_lowercase().as_str(), "self" | "parent" | "static") => n.to_string(),
            NameForm::Plain(n) if n.contains('\\') => self.resolve_qualified(n),
            NameForm::Plain(n) => match self.names.classes.get(&n.to_ascii_lowercase()) {
                Some(full) => full.clone(),
                None => self.prefix_ns(n),
            },
        }
    }

    fn class_ref_named(&self, raw: &str) -> ClassRef {
        match ClassRef::from_name(raw) {
            ClassRef::Named(_) => ClassRef::Named(self.resolve_class(raw)),
            special => special,
        }
    }

    /// A function or constant name: the resolved name, and for an unqualified
    /// name in a namespace the global name to fall back to at runtime
    fn resolve_symbol(&self, raw: &str, kind: UseKind) -> (String, Option<String>) {
        match name_form(raw) {
            NameForm::Full(n) => (n.to_string(), None),
            NameForm::Relative(n) => (self.prefix_ns(n), None),
            NameForm::Plain(n) if n.contains('\\') => (self.resolve_qualified(n), None),
            NameForm::Plain(n) => {
                let imported = match kind {
                    UseKind::Function => self.names.functions.get(&n.to_ascii_lowercase()),
                    _ => self.names.consts.get(n),
                };
                match imported {
                    Some(full) => (full.clone(), None),
                    None if self.names.ns.is_empty() => (n.to_string(), None),
                    None => (self.prefix_ns(n), Some(n.to_string())),
                }
            }
        }
    }

    fn unexpected_at<T>(&mut self, pos: usize) -> PResult<T> {
        self.pos = pos;
        self.unexpected()
    }

    /// Runs `f` with `func` as the enclosing function
    fn in_func<T>(&mut self, func: FuncCtx, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        let outer = self.func.replace(func);
        let outer_yield = self.yield_line.take();
        let out = f(self);
        self.func = outer;
        self.yield_line = outer_yield;
        out
    }

    /// A function declaration, within its in_func(): a function that yields is a
    /// generator, whose return type must allow Generator and whose body is lowered
    #[allow(clippy::too_many_arguments)]
    fn make_func(
        &self,
        kind: FuncKind,
        name: String,
        uses: Vec<(String, bool)>,
        params: Vec<Param>,
        ret: Option<TypeHint>,
        body: Vec<Stmt>,
        line: u32,
    ) -> PResult<Rc<FuncDecl>> {
        let Some(yield_line) = self.yield_line else {
            return Ok(Rc::new(FuncDecl { kind, name, uses, params, ret, body, line, is_generator: false }));
        };
        if let Some(ty) = &ret {
            let ok = ty.types.iter().any(|t| {
                matches!(t.as_str(), "object" | "mixed" | "iterable" | "static" | "traversable" | "iterator" | "generator")
            });
            if !ok {
                let msg = format!("Generator return type must be a supertype of Generator, {} given", ty.display);
                return Err(ParseError { msg, line: yield_line });
            }
        }
        let body = crate::genlower::lower(body)?;
        Ok(Rc::new(FuncDecl { kind, name, uses, params, ret, body, line, is_generator: true }))
    }

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
            Tok::Ident(s) if KEYWORDS.contains(&s.to_ascii_lowercase().as_str()) => format!("token \"{s}\""),
            Tok::Ident(s) if s.contains('\\') => format!("{} \"{s}\"", match name_form(s) {
                NameForm::Full(_) => "fully qualified name",
                NameForm::Relative(_) => "namespace-relative name",
                NameForm::Plain(_) => "namespaced name",
            }),
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
                    let body = self.body_or_alt("endwhile")?;
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
                "switch" => self.switch_statement(),
                "declare" => self.declare_statement(),
                "const" if matches!(self.peek_at(1), Tok::Ident(_)) => {
                    self.next();
                    let mut consts = Vec::new();
                    loop {
                        let name = self.class_name()?;
                        self.expect(Tok::Op("="))?;
                        consts.push((self.prefix_ns(&name), self.expr()?));
                        if !self.eat(&Tok::Op(",")) {
                            break;
                        }
                    }
                    self.end_statement()?;
                    Ok(Stmt::Const(consts))
                }
                "try" => self.try_statement(),
                // Imports only exist at the top level (and directly in namespaces)
                "use" if self.depth == 1 => {
                    self.use_decl()?;
                    Ok(Stmt::Block(Vec::new()))
                }
                "use" => self.unexpected(),
                "namespace" if self.at_namespace_decl() => {
                    Err(ParseError { msg: "Namespace declarations cannot be nested".into(), line: self.line() })
                }
                "class" | "interface" | "trait" | "abstract" | "final" | "readonly"
                    if matches!(self.peek_at(1), Tok::Ident(_)) =>
                {
                    self.class_decl()
                }
                // `enum` is only a keyword in front of a declaration
                "enum"
                    if matches!(self.peek_at(1), Tok::Ident(_))
                        && (matches!(self.peek_at(2), Tok::Op("{" | ":"))
                            || matches!(self.peek_at(2), Tok::Ident(k) if k.eq_ignore_ascii_case("implements"))) =>
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

    /// A loop body, either a statement or the alternative syntax `: ... endwhile;`
    fn body_or_alt(&mut self, end: &str) -> PResult<Vec<Stmt>> {
        if !self.eat(&Tok::Op(":")) {
            return self.body();
        }
        let body = self.alt_block(&[end])?;
        self.next();
        self.end_statement()?;
        Ok(body)
    }

    /// Statements up to (not including) one of the given keywords
    fn alt_block(&mut self, ends: &[&str]) -> PResult<Vec<Stmt>> {
        let mut body = Vec::new();
        while !ends.iter().any(|e| self.at_kw(e)) {
            if self.at(&Tok::Eof) {
                return self.unexpected();
            }
            body.push(self.statement()?);
        }
        Ok(body)
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
        let cond = self.paren_expr()?;
        if self.eat(&Tok::Op(":")) {
            return self.alt_if(cond);
        }
        let mut branches = vec![(cond, self.body()?)];
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

    /// `if (...): ... elseif (...): ... else: ... endif;` after the first condition and colon
    fn alt_if(&mut self, cond: Expr) -> PResult<Stmt> {
        const ENDS: &[&str] = &["elseif", "else", "endif"];
        let mut branches = vec![(cond, self.alt_block(ENDS)?)];
        let mut else_body = None;
        loop {
            if self.eat_kw("elseif") {
                let cond = self.paren_expr()?;
                self.expect(Tok::Op(":"))?;
                branches.push((cond, self.alt_block(ENDS)?));
            } else if self.eat_kw("else") {
                self.expect(Tok::Op(":"))?;
                else_body = Some(self.alt_block(&["endif"])?);
            } else {
                self.next();
                self.end_statement()?;
                return Ok(Stmt::If(branches, else_body));
            }
        }
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
        let body = self.body_or_alt("endfor")?;
        Ok(Stmt::For(init, cond, step, body))
    }

    /// `declare(strict_types=1);`. Other directives (ticks, encoding) have no effect here
    fn declare_statement(&mut self) -> PResult<Stmt> {
        let line = self.line();
        self.next();
        self.expect(Tok::Op("("))?;
        let mut strict = None;
        loop {
            let name = self.class_name()?;
            self.expect(Tok::Op("="))?;
            let value = self.expr()?;
            if name.eq_ignore_ascii_case("strict_types") {
                let ExprKind::Int(v @ (0 | 1)) = value.kind else {
                    return Err(ParseError { msg: "strict_types declaration must have 0 or 1 as its value".into(), line });
                };
                if self.seen_stmt || self.depth > 1 {
                    let msg = "strict_types declaration must be the very first statement in the script".into();
                    return Err(ParseError { msg, line });
                }
                strict = Some(v == 1);
            }
            if !self.eat(&Tok::Op(",")) {
                break;
            }
        }
        self.expect(Tok::Op(")"))?;
        if let Some(strict) = strict {
            if !self.at(&Tok::Semicolon) {
                let msg = "strict_types declaration must not use block mode".into();
                return Err(ParseError { msg, line });
            }
            self.next();
            return Ok(Stmt::StrictTypes(strict));
        }
        if self.eat(&Tok::Op(":")) {
            let body = self.alt_block(&["enddeclare"])?;
            self.next();
            self.end_statement()?;
            return Ok(Stmt::Block(body));
        }
        self.statement()
    }

    fn switch_statement(&mut self) -> PResult<Stmt> {
        self.next();
        let subject = self.paren_expr()?;
        let alt = self.eat(&Tok::Op(":"));
        if !alt {
            self.expect(Tok::Op("{"))?;
        }
        let close = |p: &mut Self| if alt { p.at_kw("endswitch") } else { p.at(&Tok::Op("}")) };
        let mut cases = Vec::new();
        let mut has_default = false;
        loop {
            if close(self) {
                self.next();
                if alt {
                    self.end_statement()?;
                }
                break;
            }
            let line = self.line();
            let cond = if self.eat_kw("case") {
                Some(self.expr()?)
            } else if self.eat_kw("default") {
                if has_default {
                    return Err(ParseError { msg: "Switch statements may only contain one default clause".into(), line });
                }
                has_default = true;
                None
            } else {
                return self.unexpected();
            };
            if !self.eat(&Tok::Op(":")) {
                self.expect(Tok::Semicolon)?;
            }
            let mut body = Vec::new();
            while !self.at_kw("case") && !self.at_kw("default") && !close(self) {
                if self.at(&Tok::Eof) {
                    return self.unexpected();
                }
                body.push(self.statement()?);
            }
            cases.push((cond, body));
        }
        Ok(Stmt::Switch(subject, cases))
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
        self.expect(Tok::Op(")"))?;
        let body = self.body_or_alt("endforeach")?;
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
        let name = self.prefix_ns(&name);
        let func = self.in_func(FuncCtx::Named(name.clone()), |p| {
            let params = p.params()?;
            let ret = if p.eat(&Tok::Op(":")) { Some(p.type_hint()?) } else { None };
            let body = p.brace_body()?;
            p.make_func(FuncKind::Named, name, Vec::new(), params, ret, body, line)
        })?;
        Ok(Stmt::Function(func))
    }

    /// `function (...) use (...) { ... }`, after the `function` keyword
    fn closure(&mut self, line: u32) -> PResult<ExprKind> {
        self.in_func(FuncCtx::Closure, |p| p.closure_inner(line))
    }

    fn closure_inner(&mut self, line: u32) -> PResult<ExprKind> {
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
        Ok(ExprKind::Closure(self.make_func(FuncKind::Closure, name, uses, params, ret, body, line)?))
    }

    /// `fn (...) => expr`, after the `fn` keyword
    fn arrow_fn(&mut self, line: u32) -> PResult<ExprKind> {
        self.in_func(FuncCtx::Closure, |p| p.arrow_fn_inner(line))
    }

    fn arrow_fn_inner(&mut self, line: u32) -> PResult<ExprKind> {
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
        Ok(ExprKind::Closure(self.make_func(FuncKind::Arrow, name, uses, params, ret, body, line)?))
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
        let is_trait = !is_interface && self.eat_kw("trait");
        let is_enum = !is_interface && !is_trait && self.eat_kw("enum");
        if !is_interface && !is_trait && !is_enum && !self.eat_kw("class") {
            return self.unexpected();
        }
        let Tok::Ident(name) = self.next() else {
            self.pos -= 1;
            return self.unexpected();
        };
        let name = self.prefix_ns(&name);
        let backing = if is_enum && self.eat(&Tok::Op(":")) { Some(self.type_hint()?) } else { None };
        if let Some(ty) = &backing
            && (ty.types.len() != 1 || !matches!(ty.types[0].as_str(), "int" | "string"))
        {
            return Err(ParseError { msg: format!("Enum backing type must be int or string, {} given", ty.display), line });
        }
        let mut parent = None;
        let mut interfaces = Vec::new();
        if !is_trait && !is_enum && self.eat_kw("extends") {
            if is_interface {
                interfaces = self.class_list()?;
            } else {
                parent = Some(self.resolved_class_name()?);
            }
        }
        if !is_interface && !is_trait && self.eat_kw("implements") {
            interfaces = self.class_list()?;
        }
        self.expect(Tok::Op("{"))?;
        let mut members = Vec::new();
        let outer = (self.class.replace(name.clone()), self.func.take(), std::mem::replace(&mut self.in_trait, is_trait));
        while !self.eat(&Tok::Op("}")) {
            if self.at(&Tok::Eof) {
                return self.unexpected();
            }
            let kind = match (is_interface, is_enum, &backing) {
                (true, ..) => MemberCtx::Interface,
                (_, true, b) => MemberCtx::Enum { name: &name, backed: b.is_some() },
                _ => MemberCtx::Class,
            };
            self.class_member(&mut members, kind)?;
        }
        (self.class, self.func, self.in_trait) = outer;
        let decl = ClassDecl {
            name,
            is_interface,
            is_trait,
            is_enum,
            backing,
            is_abstract,
            is_final,
            is_readonly,
            parent,
            interfaces,
            members,
            line,
        };
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

    fn resolved_class_name(&mut self) -> PResult<String> {
        match self.next() {
            Tok::Ident(n) => Ok(self.resolve_class(&n)),
            _ => {
                self.pos -= 1;
                self.unexpected()
            }
        }
    }

    /// Comma-separated class names, resolved
    fn class_list(&mut self) -> PResult<Vec<String>> {
        let mut names = vec![self.resolved_class_name()?];
        while self.eat(&Tok::Op(",")) {
            names.push(self.resolved_class_name()?);
        }
        Ok(names)
    }

    fn class_member(&mut self, members: &mut Vec<Member>, ctx: MemberCtx<'_>) -> PResult<()> {
        let in_interface = matches!(ctx, MemberCtx::Interface);
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
            let traits = self.class_list()?;
            let mut rules = Vec::new();
            if self.eat(&Tok::Op("{")) {
                while !self.eat(&Tok::Op("}")) {
                    rules.push(self.trait_rule()?);
                }
            } else {
                self.end_statement()?;
            }
            members.push(Member::Use { traits, rules });
            return Ok(());
        }
        if let MemberCtx::Enum { name: enum_name, backed } = ctx
            && self.eat_kw("case")
        {
            let name = self.class_name()?;
            let value = if self.eat(&Tok::Op("=")) { Some(self.expr()?) } else { None };
            match (backed, &value) {
                (true, None) => {
                    let msg = format!("Case {name} of backed enum {enum_name} must have a value");
                    return Err(ParseError { msg, line });
                }
                (false, Some(_)) => {
                    let msg = format!("Case {name} of non-backed enum {enum_name} must not have a value");
                    return Err(ParseError { msg, line });
                }
                _ => {}
            }
            members.push(Member::Case { name, value });
            return self.end_statement();
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
            let is_abstract = is_abstract || in_interface;
            let func = self.in_func(FuncCtx::Method(name.clone()), |p| {
                let params = p.params()?;
                let ret = if p.eat(&Tok::Op(":")) { Some(p.type_hint()?) } else { None };
                let body = if p.eat(&Tok::Semicolon) {
                    if !is_abstract {
                        return Err(ParseError { msg: format!("Non-abstract method {name}() must contain body"), line });
                    }
                    Vec::new()
                } else {
                    if is_abstract {
                        return Err(ParseError { msg: format!("Abstract function {name}() cannot contain body"), line });
                    }
                    p.brace_body()?
                };
                if params.iter().any(|p| p.promote.is_some()) && !name.eq_ignore_ascii_case("__construct") {
                    return Err(ParseError { msg: "Cannot declare promoted property outside a constructor".into(), line });
                }
                p.make_func(FuncKind::Method, name, Vec::new(), params, ret, body, line)
            })?;
            members.push(Member::Method { func, vis, is_static, is_abstract, is_final });
            return Ok(());
        }
        match ctx {
            MemberCtx::Interface => {
                return Err(ParseError { msg: "Interfaces may not include properties".into(), line });
            }
            MemberCtx::Enum { name, .. } => {
                return Err(ParseError { msg: format!("Enum {name} cannot include properties"), line });
            }
            MemberCtx::Class => {}
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

    /// One `A::m insteadof B;` or `[A::]m as [visibility] [alias];` adaptation
    fn trait_rule(&mut self) -> PResult<TraitRule> {
        let first = self.class_name()?;
        let (trait_name, method) = if self.eat(&Tok::Op("::")) {
            (Some(self.resolve_class(&first)), self.class_name()?)
        } else {
            (None, first)
        };
        let rule = if self.eat_kw("insteadof") {
            let Some(trait_name) = trait_name else { return self.unexpected() };
            TraitRule::Insteadof { trait_name, method, excluded: self.class_list()? }
        } else if self.eat_kw("as") {
            let vis = match self.peek() {
                Tok::Ident(k) => match k.to_ascii_lowercase().as_str() {
                    "public" => Some(Visibility::Public),
                    "protected" => Some(Visibility::Protected),
                    "private" => Some(Visibility::Private),
                    _ => None,
                },
                _ => None,
            };
            if vis.is_some() {
                self.next();
            }
            let alias = if matches!(self.peek(), Tok::Ident(_)) { Some(self.class_name()?) } else { None };
            if vis.is_none() && alias.is_none() {
                return self.unexpected();
            }
            TraitRule::Alias { trait_name, method, vis, alias }
        } else {
            return self.unexpected();
        };
        self.end_statement()?;
        Ok(rule)
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
            Tok::Ident(name) => Ok(self.class_ref_named(&name)),
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
                types.push(self.resolve_class(&name));
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
            let lname = name.to_ascii_lowercase();
            let name = if BUILTIN_TYPES.contains(&lname.as_str()) { name } else { self.resolve_class(&name) };
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
        let line = self.line();
        if self.at(&Tok::Op("[")) || (self.at_kw("list") && self.peek_at(1) == &Tok::Op("(")) {
            let close = if self.next() == Tok::Op("[") { Tok::Op("]") } else { Tok::Op(")") };
            if close == Tok::Op(")") {
                self.next();
            }
            let items = self.array_items(&close)?;
            return Ok(Expr::new(list_pattern(items, line)?, line));
        }
        let target = self.postfix_expr()?;
        if !target.is_lvalue() {
            return Err(ParseError { msg: "syntax error, foreach target must be a variable".into(), line: self.line() });
        }
        Ok(target)
    }

    /// Items of an array literal or list pattern up to `end`. Trailing commas are
    /// allowed; a skipped element becomes a Hole for a list pattern to accept
    fn array_items(&mut self, end: &Tok) -> PResult<Vec<ArrayItem>> {
        self.in_items += 1;
        let items = self.array_items_inner(end);
        self.in_items -= 1;
        items
    }

    fn array_items_inner(&mut self, end: &Tok) -> PResult<Vec<ArrayItem>> {
        let mut items = Vec::new();
        while !self.eat(end) {
            let line = self.line();
            if self.eat(&Tok::Op(",")) {
                items.push(ArrayItem { key: None, value: Expr::new(ExprKind::Hole, line), by_ref: false });
                continue;
            }
            let mut by_ref = self.eat(&Tok::Op("&"));
            let first = self.expr()?;
            let item = if !by_ref && self.eat(&Tok::Op("=>")) {
                by_ref = self.eat(&Tok::Op("&"));
                ArrayItem { key: Some(first), value: self.expr()?, by_ref }
            } else {
                ArrayItem { key: None, value: first, by_ref }
            };
            if item.by_ref && !item.value.is_lvalue() && !matches!(item.value.kind, ExprKind::List(_)) {
                return Err(ParseError { msg: "Cannot assign reference to non referenceable value".into(), line });
            }
            items.push(item);
            if !self.eat(&Tok::Op(",")) {
                self.expect(end.clone())?;
                break;
            }
        }
        Ok(items)
    }

    /// `[...]` after the opening bracket: a list pattern when assigned to, else an array
    fn array_literal(&mut self, end: &Tok, line: u32) -> PResult<ExprKind> {
        let items = self.array_items(end)?;
        if self.at(&Tok::Op("=")) {
            return list_pattern(items, line);
        }
        // Inside another `[...]` the decision belongs to the outermost one
        if self.in_items == 0 {
            check_array(&items)?;
        }
        Ok(ExprKind::Array(items))
    }

    /// `match (subject) { conds => expr, default => expr }`, after the keyword
    fn match_expr(&mut self) -> PResult<ExprKind> {
        let subject = self.paren_expr()?;
        self.expect(Tok::Op("{"))?;
        let mut arms = Vec::new();
        while !self.eat(&Tok::Op("}")) {
            let line = self.line();
            let conds = if self.at_kw("default") && matches!(self.peek_at(1), Tok::Op("=>" | ",")) {
                self.next();
                self.eat(&Tok::Op(","));
                if arms.iter().any(|a: &MatchArm| a.conds.is_none()) {
                    return Err(ParseError { msg: "Match expressions may only contain one default arm".into(), line });
                }
                None
            } else {
                let mut conds = vec![self.expr()?];
                while self.eat(&Tok::Op(",")) {
                    if self.at(&Tok::Op("=>")) {
                        break;
                    }
                    conds.push(self.expr()?);
                }
                Some(conds)
            };
            self.expect(Tok::Op("=>"))?;
            arms.push(MatchArm { conds, body: self.expr()? });
            if !self.eat(&Tok::Op(",")) {
                self.expect(Tok::Op("}"))?;
                break;
            }
        }
        Ok(ExprKind::Match(Box::new(subject), arms))
    }

    /// `__CLASS__` and friends; None for an ordinary constant name
    fn magic_const(&self, name: &str, line: u32) -> Option<ExprKind> {
        let s = |s: &str| ExprKind::Str(Rc::new(s.as_bytes().to_vec()));
        let class = self.class.as_deref().unwrap_or("");
        Some(match name.to_ascii_uppercase().as_str() {
            "__LINE__" => ExprKind::Int(line.into()),
            "__FILE__" => ExprKind::Magic(Magic::File),
            "__DIR__" => ExprKind::Magic(Magic::Dir),
            "__CLASS__" if self.in_trait => ExprKind::Magic(Magic::Class),
            "__CLASS__" => s(class),
            "__TRAIT__" => s(if self.in_trait { class } else { "" }),
            "__NAMESPACE__" => s(&self.names.ns),
            "__FUNCTION__" | "__METHOD__" => match &self.func {
                None => s(""),
                Some(FuncCtx::Closure) => ExprKind::Magic(Magic::ClosureName),
                Some(FuncCtx::Named(f)) => s(f),
                Some(FuncCtx::Method(m)) if name.eq_ignore_ascii_case("__METHOD__") => s(&format!("{class}::{m}")),
                Some(FuncCtx::Method(m)) => s(m),
            },
            _ => return None,
        })
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
            if matches!(lhs.kind, ExprKind::List(_)) && self.eat(&Tok::Op("=")) {
                let rhs = Box::new(self.expr_bp(P_ASSIGN)?);
                let line = lhs.line;
                lhs = Expr::new(ExprKind::Assign(Box::new(lhs), rhs), line);
                continue;
            }
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
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("yield") => {
                self.next();
                if self.func.is_none() {
                    let msg = "The \"yield\" expression can only be used inside a function".into();
                    return Err(ParseError { msg, line });
                }
                self.yield_line.get_or_insert(line);
                if self.eat_kw("from") {
                    ExprKind::YieldFrom(Box::new(self.expr_bp(P_ASSIGN)?))
                } else if matches!(self.peek(), Tok::Semicolon | Tok::Eof | Tok::Op(")" | "," | "]" | "}" | "=>")) {
                    ExprKind::Yield { key: None, value: None }
                } else {
                    let first = self.expr_bp(P_ASSIGN)?;
                    if self.eat(&Tok::Op("=>")) {
                        let value = self.expr_bp(P_ASSIGN)?;
                        ExprKind::Yield { key: Some(Box::new(first)), value: Some(Box::new(value)) }
                    } else {
                        ExprKind::Yield { key: None, value: Some(Box::new(first)) }
                    }
                }
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
                        StrPart::Code(tokens) => parse_embedded(self, tokens)?,
                    });
                }
                ExprKind::Interp(exprs)
            }
            Tok::Variable(name) => ExprKind::Var(name),
            Tok::Op("[") => self.array_literal(&Tok::Op("]"), line)?,
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect(Tok::Op(")"))?;
                return Ok(e);
            }
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("function") => self.closure(line)?,
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("fn") => self.arrow_fn(line)?,
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("match") && self.at(&Tok::Op("(")) => self.match_expr()?,
            Tok::Ident(kw) if kw.eq_ignore_ascii_case("list") && self.at(&Tok::Op("(")) => {
                self.next();
                let items = self.array_items(&Tok::Op(")"))?;
                if !self.at(&Tok::Op("=")) && self.in_items == 0 {
                    let mut e = self.unexpected::<()>().unwrap_err();
                    e.msg.push_str(", expecting \"=\"");
                    return Err(e);
                }
                list_pattern(items, line)?
            }
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
                self.static_member(self.class_ref_named(&name))?
            }
            Tok::Ident(name) => {
                let lname = name.to_ascii_lowercase();
                if self.at(&Tok::Op("(")) && matches!(lname.as_str(), "array" | "isset" | "empty") {
                    self.next();
                    let kind = match lname.as_str() {
                        "array" => self.array_literal(&Tok::Op(")"), line)?,
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
                    let kind = match self.resolve_symbol(&name, UseKind::Function) {
                        (name, None) => ExprKind::Call(name, args),
                        (name, Some(global)) => ExprKind::NsCall(name, global, args),
                    };
                    return Ok(Expr::new(kind, line));
                }
                match name.trim_start_matches('\\').to_ascii_lowercase().as_str() {
                    "true" => ExprKind::Bool(true),
                    "false" => ExprKind::Bool(false),
                    "null" => ExprKind::Null,
                    _ => match self.magic_const(&name, line) {
                        Some(magic) => magic,
                        None => match self.resolve_symbol(&name, UseKind::Const) {
                            (name, None) => ExprKind::Const(name),
                            (name, Some(global)) => ExprKind::NsConst(name, global),
                        },
                    },
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

/// Rejects skipped elements in what turned out to be an array literal
fn check_array(items: &[ArrayItem]) -> PResult<()> {
    for item in items {
        match &item.value.kind {
            ExprKind::Hole => {
                return Err(ParseError { msg: "Cannot use empty array elements in arrays".into(), line: item.value.line });
            }
            ExprKind::Array(inner) => check_array(inner)?,
            ExprKind::List(_) => {
                return Err(ParseError { msg: "Cannot use list() outside of an assignment".into(), line: item.value.line });
            }
            _ => {}
        }
    }
    Ok(())
}

/// Turns the items of `[...]` / `list(...)` into a destructuring pattern
fn list_pattern(items: Vec<ArrayItem>, line: u32) -> PResult<ExprKind> {
    let keyed = items.iter().filter(|i| !matches!(i.value.kind, ExprKind::Hole)).map(|i| i.key.is_some()).collect::<Vec<_>>();
    if keyed.contains(&true) && keyed.contains(&false) {
        return Err(ParseError { msg: "Cannot mix keyed and unkeyed array entries in assignments".into(), line });
    }
    if keyed.is_empty() {
        return Err(ParseError { msg: "Cannot use empty list".into(), line });
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let target = match item.value.kind {
            ExprKind::Hole => {
                out.push(None);
                continue;
            }
            ExprKind::Array(inner) => Expr::new(list_pattern(inner, item.value.line)?, item.value.line),
            _ if item.value.is_lvalue() || matches!(item.value.kind, ExprKind::List(_)) => item.value,
            _ => return Err(ParseError { msg: "Assignments can only happen to writable values".into(), line }),
        };
        out.push(Some(ListItem { key: item.key, target, by_ref: item.by_ref }));
    }
    Ok(ExprKind::List(out))
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
        ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Const(_)
        | ExprKind::NsConst(..)
        | ExprKind::Hole
        | ExprKind::Magic(_) => {}
        ExprKind::Match(subject, arms) => {
            free_vars(subject, out);
            for arm in arms {
                arm.conds.iter().flatten().for_each(|c| free_vars(c, out));
                free_vars(&arm.body, out);
            }
        }
        ExprKind::List(items) => {
            for item in items.iter().flatten() {
                if let Some(k) = &item.key {
                    free_vars(k, out);
                }
                free_vars(&item.target, out);
            }
        }
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
        ExprKind::Call(_, a) | ExprKind::NsCall(_, _, a) => args(a, out),
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
        ExprKind::YieldFrom(x) => free_vars(x, out),
        ExprKind::Yield { key, value } => {
            key.iter().chain(value).for_each(|x| free_vars(x, out));
        }
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
