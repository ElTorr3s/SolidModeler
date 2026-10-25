//! A small, bounded reader for STEP Part 21 (ISO 10303-21) exchange files.
//!
//! It only builds the instance table: every `#id = ...;` record with its raw
//! parameters. Meaning is given to the records by the rest of the importer.
//! Input is untrusted, so every loop is bounded by the input length, nesting
//! is capped, and malformed text is an error, never a panic.

use super::{ImportError, Limits};
use std::collections::HashMap;

/// One parameter of a record.
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Int(i64),
    Real(f64),
    Str(String),
    /// `.NAME.`, including the logicals `.T.`, `.F.` and `.U.`.
    Enum(String),
    Ref(u64),
    List(Vec<Param>),
    /// `$`: no value.
    Unset,
    /// `*`: value derived by a supertype.
    Derived,
    /// A typed value such as `LENGTH_MEASURE(25.4)`.
    Typed(String, Box<Param>),
    /// A `"..."` binary literal, kept as written.
    Binary(String),
}

/// A simple record `NAME(params)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub name: String,
    pub params: Vec<Param>,
}

/// An entity instance: a simple record, or a complex instance made of
/// several partial records `(A(..) B(..))`, sorted by name as the standard
/// requires.
#[derive(Debug, Clone, PartialEq)]
pub enum Entity {
    Simple(Record),
    Complex(Vec<Record>),
}

impl Entity {
    /// The record named `name`: the simple record itself, or the matching
    /// part of a complex instance.
    pub fn record(&self, name: &str) -> Option<&Record> {
        match self {
            Entity::Simple(r) if r.name == name => Some(r),
            Entity::Simple(_) => None,
            Entity::Complex(parts) => parts.iter().find(|r| r.name == name),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.record(name).is_some()
    }

    /// The type name of a simple instance, or `None` for a complex one.
    pub fn simple_name(&self) -> Option<&str> {
        match self {
            Entity::Simple(r) => Some(&r.name),
            Entity::Complex(_) => None,
        }
    }

    pub fn records(&self) -> &[Record] {
        match self {
            Entity::Simple(r) => std::slice::from_ref(r),
            Entity::Complex(parts) => parts,
        }
    }
}

/// A parsed exchange file.
#[derive(Debug, Default)]
pub struct Exchange {
    /// Header records (`FILE_DESCRIPTION`, `FILE_NAME`, `FILE_SCHEMA`, ...).
    pub header: Vec<Record>,
    pub entities: HashMap<u64, Entity>,
}

impl Exchange {
    pub fn header_record(&self, name: &str) -> Option<&Record> {
        self.header.iter().find(|r| r.name == name)
    }

