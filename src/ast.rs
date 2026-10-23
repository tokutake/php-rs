use std::rc::Rc;

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
    Isset(Vec<Expr>),
    Empty(Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub value: Expr,
    /// `...$args`
    pub spread: bool,
}

#[derive(Debug, Clone)]
pub struct ArrayItem {
    pub key: Option<Expr>,
    pub value: Expr,
}

impl Expr {
    pub fn new(kind: ExprKind, line: u32) -> Self {
        Expr { kind, line }
    }

    /// Whether isset() accepts the expression: a variable or any offset access
    pub fn is_variable(&self) -> bool {
        matches!(self.kind, ExprKind::Var(_) | ExprKind::Index(..))
    }

    /// Whether the expression can be assigned to
    pub fn is_lvalue(&self) -> bool {
        match &self.kind {
            ExprKind::Var(_) => true,
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
}

#[derive(Debug)]
pub struct FuncDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeHint>,
    pub body: Vec<Stmt>,
    pub line: u32,
}

impl FuncDecl {
    /// Number of parameters that must be passed
    pub fn required(&self) -> usize {
        self.params.iter().rposition(|p| p.default.is_none() && !p.variadic).map_or(0, |i| i + 1)
    }
}

#[derive(Debug)]
pub struct Param {
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
