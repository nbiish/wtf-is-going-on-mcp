//! Minimal JSON (RFC 8259): value model, strict parser, serializer.
//!
//! Written in-tree (zero-dependency constraint). Parser is recursive descent
//! with a hard depth limit so hostile input cannot blow the stack. Object
//! key order is preserved; duplicate keys resolve last-wins on lookup.

use std::fmt;

pub const MAX_DEPTH: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

#[derive(Debug, Clone)]
pub struct Error {
    pub msg: String,
    pub pos: usize,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "json error at byte {}: {}", self.pos, self.msg)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

impl Value {
    // ---------- constructors ----------

    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn num(f: f64) -> Value {
        Value::Num(f)
    }

    pub fn obj(pairs: Vec<(&str, Value)>) -> Value {
        Value::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn arr(items: Vec<Value>) -> Value {
        Value::Arr(items)
    }

    // ---------- accessors ----------

    /// Last-wins lookup (duplicate keys allowed in the vector).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(o) => o.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 => {
                Some(*n as i64)
            }
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Serialize to a JSON string.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Num(f) => write_num(*f, out),
            Value::Str(s) => write_str(s, out),
            Value::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Value::Obj(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_json())
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(s)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Value {
        Value::Num(n as f64)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Value {
        Value::Num(n)
    }
}

fn write_num(f: f64, out: &mut String) {
    if !f.is_finite() {
        // JSON has no NaN/Inf; degrade to null rather than emitting invalid JSON.
        out.push_str("null");
        return;
    }
    if f.fract() == 0.0 && f.abs() < 9.007_199_254_740_992e15 {
        out.push_str(&format!("{}", f as i64));
    } else {
        out.push_str(&format!("{f}"));
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------- parser ----------

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

/// Parse a complete JSON document. Trailing garbage is an error.
pub fn parse(input: &str) -> Result<Value> {
    let mut p = Parser { s: input.as_bytes(), pos: 0 };
    p.skip_ws();
    let v = p.value(0)?;
    p.skip_ws();
    if p.pos != p.s.len() {
        return Err(p.err("trailing characters after JSON value"));
    }
    Ok(v)
}

impl<'a> Parser<'a> {
    fn err(&self, msg: &str) -> Error {
        Error { msg: msg.to_string(), pos: self.pos }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, lit: &[u8], val: Value) -> Result<Value> {
        if self.s.len() >= self.pos + lit.len() && &self.s[self.pos..self.pos + lit.len()] == lit {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.expect(b"true", Value::Bool(true)),
            Some(b'f') => self.expect(b"false", Value::Bool(false)),
            Some(b'n') => self.expect(b"null", Value::Null),
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value> {
        self.bump(); // '{'
        let mut pairs: Vec<(String, Value)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(Value::Obj(pairs));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected object key string"));
            }
            let key = self.string()?;
            self.skip_ws();
            if self.bump() != Some(b':') {
                return Err(self.err("expected ':' after object key"));
            }
            self.skip_ws();
            let val = self.value(depth + 1)?;
            pairs.push((key, val));
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(Value::Obj(pairs)),
                _ => return Err(self.err("expected ',' or '}' in object")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value> {
        self.bump(); // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.bump();
            return Ok(Value::Arr(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(Value::Arr(items)),
                _ => return Err(self.err("expected ',' or ']' in array")),
            }
        }
    }

    fn string(&mut self) -> Result<String> {
        self.bump(); // opening '"'
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.bump().ok_or_else(|| self.err("unterminated string"))?;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.bump().ok_or_else(|| self.err("unterminated escape"))?;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hi = self.u16_hex()?;
                            let cp = if (0xd800..=0xdbff).contains(&hi) {
                                if self.bump() != Some(b'\\') || self.bump() != Some(b'u') {
                                    return Err(self.err("lone high surrogate"));
                                }
                                let lo = self.u16_hex()?;
                                if !(0xdc00..=0xdfff).contains(&lo) {
                                    return Err(self.err("invalid low surrogate"));
                                }
                                0x1_0000 + ((hi as u32 - 0xd800) << 10) + (lo as u32 - 0xdc00)
                            } else if (0xdc00..=0xdfff).contains(&hi) {
                                return Err(self.err("lone low surrogate"));
                            } else {
                                hi as u32
                            };
                            let ch = char::from_u32(cp).ok_or_else(|| self.err("invalid codepoint"))?;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                c if c < 0x20 => return Err(self.err("raw control character in string")),
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("invalid utf-8 in string"))
    }

    fn u16_hex(&mut self) -> Result<u16> {
        let mut v: u16 = 0;
        for _ in 0..4 {
            let c = self.bump().ok_or_else(|| self.err("truncated \\u escape"))?;
            let d = (c as char)
                .to_digit(16)
                .ok_or_else(|| self.err("invalid hex in \\u escape"))?;
            v = v
                .checked_mul(16)
                .and_then(|x| x.checked_add(d as u16))
                .ok_or_else(|| self.err("hex overflow"))?;
        }
        Ok(v)
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.bump();
        }
        match self.peek() {
            Some(b'0') => {
                self.bump();
                if self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    return Err(self.err("leading zeros not allowed"));
                }
            }
            Some(b'1'..=b'9') => {
                self.bump();
                while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    self.bump();
                }
            }
            _ => return Err(self.err("invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.bump();
            if !self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                return Err(self.err("digit required after decimal point"));
            }
            while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.bump();
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.bump();
            }
            if !self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                return Err(self.err("digit required in exponent"));
            }
            while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                self.bump();
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.pos]).unwrap();
        let f: f64 = text.parse().map_err(|_| self.err("number out of range"))?;
        if !f.is_finite() {
            return Err(self.err("number out of range"));
        }
        Ok(Value::Num(f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Value {
        parse(s).unwrap()
    }

    #[test]
    fn scalars() {
        assert_eq!(p("null"), Value::Null);
        assert_eq!(p(" true "), Value::Bool(true));
        assert_eq!(p("false"), Value::Bool(false));
        assert_eq!(p("42"), Value::Num(42.0));
        assert_eq!(p("-0"), Value::Num(0.0));
        assert_eq!(p("1e10").as_f64(), Some(1e10));
        assert_eq!(p("-1.5e-3").as_f64(), Some(-0.0015));
        assert_eq!(p("\"hi\""), Value::Str("hi".into()));
    }

    #[test]
    fn containers() {
        let v = p(r#"{"a":1,"b":[true,null,"x"],"c":{"d":2}}"#);
        assert_eq!(v.get("a").and_then(|x| x.as_i64()), Some(1));
        assert_eq!(v.get("b").unwrap().as_arr().unwrap().len(), 3);
        assert_eq!(v.get("c").and_then(|c| c.get("d")).and_then(|d| d.as_i64()), Some(2));
        assert_eq!(p("[]"), Value::Arr(vec![]));
        assert_eq!(p("{}"), Value::Obj(vec![]));
        assert_eq!(p("[[[]]]"), p("[[[]]]"));
    }

    #[test]
    fn string_escapes() {
        assert_eq!(p(r#""a\nb""#), Value::Str("a\nb".into()));
        assert_eq!(p(r#""\u0041""#), Value::Str("A".into()));
        assert_eq!(p(r#""\ud83d\ude00""#), Value::Str("😀".into()));
        assert_eq!(p(r#""\\\/\"""#), Value::Str("\\/\"".into()));
        assert_eq!(p("\"\u{7f}\""), Value::Str("\u{7f}".into()));
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "", "  ", "{", "}", "[", "[1,", "{\"a\"}", "{\"a\":}", "{\"a\":1,}",
            "[1,]", "01", "+1", ".5", "1.", "-", "1e", "1e+", "\"", "\"\\q\"",
            "\"\\ud83d\"", "\"\\ude00\"", "\"\u{01}\"", "tru", "nul", "nulls",
            "{\"a\":1} extra", "1 2", "{\"a\" 1}", "'x'", "NaN", "Infinity",
        ] {
            assert!(parse(bad).is_err(), "should reject: {bad:?}");
        }
    }

    #[test]
    fn depth_limit() {
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert!(parse(&deep).is_err());
        let ok = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn dup_keys_last_wins() {
        let v = p(r#"{"a":1,"a":2}"#);
        assert_eq!(v.get("a").and_then(|x| x.as_i64()), Some(2));
    }

    #[test]
    fn serialize_roundtrip() {
        let samples = [
            "null",
            "true",
            "-12.5",
            "1e300",
            r#""plain""#,
            r#""esc \" \\ \/ \b \f \n \r \t \u0001""#,
            "\"unicode 😀 áé ok\"",
            r#"[1,"two",null,false]"#,
            r#"{"k":"v","n":[1,2,{"deep":true}]}"#,
        ];
        for s in samples {
            let v = p(s);
            let out = v.to_json();
            assert_eq!(parse(&out).unwrap(), v, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn serializer_escapes_and_numbers() {
        assert_eq!(Value::str("a\"b\\c\n").to_json(), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(Value::str("\u{01}").to_json(), "\"\\u0001\"");
        assert_eq!(Value::Num(7.0).to_json(), "7");
        assert_eq!(Value::Num(-0.0).to_json(), "0");
        assert_eq!(Value::Num(1.5).to_json(), "1.5");
        assert_eq!(Value::Num(f64::NAN).to_json(), "null");
        assert_eq!(Value::Num(f64::INFINITY).to_json(), "null");
    }

    #[test]
    fn accessors_safe() {
        let v = p("[1,2]");
        assert!(v.get("x").is_none());
        assert!(v.as_str().is_none());
        assert_eq!(Value::Null.as_i64(), None);
        assert!(parse("1e400").is_err()); // out-of-range numbers fail closed
    }
}
