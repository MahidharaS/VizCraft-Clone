//! GridCraft Power Query: an engine for the M formula language.
//!
//! - [`m`]: the M lexer, parser and lazy evaluator; [`funcs`]: the function library (`Table.*`,
//!   `Text.*`, `List.*`, `Number.*`, `Date.*`, sources such as `Csv.Document`, `Json.Document`,
//!   `Excel.CurrentWorkbook`, `Folder.Files`).
//! - [`steps`]: queries as `let` expressions whose bindings are the applied steps, and the edits
//!   the editor makes to them (add, delete, rename and edit a step), plus the saved document.
//! - [`examples`]: Column From Examples, which finds an M formula that reproduces example values.
//! - [`profile`]: column quality, distribution and statistics.
//!
//! The crate depends on no other GridCraft crate; sources outside the queries (the workbook,
//! files) come through [`m::eval::Host`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod examples;
pub mod funcs;
pub mod m;
pub mod profile;
pub mod steps;
pub mod value;

pub use m::eval::{Evaluator, Host, NoHost};
pub use value::{MType, Record, Table, Value};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("Expression.SyntaxError: {0}")]
    Syntax(String),
    #[error("{0}")]
    Eval(String),
}

impl Error {
    pub fn message(&self) -> String {
        match self {
            Error::Syntax(m) => format!("Expression.SyntaxError: {m}"),
            Error::Eval(m) => m.clone(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Evaluates M text.
pub fn evaluate(src: &str, host: &dyn Host) -> Result<Value> {
    let e = m::parse(src)?;
    let ev = Evaluator::new(host);
    m::eval::check(ev.eval(&e, &m::eval::Env::default())?)
}
