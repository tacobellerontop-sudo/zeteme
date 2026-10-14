//! Just enough JSON to read the Bitwarden CLI's output, so zeteme carries no
//! JSON crate.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Numbers are kept as written; zeteme never does arithmetic on them.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Object(fields) => fields
                .iter()
                .find(|(k, _)| k == key)
                .map_or(&Value::Null, |(_, v)| v),
            _ => &Value::Null,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> &[Value] {
        match self {
            Value::Array(items) => items,
            _ => &[],
        }
    }
}

pub fn parse(text: &str) -> Option<Value> {
    let mut parser = Parser {
        chars: text.chars().peekable(),
    };
    let value = parser.value()?;
    parser.skip_space();
    parser.chars.peek().is_none().then_some(value)
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.chars.next_if(|c| c.is_whitespace()).is_some() {}
    }

    fn eat(&mut self, c: char) -> Option<()> {
        self.skip_space();
        self.chars.next_if_eq(&c).map(|_| ())
    }

    fn word(&mut self, word: &str, value: Value) -> Option<Value> {
        for c in word.chars() {
            self.chars.next_if_eq(&c)?;
        }
        Some(value)
    }

    fn value(&mut self) -> Option<Value> {
        self.skip_space();
        match *self.chars.peek()? {
            'n' => self.word("null", Value::Null),
            't' => self.word("true", Value::Bool(true)),
            'f' => self.word("false", Value::Bool(false)),
            '"' => self.string().map(Value::String),
            '[' => {
                self.chars.next();
                let mut items = Vec::new();
                if self.eat(']').is_none() {
                    loop {
                        items.push(self.value()?);
                        if self.eat(']').is_some() {
                            break;
                        }
                        self.eat(',')?;
                    }
                }
                Some(Value::Array(items))
            }
            '{' => {
                self.chars.next();
                let mut fields = Vec::new();
                if self.eat('}').is_none() {
                    loop {
                        self.skip_space();
                        let key = self.string()?;
                        self.eat(':')?;
                        fields.push((key, self.value()?));
                        if self.eat('}').is_some() {
                            break;
                        }
                        self.eat(',')?;
                    }
                }
                Some(Value::Object(fields))
            }
            _ => {
                let mut number = String::new();
                while let Some(c) = self
                    .chars
                    .next_if(|c| c.is_ascii_digit() || "+-.eE".contains(*c))
                {
                    number.push(c);
                }
                (!number.is_empty()).then_some(Value::Number(number))
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        self.chars.next_if_eq(&'"')?;
        let mut out = String::new();
        loop {
            match self.chars.next()? {
                '"' => return Some(out),
                '\\' => match self.chars.next()? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'u' => {
                        let unit = self.hex4()?;
                        let c = if (0xD800..0xDC00).contains(&unit) {
                            // A surrogate pair: 😀.
                            self.chars.next_if_eq(&'\\')?;
                            self.chars.next_if_eq(&'u')?;
                            let low = self.hex4()?;
                            char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))
                        } else {
                            char::from_u32(unit)
                        };
                        out.push(c.unwrap_or('\u{FFFD}'));
                    }
                    c => out.push(c),
                },
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut n = 0;
        for _ in 0..4 {
            n = n * 16 + self.chars.next()?.to_digit(16)?;
        }
        Some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_values() {
        let v = parse(r#" {"a": [1, -2.5e3, true, null], "b": {"c": "x\"é😀\n"}} "#).unwrap();
        assert_eq!(
            v.get("a").as_array(),
            &[
                Value::Number("1".into()),
                Value::Number("-2.5e3".into()),
                Value::Bool(true),
                Value::Null
            ]
        );
        assert_eq!(v.get("b").get("c").as_str(), Some("x\"é😀\n"));
        assert_eq!(v.get("missing"), &Value::Null);
        assert_eq!(parse("[]"), Some(Value::Array(vec![])));
        assert_eq!(parse("{}"), Some(Value::Object(vec![])));
    }

    #[test]
    fn rejects_bad_json() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("[1,"), None);
        assert_eq!(parse(r#"{"a" 1}"#), None);
        assert_eq!(parse("[1] x"), None);
        assert_eq!(parse(r#""open"#), None);
    }
}
