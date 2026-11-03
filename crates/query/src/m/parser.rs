//! M parser (precedence climbing). Keeps the source spans of `let` bindings so a query's steps
//! can be shown and edited one at a time ([`parse_let`]).

use super::MAX_DEPTH;
use super::ast::{BinOp, Expr, ListItem, UnOp};
use super::lexer::{Tok, lex};
use crate::value::MType;
use crate::{Error, Result};

pub fn parse(src: &str) -> Result<Expr> {
    let toks = lex(src)?;
    let mut p = Parser { src, toks, pos: 0, depth: 0 };
    let e = p.expr()?;
    if p.pos < p.toks.len() {
        return Err(Error::Syntax(format!("unexpected {} after the end of the expression", p.describe())));
    }
    Ok(e)
}

/// A query's top-level `let`: each binding's name and the byte span of its expression, and the
/// span of the `in` expression.
#[derive(Clone, Debug, PartialEq)]
pub struct LetDoc {
    pub bindings: Vec<(String, std::ops::Range<usize>)>,
    pub result: std::ops::Range<usize>,
}

/// Splits a query into steps when it is a `let … in …` expression.
pub fn parse_let(src: &str) -> Result<Option<LetDoc>> {
    let toks = lex(src)?;
    let mut p = Parser { src, toks, pos: 0, depth: 0 };
    if !p.is_kw("let") {
        parse(src)?;
        return Ok(None);
    }
    p.pos += 1;
    let mut bindings = Vec::new();
    loop {
        let name = p.binding_name()?;
        p.expect("=")?;
        let start = p.offset();
        p.expr()?;
        let end = p.prev_end();
        bindings.push((name, start..end));
        if p.eat(",") {
            continue;
        }
        if p.is_kw("in") {
            p.pos += 1;
            break;
        }
        return Err(Error::Syntax(format!("expected `,` or `in` but found {}", p.describe())));
    }
    let start = p.offset();
    p.expr()?;
    let end = p.prev_end();
    if p.pos < p.toks.len() {
        return Err(Error::Syntax(format!("unexpected {} after the end of the query", p.describe())));
    }
    Ok(Some(LetDoc { bindings, result: start..end }))
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<(Tok, usize)>,
    pos: usize,
    depth: usize,
}

