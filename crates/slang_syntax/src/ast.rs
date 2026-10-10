use crate::lexer::Span;

/// A parsed type, mirroring `Type` in `src/ast.h`.
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    Named(String),
    Array(Box<Type>),
    Map {
        key: Box<Type>,
        val: Box<Type>,
    },
    Opt(Box<Type>),
    Result {
        ok: Box<Type>,
        err: Box<Type>,
    },
    Chan(Box<Type>),
    Join(Box<Type>),
    Ref(Box<Type>),
    RefMut(Box<Type>),
    Own(Box<Type>),
    Gc(Box<Type>),
    Ptr(Box<Type>),
    Fn {
        params: Vec<Type>,
        ret: Option<Box<Type>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Import {
        path: String,
        alias: Option<String>,
        span: Span,
    },
    Link {
        lib: String,
        span: Span,
    },
    Let {
        pub_: bool,
        name: String,
        ty: Option<Type>,
        init: Expr,
        span: Span,
    },
    Fn(FnDecl),
    Struct(StructDecl),
    Impl(ImplDecl),
    Enum(EnumDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub pub_: bool,
    pub extern_: bool,
    pub name: String,
    pub tparams: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub pub_: bool,
    pub gc: bool,
    pub name: String,
    pub tparams: Vec<String>,
    pub fields: Vec<Field>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImplDecl {
    pub type_name: String,
    pub tparams: Vec<String>,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub pub_: bool,
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub value: Option<i64>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub items: Vec<Item>,
    /// Top-level executable statements (main package body).
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        ty: Option<Type>,
        init: Expr,
        span: Span,
    },
    Assign {
        target: Expr,
        op: String,
        value: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then: Block,
        else_: Option<Else>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    ForRange {
        name: String,
        start: Expr,
        end: Expr,
        inclusive: bool,
        body: Block,
        span: Span,
    },
    ForIn {
        name: String,
        name2: Option<String>,
        iter: Expr,
        body: Block,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Expr {
        expr: Expr,
        span: Span,
    },
    GuardLet {
        name: String,
        expr: Expr,
        err_name: Option<String>,
        err_expr: Option<Expr>,
        body: Block,
        span: Span,
    },
    Spawn {
        call: Expr,
        span: Span,
    },
    IfLet {
        name: String,
        expr: Expr,
        then: Block,
        err_name: Option<String>,
        err_expr: Option<Expr>,
        else_: Option<Block>,
        span: Span,
    },
    Select {
        cases: Vec<SelectCase>,
        default: Option<Block>,
        span: Span,
    },
    Switch {
        scrut: Expr,
        cases: Vec<SwitchCase>,
        default: Option<Block>,
        span: Span,
    },
    Unsafe {
        body: Block,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Else {
    Block(Block),
    If(Box<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectCase {
    pub is_send: bool,
    pub bind: Option<String>,
    pub ch: Expr,
    pub val: Option<Expr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
    pub vals: Vec<Expr>,
    pub body: SwitchBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SwitchBody {
    Block(Block),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Int {
        value: i64,
        big_u64: bool,
    },
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Bool(bool),
    Ident(String),
    Binary {
        op: String,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: String,
        operand: Box<Expr>,
    },
    Call {
        name: Option<String>,
        callee: Option<Box<Expr>>,
        args: Vec<Expr>,
    },
    Cast {
        operand: Box<Expr>,
        ty: Type,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Slice {
        base: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool,
    },
    List(Vec<Expr>),
    MapLit(Vec<(Expr, Expr)>),
    Field {
        base: Box<Expr>,
        name: String,
    },
    MethodCall {
        recv: Box<Expr>,
        name: String,
        args: Vec<Expr>,
    },
    StructLit {
        tyname: String,
        fields: Vec<(String, Expr)>,
    },
    Spawn(Box<Expr>),
    Switch {
        scrut: Box<Expr>,
        cases: Vec<SwitchCase>,
        default: Option<Box<SwitchBody>>,
    },
}
