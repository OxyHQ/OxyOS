//! JSON as the package's documents are written: RFC 8259, read strictly, and RFC 8785's
//! canonical form (JCS), which is what is signed.
//!
//! The reader keeps an object's members in the order written and refuses a key written twice
//! (serde_json keeps the last one, so two readers of one document could disagree on what it
//! says). Its value space is the documents': objects, arrays, strings, booleans and integers
//! from 0 to 2^53 − 1, which JCS writes as ECMAScript does and which survive any reader exactly.
//! `null`, negative numbers, fractions and exponents are refused.

use std::fmt::Write as _;

/// The deepest nesting a document may have.
pub const MAX_DEPTH: usize = 16;
/// The largest integer: 2^53 − 1, the largest every JSON reader holds exactly.
pub const MAX_INTEGER: u64 = (1 << 53) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Object(Vec<(String, Value)>),
    Array(Vec<Value>),
    String(String),
    Integer(u64),
    Bool(bool),
}

/// Why a document is not JSON this crate reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonError {
    /// Not JSON, or JSON outside the value space; the byte offset where it failed.
    Syntax(usize),
    /// A key written twice in one object.
    DuplicateKey(String),
    /// Nested deeper than [`MAX_DEPTH`].
    TooDeep,
}

pub fn parse(bytes: &[u8]) -> Result<Value, JsonError> {
    let mut reader = Reader { bytes, at: 0 };
    reader.whitespace();
    let value = reader.value(0)?;
    reader.whitespace();
    if reader.at != bytes.len() {
        return Err(JsonError::Syntax(reader.at));
    }
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn error<T>(&self) -> Result<T, JsonError> {
        Err(JsonError::Syntax(self.at))
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), JsonError> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            self.error()
        }
    }

    fn literal(&mut self, word: &[u8]) -> Result<(), JsonError> {
        if self.bytes[self.at..].starts_with(word) {
            self.at += word.len();
            Ok(())
        } else {
            self.error()
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        match self.peek() {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.literal(b"true").map(|()| Value::Bool(true)),
            Some(b'f') => self.literal(b"false").map(|()| Value::Bool(false)),
            Some(b'0'..=b'9') => self.integer(),
            _ => self.error(),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError::TooDeep);
        }
        self.expect(b'{')?;
        let mut members: Vec<(String, Value)> = Vec::new();
        self.whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.whitespace();
            if self.peek() != Some(b'"') {
                return self.error();
            }
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                return Err(JsonError::DuplicateKey(key));
            }
            self.whitespace();
            self.expect(b':')?;
            self.whitespace();
            let value = self.value(depth)?;
            members.push((key, value));
            self.whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Object(members));
                }
                _ => return self.error(),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError::TooDeep);
        }
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.whitespace();
            items.push(self.value(depth)?);
            self.whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                _ => return self.error(),
            }
        }
    }

    fn integer(&mut self) -> Result<Value, JsonError> {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        // A fraction or an exponent is a number this value space does not have.
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return self.error();
        }
        let digits = &self.bytes[start..self.at];
        if digits.len() > 1 && digits[0] == b'0' {
            return Err(JsonError::Syntax(start));
        }
        let mut value: u64 = 0;
        for &digit in digits {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(u64::from(digit - b'0')))
                .filter(|v| *v <= MAX_INTEGER)
                .ok_or(JsonError::Syntax(start))?;
        }
        Ok(Value::Integer(value))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = match self.peek() {
                Some(b @ b'0'..=b'9') => b - b'0',
                Some(b @ b'a'..=b'f') => b - b'a' + 10,
                Some(b @ b'A'..=b'F') => b - b'A' + 10,
                _ => return self.error(),
            };
            value = value * 16 + u32::from(digit);
            self.at += 1;
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let start = self.at;
            // Unescaped bytes up to the next quote, backslash or control character.
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            match std::str::from_utf8(&self.bytes[start..self.at]) {
                Ok(text) => out.push_str(text),
                Err(e) => return Err(JsonError::Syntax(start + e.valid_up_to())),
            }
            match self.peek() {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escaped = self.peek();
                    self.at += 1;
                    let c = match escaped {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            let first = self.hex4()?;
                            let code = match first {
                                0xD800..=0xDBFF => {
                                    self.literal(b"\\u")?;
                                    let second = self.hex4()?;
                                    if !(0xDC00..=0xDFFF).contains(&second) {
                                        return self.error();
                                    }
                                    0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                                }
                                // A low surrogate alone is no character.
                                0xDC00..=0xDFFF => return self.error(),
                                code => code,
                            };
                            match char::from_u32(code) {
                                Some(c) => c,
                                None => return self.error(),
                            }
                        }
                        _ => return Err(JsonError::Syntax(self.at - 1)),
                    };
                    out.push(c);
                }
                // The end of the input, or a control character JSON requires escaped.
                _ => return self.error(),
            }
        }
    }
}

/// The value in RFC 8785's canonical form: no whitespace, members sorted by their keys' UTF-16
/// code units, strings escaped as ECMAScript's `JSON.stringify` escapes them.
pub fn canonical(value: &Value) -> Vec<u8> {
    let mut out = String::new();
    write_value(&mut out, value);
    out.into_bytes()
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Object(members) => {
            let mut sorted: Vec<&(String, Value)> = members.iter().collect();
            sorted.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, (key, value)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_value(out, value);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::String(text) => write_string(out, text),
        Value::Integer(n) => {
            let _ = write!(out, "{n}");
        }
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Builds an object from members in any order; [`canonical`] sorts them.
pub fn object(members: Vec<(&str, Value)>) -> Value {
    Value::Object(
        members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

pub fn string(text: &str) -> Value {
    Value::String(text.to_string())
}