const KEYWORDS: &[&str] = &[
    "let",
    "in",
    "if",
    "then",
    "else",
    "each",
    "try",
    "otherwise",
    "error",
    "and",
    "or",
    "not",
    "true",
    "false",
    "null",
    "type",
    "meta",
    "as",
    "is",
    "section",
    "shared",
];

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }

    fn peek_at(&self, k: usize) -> Option<&Tok> {
        self.toks.get(self.pos + k).map(|t| &t.0)
    }

    fn offset(&self) -> usize {
        self.toks.get(self.pos).map(|t| t.1).unwrap_or(self.src.len())
    }

    /// End offset of the token before the current one.
    fn prev_end(&self) -> usize {
        // The next token's start, minus trailing whitespace and comments, is the end.
        let next = self.offset();
        let s = self.src.get(..next).unwrap_or_default();
        let trimmed = s.trim_end();
        let trimmed = trimmed.strip_suffix(',').map(str::trim_end).unwrap_or(trimmed);
        trimmed.len().max(self.toks.get(self.pos.saturating_sub(1)).map(|t| t.1 + 1).unwrap_or(0)).min(next)
    }

    fn describe(&self) -> String {
        match self.peek() {
            None => "the end of the text".into(),
            Some(Tok::Num(n)) => format!("number {n}"),
            Some(Tok::Text(t)) => format!("text \"{t}\""),
            Some(Tok::Ident(t)) => format!("`{t}`"),
            Some(Tok::Hash(t)) => format!("`#{t}`"),
            Some(Tok::Punct(p)) => format!("`{p}`"),
        }
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(x)) if *x == p)
    }

    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(x)) if x == k)
    }

    fn eat(&mut self, p: &str) -> bool {
        if self.is_punct(p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, p: &str) -> Result<()> {
        if self.eat(p) { Ok(()) } else { Err(Error::Syntax(format!("expected `{p}` but found {}", self.describe()))) }
    }

    fn expect_kw(&mut self, k: &str) -> Result<()> {
        if self.is_kw(k) {
            self.pos += 1;
            Ok(())
        } else {
            Err(Error::Syntax(format!("expected `{k}` but found {}", self.describe())))
        }
    }

    fn binding_name(&mut self) -> Result<String> {
        match self.peek().cloned() {
            Some(Tok::Ident(n)) => {
                self.pos += 1;
                Ok(n)
            }
            _ => Err(Error::Syntax(format!("expected a name but found {}", self.describe()))),
        }
    }

    fn expr(&mut self) -> Result<Expr> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::Syntax("the expression is nested too deeply".into()));
        }
        let r = self.expr_inner();
        self.depth -= 1;
        r
    }

    fn expr_inner(&mut self) -> Result<Expr> {
        if self.is_kw("let") {
            self.pos += 1;
            let mut bindings = Vec::new();
            loop {
                let name = self.binding_name()?;
                self.expect("=")?;
                bindings.push((name, self.expr()?));
                if self.eat(",") {
                    continue;
                }
                self.expect_kw("in")?;
                break;
            }
            return Ok(Expr::Let(bindings, Box::new(self.expr()?)));
        }
        if self.is_kw("if") {
            self.pos += 1;
            let c = self.expr()?;
            self.expect_kw("then")?;
            let t = self.expr()?;
            self.expect_kw("else")?;
            let e = self.expr()?;
            return Ok(Expr::If(Box::new(c), Box::new(t), Box::new(e)));
        }
        if self.is_kw("each") {
            self.pos += 1;
            return Ok(Expr::Lambda(vec!["_".into()], Box::new(self.expr()?)));
        }
        if self.is_kw("try") {
            self.pos += 1;
            let e = self.expr()?;
            let other = if self.is_kw("otherwise") {
                self.pos += 1;
                Some(Box::new(self.expr()?))
            } else {
                None
            };
            return Ok(Expr::Try(Box::new(e), other));
        }
        if self.is_kw("error") {
            self.pos += 1;
            return Ok(Expr::ErrorRaise(Box::new(self.expr()?)));
        }
        if self.is_punct("(")
            && let Some(params) = self.lambda_params()
        {
            return Ok(Expr::Lambda(params, Box::new(self.expr()?)));
        }
        self.binary(0)
    }

    /// `(a, optional b as text) as number =>`: the parameter names, consuming through `=>`.
    fn lambda_params(&mut self) -> Option<Vec<String>> {
        let save = self.pos;
        self.pos += 1;
        let mut names = Vec::new();
        let ok = (|| {
            if self.eat(")") {
                return true;
            }
            loop {
                if self.is_kw("optional") {
                    self.pos += 1;
                }
                let Some(Tok::Ident(n)) = self.peek().cloned() else { return false };
                self.pos += 1;
                names.push(n);
                if self.is_kw("as") {
                    self.pos += 1;
                    if self.is_kw("nullable") {
                        self.pos += 1;
                    }
                    if !matches!(self.peek(), Some(Tok::Ident(_))) {
                        return false;
                    }
                    self.pos += 1;
                }
                if self.eat(",") {
                    continue;
                }
                return self.eat(")");
            }
        })();
        if ok {
            if self.is_kw("as") {
                self.pos += 1;
                if self.is_kw("nullable") {
                    self.pos += 1;
                }
                self.pos += 1;
            }
            if self.eat("=>") {
                return Some(names);
            }
        }
        self.pos = save;
        None
    }

    fn infix(&self) -> Option<(u8, BinOp)> {
        let t = self.peek()?;
        Some(match t {
            Tok::Punct("??") => (1, BinOp::Coalesce),
            Tok::Ident(k) if k == "or" => (2, BinOp::Or),
            Tok::Ident(k) if k == "and" => (3, BinOp::And),
            Tok::Ident(k) if k == "is" => (4, BinOp::Is),
            Tok::Ident(k) if k == "as" => (4, BinOp::As),
            Tok::Punct("=") => (5, BinOp::Eq),
            Tok::Punct("<>") => (5, BinOp::Ne),
            Tok::Punct("<") => (6, BinOp::Lt),
            Tok::Punct("<=") => (6, BinOp::Le),
            Tok::Punct(">") => (6, BinOp::Gt),
            Tok::Punct(">=") => (6, BinOp::Ge),
            Tok::Punct("+") => (7, BinOp::Add),
            Tok::Punct("-") => (7, BinOp::Sub),
            Tok::Punct("&") => (7, BinOp::Concat),
            Tok::Punct("*") => (8, BinOp::Mul),
            Tok::Punct("/") => (8, BinOp::Div),
            _ => return None,
        })
    }

    fn binary(&mut self, min: u8) -> Result<Expr> {
        let mut lhs = self.unary()?;
        while let Some((bp, op)) = self.infix() {
            if bp < min {
                break;
            }
            self.pos += 1;
            if matches!(op, BinOp::As | BinOp::Is) {
                let t = self.type_expr()?;
                lhs = Expr::Binary(op, Box::new(lhs), Box::new(Expr::Type(t)));
                continue;
            }
            let rhs = self.binary(bp + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        if self.is_kw("meta") {
            self.pos += 1;
            self.binary(9)?;
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        if self.eat("-") {
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return Ok(Expr::Unary(UnOp::Plus, Box::new(self.unary()?)));
        }
        if self.is_kw("not") {
            self.pos += 1;
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.unary()?)));
        }
        let p = self.primary()?;
        self.postfix(p)
    }

    /// Text between the current `[` and its `]` (generalized identifiers like `[Order Date]`).
    fn bracket_name(&mut self) -> Result<(String, bool)> {
        // At the token after `[`.
        let start = self.offset();
        let mut depth = 0;
        while let Some(t) = self.peek() {
            match t {
                Tok::Punct("[") => depth += 1,
                Tok::Punct("]") if depth == 0 => break,
                Tok::Punct("]") => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }
        let end = self.offset();
        self.expect("]")?;
        let raw = self.src.get(start..end).unwrap_or_default().trim();
        let name = if raw.starts_with("#\"") {
            match lex(raw)?.first() {
                Some((Tok::Ident(n), _)) => n.clone(),
                _ => raw.to_string(),
            }
        } else {
            raw.to_string()
        };
        let optional = self.eat("?");
        Ok((name, optional))
    }

    fn postfix(&mut self, mut e: Expr) -> Result<Expr> {
        loop {
            if self.eat("(") {
                let args = self.args(")")?;
                e = Expr::Call(Box::new(e), args);
            } else if self.is_punct("[") {
                self.pos += 1;
                if self.is_punct("[") {
                    let names = self.projection()?;
                    let opt = self.eat("?");
                    e = Expr::Project(Box::new(e), names, opt);
                } else {
                    let (name, opt) = self.bracket_name()?;
                    e = Expr::Field(Some(Box::new(e)), name, opt);
                }
            } else if self.eat("{") {
                let idx = self.expr()?;
                self.expect("}")?;
                let opt = self.eat("?");
                e = Expr::Item(Box::new(e), Box::new(idx), opt);
            } else {
                return Ok(e);
            }
        }
    }

    /// `[A], [B]]` after the opening `[`.
    fn projection(&mut self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        loop {
            self.expect("[")?;
            names.push(self.bracket_name()?.0);
            if self.eat(",") {
                continue;
            }
            self.expect("]")?;
            return Ok(names);
        }
    }

    fn args(&mut self, close: &str) -> Result<Vec<Expr>> {
        let mut args = Vec::new();
        if self.eat(close) {
            return Ok(args);
        }
        loop {
            args.push(self.expr()?);
            if self.eat(",") {
                continue;
            }
            self.expect(close)?;
            return Ok(args);
        }
    }

    fn primary(&mut self) -> Result<Expr> {
        let Some(t) = self.peek().cloned() else { return Err(Error::Syntax("the expression ended early".into())) };
        match t {
            Tok::Num(n) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Tok::Text(s) => {
                self.pos += 1;
                Ok(Expr::Text(s))
            }
            Tok::Hash(name) => {
                self.pos += 1;
                match name.as_str() {
                    "nan" => Ok(Expr::Num(f64::NAN)),
                    "infinity" => Ok(Expr::Num(f64::INFINITY)),
                    "shared" | "sections" => Ok(Expr::Record(vec![])),
                    _ => Ok(Expr::Ident(format!("#{name}"))),
                }
            }
            Tok::Ident(name) => {
                self.pos += 1;
                match name.as_str() {
                    "true" => Ok(Expr::Bool(true)),
                    "false" => Ok(Expr::Bool(false)),
                    "null" => Ok(Expr::Null),
                    "type" => Ok(Expr::Type(self.type_expr()?)),
                    k if KEYWORDS.contains(&k) => Err(Error::Syntax(format!("unexpected `{k}`"))),
                    _ => Ok(Expr::Ident(name)),
                }
            }
            Tok::Punct("@") => {
                self.pos += 1;
                self.primary()
            }
            Tok::Punct("(") => {
                self.pos += 1;
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Punct("{") => {
                self.pos += 1;
                let mut items = Vec::new();
                if !self.eat("}") {
                    loop {
                        let a = self.expr()?;
                        if self.eat("..") {
                            items.push(ListItem::Range(a, self.expr()?));
                        } else {
                            items.push(ListItem::One(a));
                        }
                        if self.eat(",") {
                            continue;
                        }
                        self.expect("}")?;
                        break;
                    }
                }
                Ok(Expr::List(items))
            }
            Tok::Punct("[") => {
                self.pos += 1;
                self.record_or_field()
            }
            Tok::Punct("...") => {
                self.pos += 1;
                Ok(Expr::ErrorRaise(Box::new(Expr::Text("Not implemented".into()))))
            }
            _ => Err(Error::Syntax(format!("unexpected {}", self.describe()))),
        }
    }

    /// After `[`: a record `[A = 1, B = 2]`, `[]`, a projection `[[A], [B]]` of `_`, or a field
    /// access `[A]` of `_`.
    fn record_or_field(&mut self) -> Result<Expr> {
        if self.eat("]") {
            return Ok(Expr::Record(vec![]));
        }
        if self.is_punct("[") {
            let names = self.projection()?;
            return Ok(Expr::Project(Box::new(Expr::Ident("_".into())), names, false));
        }
        // A record when a name is followed by `=` before the next `,` or `]` at this level.
        let mut k = 0;
        let mut is_record = false;
        while let Some(t) = self.peek_at(k) {
            match t {
                Tok::Punct("=") => {
                    is_record = true;
                    break;
                }
                Tok::Punct("]") | Tok::Punct(",") | Tok::Punct("[") | Tok::Punct("(") => break,
                _ => k += 1,
            }
        }
        if !is_record {
            let (name, opt) = self.bracket_name()?;
            return Ok(Expr::Field(None, name, opt));
        }
        let mut fields = Vec::new();
        loop {
            let start = self.offset();
            while !self.is_punct("=") && self.peek().is_some() {
                self.pos += 1;
            }
            let raw = self.src.get(start..self.offset()).unwrap_or_default().trim().to_string();
            let name = if raw.starts_with("#\"") {
                match lex(&raw)?.first() {
                    Some((Tok::Ident(n), _)) => n.clone(),
                    _ => raw,
                }
            } else {
                raw
            };
            self.expect("=")?;
            fields.push((name, self.expr()?));
            if self.eat(",") {
                continue;
            }
            self.expect("]")?;
            return Ok(Expr::Record(fields));
        }
    }

    /// A type after `type`, `as` or `is`: primitive names, `nullable`, and `table [...]`,
    /// `{...}`, `[...]`, `function (...)` forms (kept as their base kind).
    fn type_expr(&mut self) -> Result<MType> {
        if self.is_kw("nullable") {
            self.pos += 1;
        }
        if self.is_punct("{") || self.is_punct("[") {
            let list = self.is_punct("{");
            self.skip_balanced()?;
            return Ok(if list { MType::List } else { MType::Record });
        }
        let Some(Tok::Ident(name)) = self.peek().cloned() else {
            return Err(Error::Syntax(format!("expected a type but found {}", self.describe())));
        };
        self.pos += 1;
        let t = match name.as_str() {
            "any" | "anynonnull" => MType::Any,
            "null" => MType::Null,
            "number" => MType::Number,
            "text" => MType::Text,
            "logical" => MType::Logical,
            "date" => MType::Date,
            "datetime" | "datetimezone" => MType::DateTime,
            "time" => MType::Time,
            "duration" => MType::Duration,
            "binary" => MType::Binary,
            "list" => MType::List,
            "record" => MType::Record,
            "table" => {
                if self.is_punct("[") || self.is_punct("{") {
                    self.skip_balanced()?;
                }
                MType::Table
            }
            "function" => {
                if self.is_punct("(") {
                    self.skip_balanced()?;
                }
                if self.is_kw("as") {
                    self.pos += 1;
                    self.type_expr()?;
                }
                MType::Function
            }
            "type" => MType::Type,
            "Int64.Type" | "Int32.Type" | "Int16.Type" | "Int8.Type" => MType::Int64,
            "Currency.Type" => MType::Currency,
            "Percentage.Type" => MType::Percentage,
            "Number.Type" | "Double.Type" | "Decimal.Type" | "Single.Type" => MType::Number,
            "Text.Type" => MType::Text,
            "Date.Type" => MType::Date,
            "DateTime.Type" => MType::DateTime,
            "Logical.Type" => MType::Logical,
            _ => MType::Any,
        };
        Ok(t)
    }

    fn skip_balanced(&mut self) -> Result<()> {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                None => return Err(Error::Syntax("unbalanced brackets".into())),
                Some(Tok::Punct("[" | "{" | "(")) => depth += 1,
                Some(Tok::Punct("]" | "}" | ")")) => depth -= 1,
                _ => {}
            }
            self.pos += 1;
            if depth <= 0 {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_queries() {
        let src = r#"let
    Source = Excel.CurrentWorkbook(){[Name="Sales"]}[Content],
    #"Changed Type" = Table.TransformColumnTypes(Source,{{"Order Date", type date}, {"Amount", Int64.Type}}),
    #"Added Custom" = Table.AddColumn(#"Changed Type", "Double", each [Amount] * 2, type number),
    #"Filtered Rows" = Table.SelectRows(#"Added Custom", each ([Region] <> "East" and [Order Date] >= #date(2024, 1, 1)))
in
    #"Filtered Rows""#;
        let doc = parse_let(src).unwrap().unwrap();
        assert_eq!(doc.bindings.len(), 4);
        assert_eq!(doc.bindings[1].0, "Changed Type");
        assert_eq!(&src[doc.bindings[0].1.clone()], r#"Excel.CurrentWorkbook(){[Name="Sales"]}[Content]"#);
        assert_eq!(&src[doc.result.clone()], r##"#"Filtered Rows""##);
        assert!(parse(src).is_ok());
    }

    #[test]
    fn generalized_field_names_and_lambdas() {
        let e = parse("each [Order Date] + #duration(1,0,0,0)").unwrap();
        let Expr::Lambda(p, body) = e else { panic!() };
        assert_eq!(p, vec!["_"]);
        let Expr::Binary(BinOp::Add, l, _) = *body else { panic!() };
        assert_eq!(*l, Expr::Field(None, "Order Date".into(), false));
        assert!(matches!(parse("(x, optional y as number) => x + (y ?? 0)").unwrap(), Expr::Lambda(ref p, _) if p.len() == 2));
        assert!(matches!(parse("[A = 1, #\"B c\" = \"x\"]").unwrap(), Expr::Record(ref f) if f[1].0 == "B c"));
        assert!(parse("{1..5, 7}").is_ok());
        assert!(parse("let a = 1 in").is_err());
        let deep = format!("{}1{}", "(".repeat(3000), ")".repeat(3000));
        assert!(parse(&deep).is_err());
    }
}
