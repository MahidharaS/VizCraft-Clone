//! DAX: the Data Analysis Expressions language measures, calculated columns and queries are
//! written in. [`parse`] turns text into an [`Expr`]; [`crate::eval`] runs it against a model.

mod lexer;
mod parser;

pub use parser::{BinOp, Expr, Query, UnOp, parse, parse_query};

/// The deepest an expression may nest (parentheses, calls, operators).
pub const MAX_DEPTH: usize = 128;
