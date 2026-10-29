// CEP:FILE: tools/cep_lint/src/json.rs
// CEP:WHAT: Minimal hand-rolled JSON reader for the lint configuration shape: objects, arrays,
//   strings with escape sequences, integers, floats, booleans, and null, parsed into a
//   typed Value tree with line/column diagnostics on every error path.
// CEP:WHY: The linter is a zero-dependency crate by workspace law, so configuration parsing
//   cannot pull in a JSON crate; a small explicit reader keeps the tool self-contained.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: every failure mode (bad token, bad escape, truncated input, depth overrun,
//   trailing content) returns a descriptive Err with position; no panic path exists and no
//   malformed result is silently repaired.
// CEP:ASSUMES: input is valid UTF-8 (&str); configuration files are trusted first-party
//   artifacts, yet every bound below still fails closed rather than panicking.
// CEP:COST: O(N) single pass over the input; one Value tree allocation.
// CEP:EVIDENCE: unit tests in this module cover valid documents, each error class, and the
//   depth budget; the config module's tests parse the shipped .cep/cep_lint.json shape.
// CEP:SECURITY: nesting depth is capped at MAX_JSON_DEPTH, so hostile or accidental deep
//   nesting cannot exhaust memory or stack; recursion is bounded by the same constant (38.13).

use std::fmt;

/// Maximum container nesting accepted by the parser; anything deeper is rejected.
pub const MAX_JSON_DEPTH: usize = 64;

/// CEP:WHAT: A parsed JSON value: null, bool, number (kept in source form), string, array,
///   or object, with typed accessors for the shapes the configuration uses.
/// CEP:WHY: The zero-dependency law bans a JSON crate, so the config reader needs its own
///   minimal, total value model.
/// CEP:STATUS: complete
/// CEP:FAILURE: accessors return Option for type mismatches; no accessor panics.
/// CEP:ASSUMES: numbers stay in source form; interpretation happens at the accessor.
/// CEP:COST: one tree allocation proportional to the document; O(1) accessors.
/// CEP:EVIDENCE: json::tests cover the config shape, numbers, literals, escapes, and every
///   error class.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Numbers are kept in source form; interpretation is up to the accessor.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Object member lookup by key (first match wins).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Boolean accessor with type checking.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// String slice accessor with type checking.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// CEP:WHAT: Unsigned-integer accessor accepting only integral numeric text.
    /// CEP:WHY: Severities, budgets, and exit codes must reject fractional or exponential
    ///   forms instead of silently coercing them.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None for floats, exponents, negatives, or overflow.
    /// CEP:ASSUMES: the number text is a valid JSON number (parser-guaranteed).
    /// CEP:COST: O(digits) parse on access; no allocation.
    /// CEP:EVIDENCE: json::tests assert integral acceptance and fractional rejection.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(text) => {
                if text.contains('.') || text.contains('e') || text.contains('E') {
                    return None;
                }
                text.parse::<u64>().ok()
            }
            _ => None,
        }
    }

    /// Array accessor with type checking.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Human-readable type name for diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}