    /// The schema names listed in `FILE_SCHEMA`, upper-cased.
    pub fn schemas(&self) -> Vec<String> {
        let Some(rec) = self.header_record("FILE_SCHEMA") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(Param::List(items)) = rec.params.first() {
            for item in items {
                if let Param::Str(s) = item {
                    out.push(s.trim().to_ascii_uppercase());
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Keyword(String),
    Instance(u64),
    Int(i64),
    Real(f64),
    Str(String),
    Enum(String),
    Binary(String),
    LParen,
    RParen,
    Comma,
    Semicolon,
    Equals,
    Dollar,
    Star,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
}

impl<'a> Lexer<'a> {
    fn error(&self, message: impl Into<String>) -> ImportError {
        ImportError::Syntax {
            line: self.line,
            message: message.into(),
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_space_and_comments(&mut self) -> Result<(), ImportError> {
        loop {
            match self.peek_byte() {
                Some(b'\n') => {
                    self.line += 1;
                    self.pos += 1;
                }
                Some(c) if c.is_ascii_whitespace() => self.pos += 1,
                Some(b'/') if self.src.get(self.pos + 1) == Some(&b'*') => {
                    let start_line = self.line;
                    self.pos += 2;
                    loop {
                        match self.peek_byte() {
                            None => {
                                return Err(ImportError::Syntax {
                                    line: start_line,
                                    message: "unterminated comment".into(),
                                })
                            }
                            Some(b'*') if self.src.get(self.pos + 1) == Some(&b'/') => {
                                self.pos += 2;
                                break;
                            }
                            Some(b'\n') => {
                                self.line += 1;
                                self.pos += 1;
                            }
                            Some(_) => self.pos += 1,
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn next(&mut self) -> Result<Option<Token>, ImportError> {
        self.skip_space_and_comments()?;
        let Some(c) = self.peek_byte() else {
            return Ok(None);
        };
        let token = match c {
            b'(' => {
                self.pos += 1;
                Token::LParen
            }
            b')' => {
                self.pos += 1;
                Token::RParen
            }
            b',' => {
                self.pos += 1;
                Token::Comma
            }
            b';' => {
                self.pos += 1;
                Token::Semicolon
            }
            b'=' => {
                self.pos += 1;
                Token::Equals
            }
            b'$' => {
                self.pos += 1;
                Token::Dollar
            }
            b'*' => {
                self.pos += 1;
                Token::Star
            }
            b'#' => {
                self.pos += 1;
                let digits = self.take_while(|b| b.is_ascii_digit());
                if digits.is_empty() {
                    return Err(self.error("expected digits after '#'"));
                }
                let id = digits
                    .parse::<u64>()
                    .map_err(|_| self.error("instance number too large"))?;
                Token::Instance(id)
            }
            b'\'' => Token::Str(self.string()?),
            b'"' => {
                self.pos += 1;
                let body = self.take_while(|b| b.is_ascii_hexdigit());
                if self.peek_byte() != Some(b'"') {
                    return Err(self.error("unterminated binary literal"));
                }
                self.pos += 1;
                Token::Binary(body)
            }
            b'.' if self
                .src
                .get(self.pos + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_') =>
            {
                self.pos += 1;
                let name = self.take_while(|b| b.is_ascii_alphanumeric() || b == b'_');
                if self.peek_byte() != Some(b'.') {
                    return Err(self.error("unterminated enumeration"));
                }
                self.pos += 1;
                Token::Enum(name.to_ascii_uppercase())
            }
            b'+' | b'-' | b'0'..=b'9' | b'.' => self.number()?,
            b'!' | b'_' | b'A'..=b'Z' | b'a'..=b'z' => {
                let start = self.pos;
                self.pos += 1;
                self.take_while(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
                let word = String::from_utf8_lossy(&self.src[start..self.pos]);
                Token::Keyword(word.to_ascii_uppercase())
            }
            other => {
                return Err(self.error(format!("unexpected character {:?}", char::from(other))))
            }
        };
        Ok(Some(token))
    }

    fn take_while(&mut self, keep: impl Fn(u8) -> bool) -> String {
        let start = self.pos;
        while self.peek_byte().is_some_and(&keep) {
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()
    }

    fn number(&mut self) -> Result<Token, ImportError> {
        let start = self.pos;
        if matches!(self.peek_byte(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        self.take_while(|b| b.is_ascii_digit());
        let mut real = false;
        if self.peek_byte() == Some(b'.') {
            real = true;
            self.pos += 1;
            self.take_while(|b| b.is_ascii_digit());
        }
        if matches!(self.peek_byte(), Some(b'E' | b'e')) {
            real = true;
            self.pos += 1;
            if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exp = self.take_while(|b| b.is_ascii_digit());
            if exp.is_empty() {
                return Err(self.error("malformed exponent"));
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.error("malformed number"))?;
        if !text.bytes().any(|b| b.is_ascii_digit()) {
            return Err(self.error(format!("malformed number {text:?}")));
        }
        if real {
            let value: f64 = text
                .parse()
                .map_err(|_| self.error(format!("malformed real {text:?}")))?;
            if !value.is_finite() {
                return Err(self.error(format!("real out of range {text:?}")));
            }
            Ok(Token::Real(value))
        } else {
            match text.parse::<i64>() {
                Ok(v) => Ok(Token::Int(v)),
                // Integers too large for i64 are still meaningful as reals.
                Err(_) => text
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .map(Token::Real)
                    .ok_or_else(|| self.error(format!("malformed integer {text:?}"))),
            }
        }
    }

    /// Reads a `'...'` string and decodes the Part 21 escapes.
    fn string(&mut self) -> Result<String, ImportError> {
        let start_line = self.line;
        self.pos += 1;
        let mut raw = Vec::new();
        loop {
            match self.peek_byte() {
                None => {
                    return Err(ImportError::Syntax {
                        line: start_line,
                        message: "unterminated string".into(),
                    })
                }
                Some(b'\'') if self.src.get(self.pos + 1) == Some(&b'\'') => {
                    raw.push(b'\'');
                    self.pos += 2;
                }
                Some(b'\'') => {
                    self.pos += 1;
                    break;
                }
                // Line breaks inside strings are not significant.
                Some(b'\n') => {
                    self.line += 1;
                    self.pos += 1;
                }
                Some(b'\r') => self.pos += 1,
                Some(c) => {
                    raw.push(c);
                    self.pos += 1;
                }
            }
        }
        Ok(decode_string(&raw))
    }
}

/// Decodes `\\`, `\S\`, `\X\hh`, `\X2\...\X0\` and `\X4\...\X0\`. Unknown or
/// broken escapes are kept as written; `\P?\` code page switches are dropped.
fn decode_string(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let hex = |s: &[u8]| -> Option<u32> {
        let s = std::str::from_utf8(s).ok()?;
        u32::from_str_radix(s, 16).ok()
    };
    while i < raw.len() {
        let rest = &raw[i..];
        if rest.starts_with(b"\\\\") {
            out.push('\\');
            i += 2;
        } else if rest.starts_with(b"\\S\\") && rest.len() >= 4 {
            out.push(char::from(rest[3].wrapping_add(128)));
            i += 4;
        } else if rest.starts_with(b"\\X\\") && rest.len() >= 5 {
            match hex(&rest[3..5]).and_then(char::from_u32) {
                Some(c) => out.push(c),
                None => out.push('\u{FFFD}'),
            }
            i += 5;
        } else if rest.starts_with(b"\\X2\\") || rest.starts_with(b"\\X4\\") {
            let width = if rest[2] == b'2' { 4 } else { 8 };
            let body_start = 4;
            let Some(end) = find(&rest[body_start..], b"\\X0\\") else {
                out.push_str(&String::from_utf8_lossy(rest));
                break;
            };
            let body = &rest[body_start..body_start + end];
            if width == 4 {
                let units: Vec<u16> = body
                    .chunks(4)
                    .filter_map(|c| hex(c).map(|v| v as u16))
                    .collect();
                out.extend(char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')));
            } else {
                for c in body.chunks(8) {
                    out.push(hex(c).and_then(char::from_u32).unwrap_or('\u{FFFD}'));
                }
            }
            i += body_start + end + 4;
        } else if rest.starts_with(b"\\P") && rest.len() >= 4 && rest[3] == b'\\' {
            i += 4;
        } else {
            // Bytes above 0x7F are not valid Part 21, but some writers emit
            // UTF-8 or Latin-1 anyway.
            let len = utf8_len(rest);
            match std::str::from_utf8(&rest[..len]) {
                Ok(s) => out.push_str(s),
                Err(_) => out.push(char::from(rest[0])),
            }
            i += len;
        }
    }
    out
}

fn utf8_len(bytes: &[u8]) -> usize {
    let n = match bytes.first() {
        Some(b) if *b >= 0xF0 => 4,
        Some(b) if *b >= 0xE0 => 3,
        Some(b) if *b >= 0xC0 => 2,
        _ => 1,
    };
    n.min(bytes.len()).max(1)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    peeked: Option<Token>,
    limits: &'a Limits,
}

impl<'a> Parser<'a> {
    fn peek(&mut self) -> Result<Option<&Token>, ImportError> {
        if self.peeked.is_none() {
            self.peeked = self.lexer.next()?;
        }
        Ok(self.peeked.as_ref())
    }

    fn next(&mut self) -> Result<Option<Token>, ImportError> {
        match self.peeked.take() {
            Some(t) => Ok(Some(t)),
            None => self.lexer.next(),
        }
    }

    fn expect(&mut self, want: Token) -> Result<(), ImportError> {
        match self.next()? {
            Some(t) if t == want => Ok(()),
            Some(t) => Err(self.lexer.error(format!("expected {want:?}, found {t:?}"))),
            None => Err(self
                .lexer
                .error(format!("expected {want:?}, found end of file"))),
        }
    }

    fn keyword(&mut self) -> Result<String, ImportError> {
        match self.next()? {
            Some(Token::Keyword(k)) => Ok(k),
            Some(t) => Err(self.lexer.error(format!("expected a keyword, found {t:?}"))),
            None => Err(self.lexer.error("expected a keyword, found end of file")),
        }
    }

    /// `( p, p, ... )`, after the opening parenthesis has been peeked.
    fn param_list(&mut self, depth: usize) -> Result<Vec<Param>, ImportError> {
        if depth > self.limits.max_nesting {
            return Err(self.lexer.error("lists nested too deeply"));
        }
        self.expect(Token::LParen)?;
        let mut params = Vec::new();
        if self.peek()? == Some(&Token::RParen) {
            self.next()?;
            return Ok(params);
        }
        loop {
            params.push(self.param(depth)?);
            match self.next()? {
                Some(Token::Comma) => continue,
                Some(Token::RParen) => return Ok(params),
                Some(t) => {
                    return Err(self
                        .lexer
                        .error(format!("expected ',' or ')' in list, found {t:?}")))
                }
                None => return Err(self.lexer.error("unterminated list")),
            }
        }
    }

    fn param(&mut self, depth: usize) -> Result<Param, ImportError> {
        if self.peek()? == Some(&Token::LParen) {
            return Ok(Param::List(self.param_list(depth + 1)?));
        }
        Ok(match self.next()? {
            Some(Token::Int(v)) => Param::Int(v),
            Some(Token::Real(v)) => Param::Real(v),
            Some(Token::Str(s)) => Param::Str(s),
            Some(Token::Enum(e)) => Param::Enum(e),
            Some(Token::Instance(id)) => Param::Ref(id),
            Some(Token::Binary(b)) => Param::Binary(b),
            Some(Token::Dollar) => Param::Unset,
            Some(Token::Star) => Param::Derived,
            Some(Token::Keyword(name)) => {
                let mut inner = self.param_list(depth + 1)?;
                if inner.len() != 1 {
                    return Err(self
                        .lexer
                        .error(format!("typed parameter {name} needs exactly one value")));
                }
                Param::Typed(name, Box::new(inner.remove(0)))
            }
            Some(t) => return Err(self.lexer.error(format!("unexpected {t:?} in parameters"))),
            None => return Err(self.lexer.error("unexpected end of file in parameters")),
        })
    }

    fn record(&mut self) -> Result<Record, ImportError> {
        let name = self.keyword()?;
        let params = self.param_list(0)?;
        Ok(Record { name, params })
    }

    /// Skips to just past the next `;` (used to step over header records
    /// whose syntax we don't care about).
    fn header_section(&mut self) -> Result<Vec<Record>, ImportError> {
        let mut records = Vec::new();
        loop {
            match self.peek()? {
                Some(Token::Keyword(k)) if k == "ENDSEC" => {
                    self.next()?;
                    self.expect(Token::Semicolon)?;
                    return Ok(records);
                }
                Some(Token::Keyword(_)) => {
                    records.push(self.record()?);
                    self.expect(Token::Semicolon)?;
                }
                Some(t) => {
                    let t = t.clone();
                    return Err(self.lexer.error(format!("unexpected {t:?} in header")));
                }
                None => return Err(self.lexer.error("header section is not closed")),
            }
        }
    }

    fn data_section(&mut self, exchange: &mut Exchange) -> Result<(), ImportError> {
        // `DATA;` or, in edition 3, `DATA('name', ('schema'));`.
        if self.peek()? == Some(&Token::LParen) {
            self.param_list(0)?;
        }
        self.expect(Token::Semicolon)?;
        loop {
            match self.next()? {
                Some(Token::Keyword(k)) if k == "ENDSEC" => {
                    self.expect(Token::Semicolon)?;
                    return Ok(());
                }
                Some(Token::Instance(id)) => {
                    self.expect(Token::Equals)?;
                    let entity = if self.peek()? == Some(&Token::LParen) {
                        self.next()?;
                        let mut parts = Vec::new();
                        while self.peek()? != Some(&Token::RParen) {
                            if parts.len() >= self.limits.max_complex_parts {
                                return Err(self.lexer.error("complex entity has too many parts"));
                            }
                            parts.push(self.record()?);
                        }
                        self.next()?;
                        if parts.is_empty() {
                            return Err(self.lexer.error(format!("#{id}: empty complex entity")));
                        }
                        Entity::Complex(parts)
                    } else {
                        Entity::Simple(self.record()?)
                    };
                    self.expect(Token::Semicolon)?;
                    if exchange.entities.len() >= self.limits.max_entities {
                        return Err(ImportError::TooLarge(format!(
                            "more than {} entity instances",
                            self.limits.max_entities
                        )));
                    }
                    if exchange.entities.insert(id, entity).is_some() {
                        return Err(self.lexer.error(format!("#{id} is defined twice")));
                    }
                }
                Some(t) => {
                    return Err(self
                        .lexer
                        .error(format!("expected an instance or ENDSEC, found {t:?}")))
                }
                None => return Err(self.lexer.error("data section is not closed")),
            }
        }
    }
}

/// Parses a whole exchange file.
pub fn parse(text: &[u8], limits: &Limits) -> Result<Exchange, ImportError> {
    if text.len() > limits.max_input_bytes {
        return Err(ImportError::TooLarge(format!(
            "file is {} bytes, the limit is {}",
            text.len(),
            limits.max_input_bytes
        )));
    }
    let mut p = Parser {
        lexer: Lexer {
            src: text,
            pos: 0,
            line: 1,
        },
        peeked: None,
        limits,
    };
    // Skip a UTF-8 byte order mark, which some Windows tools write.
    if text.starts_with(&[0xEF, 0xBB, 0xBF]) {
        p.lexer.pos = 3;
    }
    match p.next()? {
        Some(Token::Keyword(k)) if k == "ISO-10303-21" => {}
        _ => return Err(ImportError::NotStep),
    }
    p.expect(Token::Semicolon)?;
    let mut exchange = Exchange::default();
    let mut saw_data = false;
    loop {
        match p.next()? {
            Some(Token::Keyword(k)) if k == "HEADER" => {
                p.expect(Token::Semicolon)?;
                exchange.header.extend(p.header_section()?);
            }
            Some(Token::Keyword(k)) if k == "DATA" => {
                p.data_section(&mut exchange)?;
                saw_data = true;
            }
            // Edition 3 sections we don't use.
            Some(Token::Keyword(k)) if k == "ANCHOR" || k == "REFERENCE" || k == "SIGNATURE" => {
                p.expect(Token::Semicolon)?;
                skip_to_endsec(&mut p)?;
            }
            Some(Token::Keyword(k)) if k == "END-ISO-10303-21" => {
                p.expect(Token::Semicolon)?;
                break;
            }
            Some(t) => return Err(p.lexer.error(format!("unexpected {t:?} between sections"))),
            None => return Err(p.lexer.error("missing END-ISO-10303-21")),
        }
    }
    if !saw_data {
        return Err(ImportError::Syntax {
            line: p.lexer.line,
            message: "no DATA section".into(),
        });
    }
    Ok(exchange)
}

fn skip_to_endsec(p: &mut Parser) -> Result<(), ImportError> {
    loop {
        match p.next()? {
            Some(Token::Keyword(k)) if k == "ENDSEC" => {
                p.expect(Token::Semicolon)?;
                return Ok(());
            }
            Some(_) => {}
            None => return Err(p.lexer.error("section is not closed")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(s: &str) -> Result<Exchange, ImportError> {
        parse(s.as_bytes(), &Limits::default())
    }

    const MINIMAL: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
        FILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF { 1 0 10303 442 1 1 4 }'));\n\
        ENDSEC;\nDATA;\n";

    #[test]
    fn parses_simple_and_complex_instances() {
        let text = format!(
            "{MINIMAL}#1=CARTESIAN_POINT('p',(0.,1.5E1,-2));\n\
             /* comment */ #2 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
             #3=MEASURE(LENGTH_MEASURE(25.4),#2,$,\"0A1\");\nENDSEC;\nEND-ISO-10303-21;\n"
        );
        let ex = parse_str(&text).unwrap();
        assert_eq!(
            ex.schemas(),
            vec!["AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF { 1 0 10303 442 1 1 4 }"]
        );
        let p = ex.entities[&1].record("CARTESIAN_POINT").unwrap();
        assert_eq!(
            p.params[1],
            Param::List(vec![Param::Real(0.0), Param::Real(15.0), Param::Int(-2)])
        );
        let unit = &ex.entities[&2];
        assert!(unit.has("LENGTH_UNIT") && unit.has("SI_UNIT"));
        assert_eq!(
            unit.record("SI_UNIT").unwrap().params,
            vec![Param::Enum("MILLI".into()), Param::Enum("METRE".into())]
        );
        let m = ex.entities[&3].record("MEASURE").unwrap();
        assert_eq!(
            m.params[0],
            Param::Typed("LENGTH_MEASURE".into(), Box::new(Param::Real(25.4)))
        );
        assert_eq!(m.params[2], Param::Unset);
        assert_eq!(m.params[3], Param::Binary("0A1".into()));
    }

    #[test]
    fn decodes_string_escapes() {
        assert_eq!(decode_string(b"Bob's \\X2\\00F8\\X0\\"), "Bob's ø");
        assert_eq!(decode_string(b"a\\\\b\\X\\E9"), "a\\bé");
        assert_eq!(decode_string(b"\\X4\\0001F600\\X0\\"), "😀");
        assert_eq!(decode_string(b"\\S\\i"), "é");
        assert_eq!(decode_string(b"broken \\X2\\00"), "broken \\X2\\00");
        let text = format!(
            "{MINIMAL}#1=PRODUCT('it''s','\\X2\\00F8\\X0\\','',());ENDSEC;END-ISO-10303-21;"
        );
        let ex = parse_str(&text).unwrap();
        let r = ex.entities[&1].record("PRODUCT").unwrap();
        assert_eq!(r.params[0], Param::Str("it's".into()));
        assert_eq!(r.params[1], Param::Str("ø".into()));
    }

    #[test]
    fn rejects_malformed_input_without_panicking() {
        let cases = [
            "",
            "garbage",
            "ISO-10303-21;",
            "ISO-10303-21;HEADER;ENDSEC;END-ISO-10303-21;",
            &format!("{MINIMAL}#1=A((((;"),
            &format!("{MINIMAL}#1=A('unterminated"),
            &format!("{MINIMAL}#1=A(1.5E);ENDSEC;END-ISO-10303-21;"),
            &format!("{MINIMAL}#1=A(1);#1=B(2);ENDSEC;END-ISO-10303-21;"),
            &format!("{MINIMAL}#99999999999999999999999=A();ENDSEC;END-ISO-10303-21;"),
            &format!("{MINIMAL}#1=();ENDSEC;END-ISO-10303-21;"),
            &format!("{MINIMAL}#1=A(1e999);ENDSEC;END-ISO-10303-21;"),
            &format!("{MINIMAL}#1=A(/* never closed"),
            &format!("{MINIMAL}#1=A(.T);ENDSEC;END-ISO-10303-21;"),
            &format!("{MINIMAL}#1=A(\u{1F600});ENDSEC;END-ISO-10303-21;"),
        ];
        for case in cases {
            assert!(parse_str(case).is_err(), "accepted {case:?}");
        }
    }

    #[test]
    fn limits_nesting_and_size() {
        let deep = format!(
            "{MINIMAL}#1=A({}1{});ENDSEC;END-ISO-10303-21;",
            "(".repeat(100_000),
            ")".repeat(100_000)
        );
        assert!(matches!(parse_str(&deep), Err(ImportError::Syntax { .. })));
        let limits = Limits {
            max_entities: 2,
            ..Limits::default()
        };
        let many = format!("{MINIMAL}#1=A();#2=A();#3=A();ENDSEC;END-ISO-10303-21;");
        assert!(matches!(
            parse(many.as_bytes(), &limits),
            Err(ImportError::TooLarge(_))
        ));
    }

    #[test]
    fn fuzzed_prefixes_and_byte_flips_never_panic() {
        let text = format!(
            "{MINIMAL}#1=CARTESIAN_POINT('p',(0.,1.5E1,-2));\n\
             #2=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\n\
             #3=PRODUCT('a''b','\\X2\\00F8\\X0\\',$,(#1,#2));ENDSEC;END-ISO-10303-21;\n"
        );
        let bytes = text.as_bytes();
        for cut in 0..bytes.len() {
            let _ = parse(&bytes[..cut], &Limits::default());
        }
        let mut state = 0x2545_f491_u32;
        for _ in 0..5_000 {
            let mut copy = bytes.to_vec();
            for _ in 0..3 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let i = state as usize % copy.len();
                copy[i] = (state >> 8) as u8;
            }
            let _ = parse(&copy, &Limits::default());
        }
    }
}
