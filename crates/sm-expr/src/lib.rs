//! Expressions for every user-entered number, as in Inventor's parameters:
//! `40 mm`, `Length / 2`, `30 deg`, `max(Width; 2 in) * 3 ul`.
//!
//! - Numbers carry units. Lengths evaluate to millimetres, angles to
//!   radians, whatever unit was typed.
//! - Evaluation checks dimensions: `Length * Width` in a length field is an
//!   error, and so is adding a length to an angle.
//! - Functions and constants follow Inventor: `sin cos tan asin acos atan
//!   sqrt abs ceil floor round min max`, `PI` and `E`. Arguments are
//!   separated by `;` (Inventor's separator) or `,`.
//! - Identifiers name parameters (user parameters and model parameters such
//!   as `d0`). Unit, function and constant names are reserved and can't be
//!   parameter names; see [`is_reserved`].
//!
//! Expressions come from files, so their size and nesting are bounded.

use std::fmt;

/// Longest expression text accepted.
pub const MAX_LEN: usize = 1_000;
/// Deepest nesting of brackets, functions and unary operators.
pub const MAX_DEPTH: usize = 64;

/// Powers of length and angle. `{0, 0}` is unitless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Dimension {
    pub length: i8,
    pub angle: i8,
}

impl Dimension {
    pub const UNITLESS: Self = Self {
        length: 0,
        angle: 0,
    };
    pub const LENGTH: Self = Self {
        length: 1,
        angle: 0,
    };
    pub const ANGLE: Self = Self {
        length: 0,
        angle: 1,
    };

    fn times(self, other: Self) -> Option<Self> {
        Some(Self {
            length: self.length.checked_add(other.length)?,
            angle: self.angle.checked_add(other.angle)?,
        })
    }

    fn inverse(self) -> Option<Self> {
        Some(Self {
            length: self.length.checked_neg()?,
            angle: self.angle.checked_neg()?,
        })
    }
}

impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.length, self.angle) {
            (0, 0) => write!(f, "unitless"),
            (1, 0) => write!(f, "length"),
            (0, 1) => write!(f, "angle"),
            (l, a) => write!(f, "length^{l} angle^{a}"),
        }
    }
}

/// A value in internal units (mm, rad) with its dimension.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub dim: Dimension,
}

impl Quantity {
    pub fn new(value: f64, dim: Dimension) -> Self {
        Self { value, dim }
    }

    /// The value, if it has dimension `dim`.
    pub fn expect(self, dim: Dimension) -> Result<f64, ExprError> {
        if self.dim == dim {
            Ok(self.value)
        } else {
            Err(ExprError::WrongDimension {
                expected: dim,
                found: self.dim,
            })
        }
    }
}

/// A unit a number or parameter can be given in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    pub name: &'static str,
    /// Size of one unit in internal units (mm or rad).
    pub scale: f64,
    pub dim: Dimension,
}

const UNITS: &[Unit] = &[
    unit("mm", 1.0, Dimension::LENGTH),
    unit("cm", 10.0, Dimension::LENGTH),
    unit("m", 1000.0, Dimension::LENGTH),
    unit("um", 0.001, Dimension::LENGTH),
    unit("in", 25.4, Dimension::LENGTH),
    unit("ft", 304.8, Dimension::LENGTH),
    unit("deg", std::f64::consts::PI / 180.0, Dimension::ANGLE),
    unit("rad", 1.0, Dimension::ANGLE),
    unit("ul", 1.0, Dimension::UNITLESS),
];

const fn unit(name: &'static str, scale: f64, dim: Dimension) -> Unit {
    Unit { name, scale, dim }
}

const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "asin", "acos", "atan", "sqrt", "abs", "ceil", "floor", "round", "min",
    "max",
];
const CONSTANTS: &[(&str, f64)] = &[("PI", std::f64::consts::PI), ("E", std::f64::consts::E)];

/// The unit called `name`, such as `mm`, `in`, `deg` or `ul`.
pub fn unit_named(name: &str) -> Option<Unit> {
    UNITS.iter().copied().find(|u| u.name == name)
}

/// True for names that can't be parameters: units, functions, constants.
pub fn is_reserved(name: &str) -> bool {
    unit_named(name).is_some()
        || FUNCTIONS.contains(&name)
        || CONSTANTS.iter().any(|&(c, _)| c == name)
}