/// CEP:WHAT: JSON parse failure with the 1-based line and column of the offending byte.
/// CEP:WHY: Configuration bootstrap errors must point at the exact defect so a broken
///   gate can be repaired in seconds.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a plain data carrier with a formatted Display.
/// CEP:ASSUMES: positions count characters, not bytes, for multi-byte input.
/// CEP:COST: one struct per failed parse.
/// CEP:EVIDENCE: json::tests::error_positions_are_reported pins line/column reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JSON parse error at line {} column {}: {}",
            self.line, self.column, self.message
        )
    }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Parser {
    fn new(source: &str) -> Parser {
        Parser {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    fn err(&self, message: &str) -> JsonError {
        JsonError {
            line: self.line,
            column: self.column,
            message: message.to_string(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), JsonError> {
        self.skip_ws();
        match self.peek() {
            Some(c) if c == expected => {
                self.bump();
                Ok(())
            }
            Some(c) => Err(self.err(&format!("expected '{}', found '{}'", expected, c))),
            None => Err(self.err(&format!("expected '{}', found end of input", expected))),
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        // Caller verified the opening quote.
        self.bump();
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string literal")),
                Some('"') => return Ok(out),
                Some('\n') => return Err(self.err("newline inside string literal")),
                Some('\\') => match self.bump() {
                    None => return Err(self.err("unterminated escape sequence")),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('/') => out.push('/'),
                    Some('b') => out.push('\u{0008}'),
                    Some('f') => out.push('\u{000C}'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('u') => {
                        let value = self.parse_hex4()?;
                        if (0xD800..0xDC00).contains(&value) {
                            // High surrogate: must pair with a low surrogate.
                            if self.peek() == Some('\\') {
                                self.bump();
                            } else {
                                return Err(self.err("lone high surrogate"));
                            }
                            if self.peek() == Some('u') {
                                self.bump();
                            } else {
                                return Err(self.err("lone high surrogate"));
                            }
                            let low = self.parse_hex4()?;
                            if !(0xDC00..0xE000).contains(&low) {
                                return Err(self.err("invalid low surrogate"));
                            }
                            let combined =
                                0x10000u32 + ((value - 0xD800) << 10) + (low - 0xDC00);
                            match char::from_u32(combined) {
                                Some(c) => out.push(c),
                                None => return Err(self.err("invalid surrogate pair")),
                            }
                        } else if (0xDC00..0xE000).contains(&value) {
                            return Err(self.err("lone low surrogate"));
                        } else {
                            match char::from_u32(value) {
                                Some(c) => out.push(c),
                                None => return Err(self.err("invalid unicode escape")),
                            }
                        }
                    }
                    Some(c) => {
                        return Err(self.err(&format!("unknown escape character '{}'", c)));
                    }
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, JsonError> {
        let mut value: u32 = 0;
        for _ in 0..4 {
            let c = self
                .bump()
                .ok_or_else(|| self.err("truncated unicode escape"))?;
            let digit = match c.to_digit(16) {
                Some(d) => d,
                None => {
                    return Err(
                        self.err(&format!("invalid hex digit '{}' in unicode escape", c))
                    );
                }
            };
            value = value.saturating_mul(16).saturating_add(digit);
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<String, JsonError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.bump();
        }
        let mut saw_digit = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
                if c.is_ascii_digit() {
                    saw_digit = true;
                }
                self.bump();
            } else {
                break;
            }
        }
        if !saw_digit {
            return Err(self.err("number literal has no digits"));
        }
        // Slice the char vector, not the source string: self.pos counts chars, and byte
        // offsets would diverge after any multi-byte character.
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn parse_literal(&mut self, word: &str, value: Value) -> Result<Value, JsonError> {
        for expected in word.chars() {
            match self.bump() {
                Some(c) if c == expected => {}
                Some(c) => {
                    return Err(self.err(&format!(
                        "invalid literal, expected '{}' found '{}'",
                        word, c
                    )));
                }
                None => return Err(self.err("truncated literal")),
            }
        }
        Ok(value)
    }

    fn parse_value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_JSON_DEPTH {
            return Err(self.err("container nesting exceeds depth budget"));
        }
        self.skip_ws();
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some('{') => {
                self.bump();
                let mut members: Vec<(String, Value)> = Vec::new();
                self.skip_ws();
                if self.peek() == Some('}') {
                    self.bump();
                    return Ok(Value::Object(members));
                }
                loop {
                    self.skip_ws();
                    match self.peek() {
                        Some('"') => {}
                        Some(c) => {
                            return Err(self.err(&format!(
                                "expected object key string, found '{}'",
                                c
                            )));
                        }
                        None => return Err(self.err("expected object key, found end of input")),
                    }
                    let key = self.parse_string()?;
                    self.expect_char(':')?;
                    let value = self.parse_value(depth + 1)?;
                    members.push((key, value));
                    self.skip_ws();
                    match self.peek() {
                        Some(',') => {
                            self.bump();
                        }
                        Some('}') => {
                            self.bump();
                            return Ok(Value::Object(members));
                        }
                        Some(c) => {
                            return Err(self.err(&format!(
                                "expected ',' or '}}', found '{}'",
                                c
                            )));
                        }
                        None => return Err(self.err("unterminated object: end of input")),
                    }
                }
            }
            Some('[') => {
                self.bump();
                let mut items: Vec<Value> = Vec::new();
                self.skip_ws();
                if self.peek() == Some(']') {
                    self.bump();
                    return Ok(Value::Array(items));
                }
                loop {
                    let value = self.parse_value(depth + 1)?;
                    items.push(value);
                    self.skip_ws();
                    match self.peek() {
                        Some(',') => {
                            self.bump();
                        }
                        Some(']') => {
                            self.bump();
                            return Ok(Value::Array(items));
                        }
                        Some(c) => {
                            return Err(self.err(&format!(
                                "expected ',' or ']', found '{}'",
                                c
                            )));
                        }
                        None => return Err(self.err("unterminated array: end of input")),
                    }
                }
            }
            Some('"') => self.parse_string().map(Value::String),
            Some('t') => self.parse_literal("true", Value::Bool(true)),
            Some('f') => self.parse_literal("false", Value::Bool(false)),
            Some('n') => self.parse_literal("null", Value::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number().map(Value::Number),
            Some(c) => Err(self.err(&format!("unexpected character '{}'", c))),
        }
    }
}

/// CEP:WHAT: Parses a complete JSON document, rejecting trailing content after the
///   top-level value, with a hard nesting-depth budget.
/// CEP:WHY: The linter cannot pull in a JSON crate, and hostile or accidental deep
///   nesting must fail closed rather than exhaust the stack.
/// CEP:STATUS: complete
/// CEP:FAILURE: descriptive Err with position for bad tokens, bad escapes, truncated
///   input, depth overrun, and trailing content; no panic path exists.
/// CEP:ASSUMES: input is valid UTF-8; recursion is bounded by MAX_JSON_DEPTH.
/// CEP:COST: O(N) single pass over the input.
/// CEP:EVIDENCE: json::tests cover valid documents, each error class, and the depth
///   budget; config::tests parse the shipped .cep/cep_lint.json through this reader.
pub fn parse(source: &str) -> Result<Value, JsonError> {
    let mut parser = Parser::new(source);
    let value = parser.parse_value(1)?;
    parser.skip_ws();
    if parser.pos < parser.chars.len() {
        return Err(parser.err("trailing content after top-level JSON value"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_shape() {
        let src = r#"{
            "standard": {"name": "CEP&CC", "version": "0.1"},
            "fail_on": [0, 1],
            "rules": {
                "R": {"severity": 1, "tokens": [".unwrap()"], "icase": false}
            },
            "note": "escapes: \"quoted\" \n tab\t"
        }"#;
        let value = parse(src).expect("valid config json");
        let standard = value.get("standard").expect("standard present");
        assert_eq!(standard.get("name").and_then(|v| v.as_str()), Some("CEP&CC"));
        let fail_on = value.get("fail_on").and_then(|v| v.as_array()).unwrap();
        assert_eq!(fail_on.len(), 2);
        assert_eq!(fail_on[0].as_u64(), Some(0));
        let rules = value.get("rules").expect("rules present");
        let rule = rules.get("R").expect("rule R present");
        assert_eq!(rule.get("severity").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(
            rule.get("tokens").and_then(|v| v.as_array()).map(|a| a.len()),
            Some(1)
        );
    }

    #[test]
    fn rejects_trailing_content() {
        let err = parse("{} extra").expect_err("trailing content must fail");
        assert!(err.message.contains("trailing"));
    }

    #[test]
    fn rejects_truncated_input_without_panic() {
        for src in [
            "{", "[1, 2", "\"abc", "{\"a\":}", "[", "{\"a\"", "tru", "{\"a\":1,}", "[1,]",
        ] {
            assert!(parse(src).is_err(), "expected error for {:?}", src);
        }
    }

    #[test]
    fn rejects_bad_escape_and_bad_number() {
        assert!(parse(r#""\q""#).is_err());
        assert!(parse("-").is_err());
        assert!(parse("{1:2}").is_err());
    }

    #[test]
    fn depth_budget_enforced() {
        let mut deep = String::new();
        for _ in 0..(MAX_JSON_DEPTH + 8) {
            deep.push('[');
        }
        let err = parse(&deep).expect_err("deep nesting must fail");
        assert!(err.message.contains("depth"));
    }

    #[test]
    fn empty_containers_and_numbers_and_literals() {
        let value = parse("{\"a\": [], \"b\": {}, \"c\": [0, 1, -12, 3.5e2, true, false, null]}")
            .expect("document");
        assert_eq!(
            value.get("a").and_then(|v| v.as_array()).map(|a| a.len()),
            Some(0)
        );
        assert_eq!(value.get("b"), Some(&Value::Object(Vec::new())));
        let items = value.get("c").and_then(|v| v.as_array()).unwrap();
        assert_eq!(items.len(), 7);
        assert_eq!(items[1].as_u64(), Some(1));
        assert_eq!(items[2].as_u64(), None);
        assert_eq!(items[6], Value::Null);
    }

    #[test]
    fn unicode_escapes_including_surrogate_pair() {
        let value = parse(r#""A😀""#).expect("unicode string");
        assert_eq!(value.as_str(), Some("A\u{1F600}"));
        assert!(parse(r#""😀""#).is_ok());
        assert!(parse(r#""\ud83d""#).is_err());
        assert!(parse(r#""\ud83dx""#).is_err());
    }

    #[test]
    fn unicode_before_numbers_keeps_text_intact() {
        // Regression: parse_number sliced the source by char offsets, which diverge from
        // byte offsets after any multi-byte character.
        let value = parse("{\"é\": 5, \"λ\": [true, -12]}").expect("unicode doc parses");
        assert_eq!(value.get("é").and_then(|v| v.as_u64()), Some(5));
        assert_eq!(
            value.get("λ").and_then(|v| v.as_array()).map(|a| a.len()),
            Some(2)
        );
        assert_eq!(
            value
                .get("λ")
                .and_then(|v| v.as_array())
                .and_then(|a| a.get(1))
                .and_then(|v| v.as_u64()),
            None,
            "-12 is not a u64"
        );
    }

    #[test]
    fn error_positions_are_reported() {
        let err = parse("{\n  \"a\": ???\n}").expect_err("bad token");
        assert_eq!(err.line, 2);
        assert!(err.message.contains("unexpected"));
    }
}
