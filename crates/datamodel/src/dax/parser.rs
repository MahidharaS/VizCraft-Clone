//! DAX expressions and queries: the syntax tree and a precedence-climbing parser.

use super::MAX_DEPTH;
use super::lexer::{Tok, lex};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    /// `==`: equal without blank coercion.
    StrictEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    In,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    /// `Table[Column]`.
    Column {
        table: String,
        column: String,
    },
    /// `[Name]`: a measure, or a column of the current row context.
    Bracket(String),
    /// A bare name: a table, or a variable.
    Name(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// A function call; the name is upper-case.
    Call(String, Vec<Expr>),
    /// `VAR a = … VAR b = … RETURN body`.
    Var(Vec<(String, Expr)>, Box<Expr>),
    /// `{ 1, 2, 3 }` or `{ (1, "a"), (2, "b") }`.
    TableCtor(Vec<Vec<Expr>>),
}

/// A DAX query: `DEFINE MEASURE T[m] = … EVALUATE <table> ORDER BY …`.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    /// Query-scoped measures (table, name, expression).
    pub measures: Vec<(String, String, Expr)>,
    pub evaluate: Expr,
    /// Sort keys and whether each is descending.
    pub order_by: Vec<(Expr, bool)>,
}

pub fn parse(src: &str) -> Result<Expr> {
    let src = src.trim_start();
    let src = src.strip_prefix('=').unwrap_or(src);
    let mut p = Parser { toks: lex(src)?, pos: 0, depth: 0 };
    let e = p.expr(0)?;
    match p.peek() {
        None => Ok(e),
        Some(t) => Err(Error::Parse(format!("unexpected {} after the end of the expression", describe(t)))),
    }
}

