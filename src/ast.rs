use std::rc::Rc;

/// A script file: its path as messages show it, and its declare(strict_types)
#[derive(Debug)]
pub struct SourceFile {
    pub path: Rc<str>,
    pub strict: bool,
}

/// A parsed file
pub struct Program {
    pub stmts: Vec<Stmt>,
    pub file: Rc<SourceFile>,
}

use crate::object::Visibility;

/// An expression node. `line` is where the node starts, as in php-src's AST
#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<Vec<u8>>),
    /// Interpolated string: the parts are concatenated
    Interp(Vec<Expr>),
    Var(String),
    Const(String),
    /// An unqualified constant in a namespace, with the global name to fall back to
    NsConst(String, String),
    Array(Vec<ArrayItem>),
    /// `$a[k]`, or `$a[]` (only valid as a write target) when the index is None
    Index(Box<Expr>, Option<Box<Expr>>),
    /// The target is a Var or Index expression
    Assign(Box<Expr>, Box<Expr>),
    /// `$a = &$b`: both sides are Var or Index expressions
    AssignRef(Box<Expr>, Box<Expr>),
    /// `$a += 1` etc. The operator is stored without the `=` (e.g. `+`)
    CompoundAssign(&'static str, Box<Expr>, Box<Expr>),
    IncDec { target: Box<Expr>, inc: bool, prefix: bool },
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    Call(String, Vec<Arg>),
    /// An unqualified call in a namespace: the namespaced function if it is
    /// defined, else the global one
    NsCall(String, String, Vec<Arg>),
    /// Calling the value of an expression: `$f(1)`, `(function () {})()`
    CallValue(Box<Expr>, Vec<Arg>),
    /// `function () use ($x) {}` and `fn () => ...`
    Closure(Rc<FuncDecl>),
    New(ClassRef, Vec<Arg>),
    MethodCall { object: Box<Expr>, name: Name, args: Vec<Arg>, nullsafe: bool },
    Prop { object: Box<Expr>, name: Name, nullsafe: bool },
    StaticCall { class: ClassRef, name: String, args: Vec<Arg> },
    StaticProp { class: ClassRef, name: String },
    /// `A::NAME`, including `A::class`
    ClassConst { class: ClassRef, name: String },
    InstanceOf(Box<Expr>, ClassRef),
    Clone(Box<Expr>),
    /// `@expr`: diagnostics raised while evaluating it are dropped
    Silence(Box<Expr>),
    /// `include`, `require` and their `_once` forms
    Include(IncludeKind, Box<Expr>),
    /// `exit` / `die`, with an optional status or message
    Exit(Option<Box<Expr>>),
    Throw(Box<Expr>),
    Isset(Vec<Expr>),
    Empty(Box<Expr>),
    /// `match (subject) { ... }`
    Match(Box<Expr>, Vec<MatchArm>),
    /// `[$a, 'k' => $b] = ...` / `list(...)`: only valid as an assignment target.
    /// Skipped positions (`[, $b]`) are None
    List(Vec<Option<ListItem>>),
    /// A skipped element in `[...]` that may still turn out to be a list pattern
    Hole,
    /// A magic constant that is only known at runtime
    Magic(Magic),
    /// `yield`, `yield $v`, `yield $k => $v`. Generator bodies are lowered so
    /// that these only remain as Stmt::Yield
    Yield { key: Option<Box<Expr>>, value: Option<Box<Expr>> },
    YieldFrom(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IncludeKind {
    Include,
    IncludeOnce,
    Require,
    RequireOnce,
}

impl IncludeKind {
    pub fn name(self) -> &'static str {
        match self {
            IncludeKind::Include => "include",
            IncludeKind::IncludeOnce => "include_once",
            IncludeKind::Require => "require",
            IncludeKind::RequireOnce => "require_once",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Magic {
    File,
    Dir,
    /// `__FUNCTION__` / `__METHOD__` inside a closure
    ClosureName,
    /// `__CLASS__` inside a trait: the class using it
    Class,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    /// None for `default`
    pub conds: Option<Vec<Expr>>,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub struct ListItem {
    pub key: Option<Expr>,
    /// A variable, offset, property or nested List
    pub target: Expr,
    pub by_ref: bool,
}

/// A member name after `->`: literal, or `$name` / `{expr}` evaluated at runtime
#[derive(Debug, Clone)]
pub enum Name {
    Lit(String),
    Dyn(Box<Expr>),
}

/// The class part of `new X`, `X::m()`, `instanceof X`
#[derive(Debug, Clone)]
pub enum ClassRef {
    Named(String),
    SelfRef,
    Parent,
    Static,
    /// `$obj::X`, `new $className`
    Dynamic(Box<Expr>),
}

impl ClassRef {
    pub fn from_name(name: &str) -> ClassRef {
        match name.to_ascii_lowercase().as_str() {
            "self" => ClassRef::SelfRef,
            "parent" => ClassRef::Parent,
            "static" => ClassRef::Static,
            _ => ClassRef::Named(name.trim_start_matches('\\').to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub value: Expr,
    /// `...$args`
    pub spread: bool,
    /// `name: value`
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ArrayItem {
    pub key: Option<Expr>,
    pub value: Expr,
    /// `[&$a]`: the element is a reference to the variable
    pub by_ref: bool,
}

impl Expr {
    pub fn new(kind: ExprKind, line: u32) -> Self {
        Expr { kind, line }
    }

    /// Whether isset() accepts the expression: a variable, offset or property access
    pub fn is_variable(&self) -> bool {
        matches!(self.kind, ExprKind::Var(_) | ExprKind::Index(..) | ExprKind::Prop { .. } | ExprKind::StaticProp { .. })
    }

    /// Whether the expression can be assigned to
    pub fn is_lvalue(&self) -> bool {
        match &self.kind {
            ExprKind::Var(_) | ExprKind::StaticProp { .. } => true,
            ExprKind::Prop { nullsafe, .. } => !nullsafe,
            ExprKind::Index(base, _) => base.is_lvalue(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Echo(Vec<Expr>),
    Expr(Expr),
    If(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>),
    While(Expr, Vec<Stmt>),
    DoWhile(Vec<Stmt>, Expr),
    For(Vec<Expr>, Vec<Expr>, Vec<Expr>, Vec<Stmt>),
    Foreach { subject: Expr, key: Option<Expr>, value: Expr, by_ref: bool, body: Vec<Stmt> },
    Break(u32),
    Continue(u32),
    Unset(Vec<Expr>),
    Block(Vec<Stmt>),
    Function(Rc<FuncDecl>),
    Return(Option<Expr>),
    Global(Vec<String>),
    /// `static $a = 1, $b;`
    StaticVar(Vec<(String, Option<Expr>)>),
    Try { body: Vec<Stmt>, catches: Vec<Catch>, finally: Option<Vec<Stmt>> },
    Class(Rc<ClassDecl>),
    /// Cases in order with their bodies; `default` has no condition
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    /// `const A = 1, B = 2;` at the top level
    Const(Vec<(String, Expr)>),
    /// `declare(strict_types=...)`, which can only be the first statement. The
    /// mode is recorded in the file's SourceFile
    StrictTypes,
    /// In a lowered generator body: a yield, whose result (the value sent in)
    /// goes to the hidden variable `target`
    Yield { key: Option<Expr>, value: Option<Expr>, target: Option<String> },
    YieldFrom { source: Expr, target: Option<String> },
    /// In a lowered generator body: a statement without yields, run as usual
    Plain(Box<Stmt>),
}

#[derive(Debug)]
pub struct ClassDecl {
    pub name: String,
    pub is_interface: bool,
    pub is_trait: bool,
    pub is_enum: bool,
    /// The backing type of an enum (`enum E: string`)
    pub backing: Option<TypeHint>,
    pub is_abstract: bool,
    pub is_final: bool,
    pub is_readonly: bool,
    pub parent: Option<String>,
    /// For an interface, the interfaces it extends
    pub interfaces: Vec<String>,
    /// In declaration order, which decides property layout
    pub members: Vec<Member>,
    pub line: u32,
}

#[derive(Debug)]
pub enum Member {
    Const { name: String, value: Expr, vis: Visibility },
    Prop { name: String, ty: Option<TypeHint>, default: Option<Expr>, vis: Visibility, is_static: bool, readonly: bool },
    Method { func: Rc<FuncDecl>, vis: Visibility, is_static: bool, is_abstract: bool, is_final: bool },
    /// `use A, B { ... }` inside a class body
    Use { traits: Vec<String>, rules: Vec<TraitRule> },
    /// `case Name = value;` in an enum
    Case { name: String, value: Option<Expr> },
}

impl ClassDecl {
    /// Classes, interfaces and traits that must exist before this one can be declared
    pub fn dependencies(&self) -> Vec<&str> {
        let mut deps: Vec<&str> = self.parent.iter().chain(&self.interfaces).map(String::as_str).collect();
        for m in &self.members {
            if let Member::Use { traits, .. } = m {
                deps.extend(traits.iter().map(String::as_str));
            }
        }
        deps
    }

    pub fn kind(&self) -> &'static str {
        if self.is_interface {
            "interface"
        } else if self.is_trait {
            "trait"
        } else if self.is_enum {
            "enum"
        } else {
            "class"
        }
    }
}

/// The adaptations in `use T { ... }`
#[derive(Debug)]
pub enum TraitRule {
    /// `A::m insteadof B, C`: m comes from A, not from B or C
    Insteadof { trait_name: String, method: String, excluded: Vec<String> },
    /// `[A::]m as [visibility] [alias]`
    Alias { trait_name: Option<String>, method: String, vis: Option<Visibility>, alias: Option<String> },
}

#[derive(Debug, Clone)]
pub struct Catch {
    /// Class names; any of them matches
    pub types: Vec<String>,
    /// PHP 8 allows catching without a variable
    pub var: Option<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FuncKind {
    Named,
    Closure,
    /// `fn () => expr`: captures used variables by value automatically
    Arrow,
    Method,
}

#[derive(Debug)]
pub struct FuncDecl {
    pub kind: FuncKind,
    pub name: String,
    /// Captured variables (`use ($a, &$b)`), with whether they are by reference.
    /// For arrow functions these are computed from the body
    pub uses: Vec<(String, bool)>,
    pub params: Vec<Param>,
    pub ret: Option<TypeHint>,
    pub body: Vec<Stmt>,
    pub line: u32,
    /// Contains `yield`: calling it creates a Generator, and the body is lowered
    /// (see genlower)
    pub is_generator: bool,
    /// Where it is declared, which decides the file of its diagnostics and its strict_types
    pub file: Rc<SourceFile>,
}

impl FuncDecl {
    /// Number of parameters that must be passed
    pub fn required(&self) -> usize {
        self.params.iter().rposition(|p| p.default.is_none() && !p.variadic).map_or(0, |i| i + 1)
    }
}

#[derive(Debug)]
pub struct Param {
    /// Constructor promotion: `public readonly int $x`
    pub promote: Option<(Visibility, bool)>,
    pub name: String,
    pub ty: Option<TypeHint>,
    pub default: Option<Expr>,
    pub by_ref: bool,
    pub variadic: bool,
}

/// A declared type such as `int`, `?string` or `int|float`
#[derive(Debug, Clone)]
pub struct TypeHint {
    /// Lowercased member types; `?T` is stored as `T` plus "null"
    pub types: Vec<String>,
    /// As written, for error messages
    pub display: String,
}

impl TypeHint {
    pub fn allows(&self, ty: &str) -> bool {
        self.types.iter().any(|t| t == ty || t == "mixed")
    }
}
