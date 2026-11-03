//! The M syntax tree.

use crate::value::MType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce,
    As,
    Is,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Null,
    Bool(bool),
    Num(f64),
    Text(String),
    Ident(String),
    /// `let a = …, b = … in body`.
    Let(Vec<(String, Expr)>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `(a, optional b) => body`; `each body` is a function of `_`.
    Lambda(Vec<String>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    /// `[Field]` (of `_` when there's no target), `t[Field]`, with `?` for optional access.
    Field(Option<Box<Expr>>, String, bool),
    /// `t[[A], [B]]`: a projection.
    Project(Box<Expr>, Vec<String>, bool),
    /// `list{0}` or `table{[Key = v]}`.
    Item(Box<Expr>, Box<Expr>, bool),
    Record(Vec<(String, Expr)>),
    /// List items; a range `a..b` is an item of two expressions.
    List(Vec<ListItem>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Try(Box<Expr>, Option<Box<Expr>>),
    ErrorRaise(Box<Expr>),
    Type(MType),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListItem {
    One(Expr),
    Range(Expr, Expr),
}