pub fn parse_query(src: &str) -> Result<Query> {
    let mut p = Parser { toks: lex(src)?, pos: 0, depth: 0 };
    let mut measures = Vec::new();
    if p.keyword("DEFINE") {
        while p.keyword("MEASURE") {
            let (table, name) = match p.next() {
                Some(Tok::Ident(t) | Tok::Quoted(t)) => match p.next() {
                    Some(Tok::Bracket(n)) => (t, n),
                    _ => return Err(Error::Parse("expected `Table[Measure]` after MEASURE".into())),
                },
                _ => return Err(Error::Parse("expected `Table[Measure]` after MEASURE".into())),
            };
            p.expect_op("=")?;
            measures.push((table, name, p.expr(0)?));
        }
    }
    if !p.keyword("EVALUATE") {
        return Err(Error::Parse("a query starts with EVALUATE (or DEFINE)".into()));
    }
    let evaluate = p.expr(0)?;
    let mut order_by = Vec::new();
    if p.keyword("ORDER") {
        if !p.keyword("BY") {
            return Err(Error::Parse("expected BY after ORDER".into()));
        }
        loop {
            let e = p.expr(0)?;
            let desc = if p.keyword("DESC") {
                true
            } else {
                p.keyword("ASC");
                false
            };
            order_by.push((e, desc));
            if p.peek() == Some(&Tok::Comma) {
                p.pos += 1;
            } else {
                break;
            }
        }
    }
    match p.peek() {
        None => Ok(Query { measures, evaluate, order_by }),
        Some(t) => Err(Error::Parse(format!("unexpected {} after the query", describe(t)))),
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Number(n) => format!("number {n}"),
        Tok::Str(s) => format!("text \"{s}\""),
        Tok::Ident(s) => format!("`{s}`"),
        Tok::Quoted(s) => format!("'{s}'"),
        Tok::Bracket(s) => format!("[{s}]"),
        Tok::LParen => "`(`".into(),
        Tok::RParen => "`)`".into(),
        Tok::LBrace => "`{`".into(),
        Tok::RBrace => "`}`".into(),
        Tok::Comma => "`,`".into(),
        Tok::Op(o) => format!("`{o}`"),
    }
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

/// Binding power of infix operators (higher binds tighter).
fn infix(t: &Tok) -> Option<(u8, BinOp)> {
    Some(match t {
        Tok::Op("||") => (1, BinOp::Or),
        Tok::Op("&&") => (2, BinOp::And),
        Tok::Op("=") => (4, BinOp::Eq),
        Tok::Op("==") => (4, BinOp::StrictEq),
        Tok::Op("<>") => (4, BinOp::Ne),
        Tok::Op("<") => (4, BinOp::Lt),
        Tok::Op("<=") => (4, BinOp::Le),
        Tok::Op(">") => (4, BinOp::Gt),
        Tok::Op(">=") => (4, BinOp::Ge),
        Tok::Ident(k) if k.eq_ignore_ascii_case("IN") => (4, BinOp::In),
        Tok::Op("&") => (5, BinOp::Concat),
        Tok::Op("+") => (6, BinOp::Add),
        Tok::Op("-") => (6, BinOp::Sub),
        Tok::Op("*") => (7, BinOp::Mul),
        Tok::Op("/") => (7, BinOp::Div),
        Tok::Op("^") => (9, BinOp::Pow),
        _ => return None,
    })
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn keyword(&mut self, k: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Ident(s)) if s.eq_ignore_ascii_case(k)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: Tok) -> Result<()> {
        match self.next() {
            Some(got) if got == t => Ok(()),
            Some(got) => Err(Error::Parse(format!("expected {} but found {}", describe(&t), describe(&got)))),
            None => Err(Error::Parse(format!("expected {} but the expression ended", describe(&t)))),
        }
    }

    fn expect_op(&mut self, op: &'static str) -> Result<()> {
        self.expect(Tok::Op(op))
    }

    fn expr(&mut self, min: u8) -> Result<Expr> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::Parse("the expression is nested too deeply".into()));
        }
        let r = self.expr_inner(min);
        self.depth -= 1;
        r
    }

    fn expr_inner(&mut self, min: u8) -> Result<Expr> {
        let mut lhs = self.prefix()?;
        while let Some((bp, op)) = self.peek().and_then(infix) {
            if bp < min {
                break;
            }
            self.pos += 1;
            // `^` is right-associative, the others left-associative.
            let rhs = if op == BinOp::Pow { self.expr(bp)? } else { self.expr(bp + 1)? };
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<Expr> {
        let Some(t) = self.next() else { return Err(Error::Parse("the expression ended early".into())) };
        match t {
            Tok::Number(n) => Ok(Expr::Number(n)),
            Tok::Str(s) => Ok(Expr::Text(s)),
            Tok::Op("-") => Ok(Expr::Unary(UnOp::Neg, Box::new(self.expr(8)?))),
            Tok::Op("+") => self.expr(8),
            Tok::LParen => {
                let e = self.expr(0)?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Tok::LBrace => self.table_ctor(),
            Tok::Bracket(name) => Ok(Expr::Bracket(name)),
            Tok::Quoted(table) => match self.peek() {
                Some(Tok::Bracket(_)) => match self.next() {
                    Some(Tok::Bracket(column)) => Ok(Expr::Column { table, column }),
                    _ => Err(Error::Parse("expected a column".into())),
                },
                _ => Ok(Expr::Name(table)),
            },
            Tok::Ident(name) => self.ident(name),
            other => Err(Error::Parse(format!("unexpected {}", describe(&other)))),
        }
    }

    fn ident(&mut self, name: String) -> Result<Expr> {
        let upper = name.to_ascii_uppercase();
        if upper == "VAR" {
            return self.var_block();
        }
        if upper == "NOT" && self.peek() != Some(&Tok::LParen) {
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.expr(3)?)));
        }
        match self.peek() {
            Some(Tok::LParen) => {
                self.pos += 1;
                let mut args = Vec::new();
                if self.peek() != Some(&Tok::RParen) {
                    loop {
                        args.push(self.expr(0)?);
                        if self.peek() == Some(&Tok::Comma) {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Tok::RParen)?;
                if upper == "NOT" {
                    let mut it = args.into_iter();
                    return match (it.next(), it.next()) {
                        (Some(a), None) => Ok(Expr::Unary(UnOp::Not, Box::new(a))),
                        _ => Err(Error::Parse("NOT takes one argument".into())),
                    };
                }
                Ok(Expr::Call(upper, args))
            }
            Some(Tok::Bracket(_)) => match self.next() {
                Some(Tok::Bracket(column)) => Ok(Expr::Column { table: name, column }),
                _ => Err(Error::Parse("expected a column".into())),
            },
            _ => match upper.as_str() {
                "TRUE" => Ok(Expr::Bool(true)),
                "FALSE" => Ok(Expr::Bool(false)),
                _ => Ok(Expr::Name(name)),
            },
        }
    }

    fn var_block(&mut self) -> Result<Expr> {
        let mut bindings = Vec::new();
        loop {
            let name = match self.next() {
                Some(Tok::Ident(n)) => n,
                _ => return Err(Error::Parse("expected a variable name after VAR".into())),
            };
            self.expect_op("=")?;
            bindings.push((name, self.expr(0)?));
            if self.keyword("VAR") {
                continue;
            }
            if self.keyword("RETURN") {
                break;
            }
            return Err(Error::Parse("expected VAR or RETURN".into()));
        }
        Ok(Expr::Var(bindings, Box::new(self.expr(0)?)))
    }

    fn table_ctor(&mut self) -> Result<Expr> {
        let mut rows = Vec::new();
        if self.peek() == Some(&Tok::RBrace) {
            self.pos += 1;
            return Ok(Expr::TableCtor(rows));
        }
        loop {
            // `(a, b)` is a row of several columns; anything else is a one-column row.
            let row = match self.peek() {
                Some(Tok::LParen) => {
                    let save = self.pos;
                    self.pos += 1;
                    let mut items = vec![self.expr(0)?];
                    while self.peek() == Some(&Tok::Comma) {
                        self.pos += 1;
                        items.push(self.expr(0)?);
                    }
                    if items.len() == 1 {
                        self.pos = save;
                        vec![self.expr(0)?]
                    } else {
                        self.expect(Tok::RParen)?;
                        items
                    }
                }
                _ => vec![self.expr(0)?],
            };
            rows.push(row);
            match self.next() {
                Some(Tok::Comma) => continue,
                Some(Tok::RBrace) => break,
                _ => return Err(Error::Parse("expected `,` or `}` in the table constructor".into())),
            }
        }
        Ok(Expr::TableCtor(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence() {
        let e = parse("1 + 2 * 3 ^ 2").unwrap();
        let Expr::Binary(BinOp::Add, _, rhs) = e else { panic!() };
        assert!(matches!(*rhs, Expr::Binary(BinOp::Mul, _, _)));
        let e = parse("[A] > 1 && NOT [B] = 2 || TRUE").unwrap();
        assert!(matches!(e, Expr::Binary(BinOp::Or, _, _)));
    }

    #[test]
    fn calls_columns_vars() {
        let e = parse("=CALCULATE(SUM(Sales[Amount]), 'Product'[Color] IN {\"Red\", \"Blue\"})").unwrap();
        let Expr::Call(name, args) = e else { panic!() };
        assert_eq!(name, "CALCULATE");
        assert_eq!(args.len(), 2);
        let e = parse("VAR x = 2 VAR y = x * 3 RETURN y + 1").unwrap();
        assert!(matches!(e, Expr::Var(ref b, _) if b.len() == 2));
        assert!(parse("SUM(").is_err());
        assert!(parse("1 2").is_err());
    }

    #[test]
    fn queries() {
        let q = parse_query("DEFINE MEASURE Sales[Big] = [Total] * 2 EVALUATE VALUES(Product[Color]) ORDER BY Product[Color] DESC").unwrap();
        assert_eq!(q.measures.len(), 1);
        assert_eq!(q.order_by.len(), 1);
        assert!(q.order_by[0].1);
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let src = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
        assert!(parse(&src).is_err());
    }
}