/// A valid parameter name: a letter or `_`, then letters, digits or `_`,
/// and not reserved.
pub fn is_parameter_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && !is_reserved(name)
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExprError {
    #[error("expression is empty")]
    Empty,
    #[error("expression is longer than {MAX_LEN} characters")]
    TooLong,
    #[error("expression is nested more than {MAX_DEPTH} deep")]
    TooDeep,
    #[error("unexpected {0:?} at position {1}")]
    Unexpected(String, usize),
    #[error("{0:?} is not a number")]
    BadNumber(String),
    #[error("unknown parameter {0:?}")]
    UnknownName(String),
    #[error("{0:?} is a function; call it with brackets")]
    BareFunction(String),
    #[error("{0:?} takes {1} argument(s)")]
    WrongArgCount(String, &'static str),
    #[error("expected {expected}, found {found}")]
    WrongDimension {
        expected: Dimension,
        found: Dimension,
    },
    #[error("can't combine {0} with {1}")]
    Mismatch(Dimension, Dimension),
    #[error("exponent must be a unitless whole number when the base has units")]
    BadExponent,
    #[error("result is not a finite number")]
    NotFinite,
    #[error("parameter {0:?} depends on itself")]
    Cycle(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Op(char),
}

/// Splits `text` into tokens with their byte offsets.
fn tokens(text: &str) -> Result<Vec<(Token, usize, usize)>, ExprError> {
    if text.len() > MAX_LEN {
        return Err(ExprError::TooLong);
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().expect("i is a char boundary");
        if c.is_whitespace() {
            i += c.len_utf8();
        } else if c.is_ascii_digit()
            || (c == '.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = i;
            while i < text.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            // An exponent only if digits follow, so `2em` stays `2 em`.
            if i < text.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let mut j = i + 1;
                if j < text.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                    j += 1;
                }
                if j < text.len() && bytes[j].is_ascii_digit() {
                    i = j;
                    while i < text.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let s = &text[start..i];
            let n: f64 = s.parse().map_err(|_| ExprError::BadNumber(s.into()))?;
            out.push((Token::Number(n), start, i));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while let Some(c) = text[i..].chars().next() {
                if c.is_alphanumeric() || c == '_' {
                    i += c.len_utf8();
                } else {
                    break;
                }
            }
            out.push((Token::Ident(text[start..i].into()), start, i));
        } else if "+-*/^(),;".contains(c) {
            out.push((Token::Op(if c == ';' { ',' } else { c }), i, i + 1));
            i += 1;
        } else {
            return Err(ExprError::Unexpected(c.into(), i));
        }
    }
    Ok(out)
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr(Node);

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Number(f64),
    /// A literal with a unit, already in internal units.
    Quantity(Quantity),
    Name(String),
    Neg(Box<Node>),
    Binary(char, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
}

struct Parser {
    tokens: Vec<(Token, usize, usize)>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|t| &t.0)
    }

    fn unexpected(&self) -> ExprError {
        match self.tokens.get(self.pos) {
            Some((t, at, _)) => ExprError::Unexpected(
                match t {
                    Token::Number(n) => n.to_string(),
                    Token::Ident(s) => s.clone(),
                    Token::Op(c) => c.to_string(),
                },
                *at,
            ),
            None => ExprError::Unexpected("end of expression".into(), usize::MAX),
        }
    }

    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Token::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn nested<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ExprError>,
    ) -> Result<T, ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ExprError::TooDeep);
        }
        let r = f(self);
        self.depth -= 1;
        r
    }

    fn expr(&mut self) -> Result<Node, ExprError> {
        let mut left = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Token::Op(c @ ('+' | '-'))) => *c,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.term()?;
            left = Node::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn term(&mut self) -> Result<Node, ExprError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Op(c @ ('*' | '/'))) => *c,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.unary()?;
            left = Node::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Result<Node, ExprError> {
        if self.eat('-') {
            return self.nested(|p| Ok(Node::Neg(Box::new(p.unary()?))));
        }
        if self.eat('+') {
            return self.nested(|p| p.unary());
        }
        let base = self.atom()?;
        if self.eat('^') {
            // Right-associative, and binds tighter than a leading minus on
            // the exponent: 2^-1 is 0.5.
            let exponent = self.nested(|p| p.unary())?;
            return Ok(Node::Binary('^', Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Node, ExprError> {
        let Some((token, _, _)) = self.tokens.get(self.pos).cloned() else {
            return Err(self.unexpected());
        };
        let node = match token {
            Token::Number(n) => {
                self.pos += 1;
                Node::Number(n)
            }
            Token::Op('(') => {
                self.pos += 1;
                let inner = self.nested(|p| p.expr())?;
                if !self.eat(')') {
                    return Err(self.unexpected());
                }
                inner
            }
            Token::Ident(name) => {
                self.pos += 1;
                if FUNCTIONS.contains(&name.as_str()) {
                    if !self.eat('(') {
                        return Err(ExprError::BareFunction(name));
                    }
                    let args = self.nested(|p| {
                        let mut args = vec![p.expr()?];
                        while p.eat(',') {
                            args.push(p.expr()?);
                        }
                        Ok(args)
                    })?;
                    if !self.eat(')') {
                        return Err(self.unexpected());
                    }
                    Node::Call(name, args)
                } else {
                    Node::Name(name)
                }
            }
            Token::Op(_) => return Err(self.unexpected()),
        };
        // A unit right after a number or bracket applies to it: `40 mm`,
        // `(a + 2) mm`.
        if let Some(Token::Ident(u)) = self.peek() {
            if let Some(unit) = unit_named(u) {
                self.pos += 1;
                let scaled = Node::Quantity(Quantity::new(unit.scale, unit.dim));
                return Ok(match node {
                    Node::Number(n) => Node::Quantity(Quantity::new(n * unit.scale, unit.dim)),
                    other => Node::Binary('*', Box::new(other), Box::new(scaled)),
                });
            }
        }
        Ok(node)
    }
}

impl Expr {
    pub fn parse(text: &str) -> Result<Self, ExprError> {
        let tokens = tokens(text)?;
        if tokens.is_empty() {
            return Err(ExprError::Empty);
        }
        let mut p = Parser {
            tokens,
            pos: 0,
            depth: 0,
        };
        let node = p.expr()?;
        if p.pos != p.tokens.len() {
            return Err(p.unexpected());
        }
        Ok(Self(node))
    }

    /// The parameter names this expression uses, each once, in order of
    /// first use. Units are not names.
    pub fn names(&self) -> Vec<&str> {
        fn walk<'a>(n: &'a Node, out: &mut Vec<&'a str>) {
            match n {
                Node::Name(s) => {
                    if !out.contains(&s.as_str()) && !CONSTANTS.iter().any(|&(c, _)| c == s) {
                        out.push(s);
                    }
                }
                Node::Neg(a) => walk(a, out),
                Node::Binary(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Node::Call(_, args) => args.iter().for_each(|a| walk(a, out)),
                Node::Number(_) | Node::Quantity(_) => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &mut out);
        out
    }

    /// Evaluates the expression, looking parameters up with `lookup`.
    pub fn eval(&self, lookup: &dyn Fn(&str) -> Option<Quantity>) -> Result<Quantity, ExprError> {
        let q = eval(&self.0, lookup)?;
        if q.value.is_finite() {
            Ok(q)
        } else {
            Err(ExprError::NotFinite)
        }
    }
}

fn eval(n: &Node, lookup: &dyn Fn(&str) -> Option<Quantity>) -> Result<Quantity, ExprError> {
    use Dimension as D;
    let unitless = |v: f64| Quantity::new(v, D::UNITLESS);
    Ok(match n {
        Node::Number(v) => unitless(*v),
        Node::Quantity(q) => *q,
        Node::Name(s) => match CONSTANTS.iter().find(|&&(c, _)| c == s) {
            Some(&(_, v)) => unitless(v),
            None => lookup(s).ok_or_else(|| ExprError::UnknownName(s.clone()))?,
        },
        Node::Neg(a) => {
            let a = eval(a, lookup)?;
            Quantity::new(-a.value, a.dim)
        }
        Node::Binary(op, a, b) => {
            let (a, b) = (eval(a, lookup)?, eval(b, lookup)?);
            let overflow = || ExprError::Mismatch(a.dim, b.dim);
            match op {
                '+' | '-' => {
                    if a.dim != b.dim {
                        return Err(ExprError::Mismatch(a.dim, b.dim));
                    }
                    let v = if *op == '+' {
                        a.value + b.value
                    } else {
                        a.value - b.value
                    };
                    Quantity::new(v, a.dim)
                }
                '*' => Quantity::new(a.value * b.value, a.dim.times(b.dim).ok_or_else(overflow)?),
                '/' => Quantity::new(
                    a.value / b.value,
                    a.dim
                        .times(b.dim.inverse().ok_or_else(overflow)?)
                        .ok_or_else(overflow)?,
                ),
                '^' => {
                    let e = b.expect(D::UNITLESS).map_err(|_| ExprError::BadExponent)?;
                    let dim = if a.dim == D::UNITLESS {
                        D::UNITLESS
                    } else {
                        if e.fract() != 0.0 || e.abs() > 16.0 {
                            return Err(ExprError::BadExponent);
                        }
                        let k = e as i8;
                        D {
                            length: a.dim.length.checked_mul(k).ok_or(ExprError::BadExponent)?,
                            angle: a.dim.angle.checked_mul(k).ok_or(ExprError::BadExponent)?,
                        }
                    };
                    Quantity::new(a.value.powf(e), dim)
                }
                _ => unreachable!("parser only builds + - * / ^"),
            }
        }
        Node::Call(f, args) => {
            let args = args
                .iter()
                .map(|a| eval(a, lookup))
                .collect::<Result<Vec<_>, _>>()?;
            let one = || match args.as_slice() {
                [a] => Ok(*a),
                _ => Err(ExprError::WrongArgCount(f.clone(), "1")),
            };
            match f.as_str() {
                // Trig takes an angle, or a unitless number as radians.
                "sin" | "cos" | "tan" => {
                    let a = one()?;
                    if a.dim != D::ANGLE && a.dim != D::UNITLESS {
                        return Err(ExprError::WrongDimension {
                            expected: D::ANGLE,
                            found: a.dim,
                        });
                    }
                    let v = match f.as_str() {
                        "sin" => a.value.sin(),
                        "cos" => a.value.cos(),
                        _ => a.value.tan(),
                    };
                    unitless(v)
                }
                "asin" | "acos" | "atan" => {
                    let v = one()?.expect(D::UNITLESS)?;
                    let r = match f.as_str() {
                        "asin" => v.asin(),
                        "acos" => v.acos(),
                        _ => v.atan(),
                    };
                    Quantity::new(r, D::ANGLE)
                }
                "sqrt" => {
                    let a = one()?;
                    if a.dim.length % 2 != 0 || a.dim.angle % 2 != 0 {
                        return Err(ExprError::Mismatch(a.dim, a.dim));
                    }
                    let dim = D {
                        length: a.dim.length / 2,
                        angle: a.dim.angle / 2,
                    };
                    Quantity::new(a.value.sqrt(), dim)
                }
                "abs" | "ceil" | "floor" | "round" => {
                    let a = one()?;
                    let v = match f.as_str() {
                        "abs" => a.value.abs(),
                        "ceil" => a.value.ceil(),
                        "floor" => a.value.floor(),
                        _ => a.value.round(),
                    };
                    Quantity::new(v, a.dim)
                }
                "min" | "max" => {
                    let Some((first, rest)) = args.split_first().filter(|(_, r)| !r.is_empty())
                    else {
                        return Err(ExprError::WrongArgCount(f.clone(), "2 or more"));
                    };
                    let mut best = *first;
                    for a in rest {
                        if a.dim != best.dim {
                            return Err(ExprError::Mismatch(best.dim, a.dim));
                        }
                        let better = if f == "min" {
                            a.value < best.value
                        } else {
                            a.value > best.value
                        };
                        if better {
                            best = *a;
                        }
                    }
                    best
                }
                _ => unreachable!("parser only calls known functions"),
            }
        }
    })
}

/// Rewrites every use of parameter `old` in `text` as `new`, keeping the
/// rest of the text (spacing, units, number formatting) as typed. Used when
/// a parameter is renamed.
pub fn rename(text: &str, old: &str, new: &str) -> Result<String, ExprError> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (token, start, end) in tokens(text)? {
        if token == Token::Ident(old.into()) {
            out.push_str(&text[last..start]);
            out.push_str(new);
            last = end;
        }
    }
    out.push_str(&text[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Dimension as D;

    fn params(name: &str) -> Option<Quantity> {
        match name {
            "Length" => Some(Quantity::new(40.0, D::LENGTH)),
            "Width" => Some(Quantity::new(20.0, D::LENGTH)),
            "Count" => Some(Quantity::new(3.0, D::UNITLESS)),
            "Tilt" => Some(Quantity::new(std::f64::consts::FRAC_PI_6, D::ANGLE)),
            "a" => Some(Quantity::new(1.0, D::LENGTH)),
            _ => None,
        }
    }

    fn ev(text: &str) -> Result<Quantity, ExprError> {
        Expr::parse(text)?.eval(&params)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * (1.0 + b.abs())
    }

    #[test]
    fn literals_convert_to_internal_units() {
        for (text, mm) in [
            ("40 mm", 40.0),
            ("40mm", 40.0),
            ("4 cm", 40.0),
            ("0.04 m", 40.0),
            ("1 in", 25.4),
            ("1 ft", 304.8),
            ("1500 um", 1.5),
            ("1.5e1 mm", 15.0),
            (".5 in", 12.7),
        ] {
            let q = ev(text).unwrap();
            assert_eq!(q.dim, D::LENGTH, "{text}");
            assert!(close(q.value, mm), "{text}: {}", q.value);
        }
        assert!(close(
            ev("30 deg").unwrap().expect(D::ANGLE).unwrap(),
            std::f64::consts::FRAC_PI_6
        ));
        assert_eq!(ev("2 rad").unwrap(), Quantity::new(2.0, D::ANGLE));
        assert_eq!(ev("3 ul").unwrap(), Quantity::new(3.0, D::UNITLESS));
        assert_eq!(ev("3").unwrap(), Quantity::new(3.0, D::UNITLESS));
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("Length / 2").unwrap(), Quantity::new(20.0, D::LENGTH));
        assert_eq!(ev("Length - Width * 2 / 4").unwrap().value, 30.0);
        assert_eq!(ev("(Length - Width) * 2").unwrap().value, 40.0);
        assert_eq!(ev("-Width + 5 mm").unwrap().value, -15.0);
        assert_eq!(ev("2 ^ 3 ^ 2").unwrap().value, 512.0);
        assert_eq!(ev("2 ^ -1").unwrap().value, 0.5);
        assert_eq!(ev("(Count + 1) mm").unwrap(), Quantity::new(4.0, D::LENGTH));
        assert_eq!(ev("10 in - 254 mm").unwrap().value, 0.0);
        let area = ev("Length * Width").unwrap();
        assert_eq!(
            area.dim,
            D {
                length: 2,
                angle: 0
            }
        );
        assert_eq!(
            ev("sqrt(Length * Width * 2)").unwrap(),
            Quantity::new(40.0, D::LENGTH)
        );
        assert_eq!(
            ev("Width ^ 2 / Length").unwrap(),
            Quantity::new(10.0, D::LENGTH)
        );
    }

    #[test]
    fn functions_and_constants() {
        assert!(close(ev("sin(30 deg)").unwrap().value, 0.5));
        assert!(close(ev("cos(Tilt * 2)").unwrap().value, 0.5));
        assert!(close(ev("tan(PI / 4)").unwrap().value, 1.0));
        assert!(close(
            ev("asin(0.5)").unwrap().expect(D::ANGLE).unwrap(),
            std::f64::consts::FRAC_PI_6
        ));
        assert!(close(ev("E").unwrap().value, std::f64::consts::E));
        assert_eq!(ev("max(Width; 1 in; 5 mm)").unwrap().value, 25.4);
        assert_eq!(ev("min(Width, Length)").unwrap().value, 20.0);
        assert_eq!(ev("abs(-Width)").unwrap().value, 20.0);
        assert_eq!(ev("ceil(2.1 mm)").unwrap(), Quantity::new(3.0, D::LENGTH));
        assert_eq!(ev("floor(2.9)").unwrap().value, 2.0);
        assert_eq!(ev("round(2.5)").unwrap().value, 3.0);
    }

    #[test]
    fn dimension_errors() {
        assert!(matches!(
            ev("Length + 30 deg"),
            Err(ExprError::Mismatch(..))
        ));
        assert!(matches!(ev("Length + 3"), Err(ExprError::Mismatch(..))));
        assert!(matches!(
            ev("Length * Width").unwrap().expect(D::LENGTH),
            Err(ExprError::WrongDimension { .. })
        ));
        assert!(matches!(
            ev("sin(Length)"),
            Err(ExprError::WrongDimension { .. })
        ));
        assert!(matches!(
            ev("asin(Length)"),
            Err(ExprError::WrongDimension { .. })
        ));
        assert!(matches!(ev("sqrt(Length)"), Err(ExprError::Mismatch(..))));
        assert!(matches!(ev("Length ^ 0.5"), Err(ExprError::BadExponent)));
        assert!(matches!(ev("Length ^ Width"), Err(ExprError::BadExponent)));
        assert!(matches!(ev("Length ^ 100"), Err(ExprError::BadExponent)));
        assert!(matches!(
            ev("max(Length; 2 deg)"),
            Err(ExprError::Mismatch(..))
        ));
        // Powers of a dimension can't overflow the i8 exponent.
        let deep = vec!["a"; 130].join(" * ");
        assert!(matches!(ev(&deep), Err(ExprError::Mismatch(..))));
    }

    #[test]
    fn syntax_errors() {
        for text in [
            "",
            "   ",
            "1 +",
            "(1",
            "1)",
            "* 2",
            "1 2",
            "Length Width",
            "max()",
            "1 @ 2",
        ] {
            assert!(ev(text).is_err(), "{text:?}");
        }
        assert_eq!(ev(""), Err(ExprError::Empty));
        assert_eq!(ev("sin"), Err(ExprError::BareFunction("sin".into())));
        assert!(matches!(ev("max(1)"), Err(ExprError::WrongArgCount(..))));
        assert!(matches!(ev("sin(1; 2)"), Err(ExprError::WrongArgCount(..))));
        assert_eq!(ev("Height"), Err(ExprError::UnknownName("Height".into())));
        assert_eq!(ev("1 / 0"), Err(ExprError::NotFinite));
        assert_eq!(ev("sqrt(-1)"), Err(ExprError::NotFinite));
        assert!(matches!(ev("1..2"), Err(ExprError::BadNumber(_))));
    }

    #[test]
    fn hostile_input_is_bounded() {
        assert_eq!(ev(&"1+".repeat(600)), Err(ExprError::TooLong));
        assert_eq!(
            ev(&format!("{}1{}", "(".repeat(100), ")".repeat(100))),
            Err(ExprError::TooDeep)
        );
        assert_eq!(
            ev(&format!("{}1", "-".repeat(100))),
            Err(ExprError::TooDeep)
        );
        assert_eq!(
            ev(&format!("{}1", "2^".repeat(100))),
            Err(ExprError::TooDeep)
        );
        assert_eq!(
            ev(&format!("{}1{}", "abs(".repeat(100), ")".repeat(100))),
            Err(ExprError::TooDeep)
        );
        // Within the limits, deep nesting works.
        assert_eq!(
            ev(&format!("{}1{}", "(".repeat(50), ")".repeat(50)))
                .unwrap()
                .value,
            1.0
        );
        // Non-ASCII text is handled by character, never split mid-character.
        assert!(matches!(ev("Länge"), Err(ExprError::UnknownName(_))));
        assert!(matches!(ev("1 ø"), Err(ExprError::Unexpected(..))));
    }

    #[test]
    fn names_and_rename() {
        let e = Expr::parse("max(Length; d0) * 2 + Length - PI mm").unwrap();
        assert_eq!(e.names(), ["Length", "d0"]);
        assert_eq!(
            rename("Length/2 + Lengthy + 3 mm", "Length", "Size").unwrap(),
            "Size/2 + Lengthy + 3 mm"
        );
        assert_eq!(rename("d1*d10", "d1", "Gap").unwrap(), "Gap*d10");
    }

    #[test]
    fn reserved_and_parameter_names() {
        for r in ["mm", "in", "deg", "ul", "sin", "max", "PI", "E"] {
            assert!(is_reserved(r) && !is_parameter_name(r), "{r}");
        }
        for ok in ["Length", "d0", "_x", "Länge", "mmm", "pi"] {
            assert!(is_parameter_name(ok), "{ok}");
        }
        for bad in ["", "2x", "a b", "a-b"] {
            assert!(!is_parameter_name(bad), "{bad}");
        }
        assert_eq!(unit_named("in").unwrap().scale, 25.4);
        assert!(unit_named("furlong").is_none());
    }
}
