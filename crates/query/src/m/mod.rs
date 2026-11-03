//! The M formula language: lexer, parser ([`parse`]) and evaluator ([`eval`]).

pub mod ast;
pub mod eval;
pub(crate) mod lexer;
mod parser;

pub use ast::Expr;
pub use parser::{LetDoc, parse, parse_let};

/// Deepest nesting the parser and evaluator accept.
pub const MAX_DEPTH: usize = 200;
