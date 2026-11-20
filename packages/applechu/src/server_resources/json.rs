//! 最小 JSON 解析与序列化
//!
//! 仅覆盖网络响应改写所需的能力，不引入第三方依赖：
//! - 解析对象 / 数组 / 字符串 / 数字 / 布尔 / null；
//! - 保留对象键顺序，数字以原始字面量保存，保证未修改内容可逐字节回写；
//! - 数字读取对「数字被序列化成字符串」的场景保持宽松（MinatoCore 的
//!   SegaStringSerializer 会把整数写成 `"25000"`）。

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    /// 数字保留原始字面量，避免浮点往返丢失精度
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// 按键取值；非对象返回 `None`
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(value) => Some(*value),
            // 兼容布尔被写成字符串
            JsonValue::String(value) => match value.trim() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// 读取整数；数字或数字字符串均可接受
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Number(text) => text.trim().parse::<i64>().ok(),
            JsonValue::String(text) => text.trim().parse::<i64>().ok(),
            _ => None,
        }
    }

    /// 读取字符串数组；非数组返回空切片语义（调用方按「不限制」处理）
    pub fn string_array(&self) -> Vec<String> {
        self.as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn write_to(&self, output: &mut String) {
        match self {
            JsonValue::Null => output.push_str("null"),
            JsonValue::Bool(true) => output.push_str("true"),
            JsonValue::Bool(false) => output.push_str("false"),
            JsonValue::Number(text) => output.push_str(text),
            JsonValue::String(value) => write_json_string(value, output),
            JsonValue::Array(items) => {
                output.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    item.write_to(output);
                }
                output.push(']');
            }
            JsonValue::Object(entries) => {
                output.push('{');
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    write_json_string(key, output);
                    output.push(':');
                    value.write_to(output);
                }
                output.push('}');
            }
        }
    }
}

impl std::fmt::Display for JsonValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = String::new();
        self.write_to(&mut output);
        formatter.write_str(&output)
    }
}

fn write_json_string(value: &str, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            control if (control as u32) < 0x20 => {
                let _ = write!(output, "\\u{:04x}", control as u32);
            }
            other => output.push(other),
        }
    }
    output.push('"');
}

/// 解析 JSON 文本；失败返回 `None`
pub fn parse(text: &str) -> Option<JsonValue> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        position: 0,
    };
    parser.skip_whitespace();
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    // 尾部只允许空白，避免把不完整解析当成成功
    (parser.position == parser.bytes.len()).then_some(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.position) {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn parse_value(&mut self) -> Option<JsonValue> {
        match self.peek()? {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => self.parse_string().map(JsonValue::String),
            b't' => self.parse_literal("true", JsonValue::Bool(true)),
            b'f' => self.parse_literal("false", JsonValue::Bool(false)),
            b'n' => self.parse_literal("null", JsonValue::Null),
            _ => self.parse_number(),
        }
    }

    fn parse_literal(&mut self, literal: &str, value: JsonValue) -> Option<JsonValue> {
        let end = self.position.checked_add(literal.len())?;
        (self.bytes.get(self.position..end)? == literal.as_bytes()).then(|| {
            self.position = end;
            value
        })
    }

    fn parse_object(&mut self) -> Option<JsonValue> {
        self.position += 1; // 跳过 '{'
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b'}' {
            self.position += 1;
            return Some(JsonValue::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.peek()? != b':' {
                return None;
            }
            self.position += 1;
            self.skip_whitespace();
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek()? {
                b',' => self.position += 1,
                b'}' => {
                    self.position += 1;
                    return Some(JsonValue::Object(entries));
                }
                _ => return None,
            }
        }
    }

    fn parse_array(&mut self) -> Option<JsonValue> {
        self.position += 1; // 跳过 '['
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b']' {
            self.position += 1;
            return Some(JsonValue::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.parse_value()?);
            self.skip_whitespace();
            match self.peek()? {
                b',' => self.position += 1,
                b']' => {
                    self.position += 1;
                    return Some(JsonValue::Array(items));
                }
                _ => return None,
            }
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        if self.peek()? != b'"' {
            return None;
        }
        self.position += 1;
        let mut output = String::new();
        loop {
            let byte = self.peek()?;
            self.position += 1;
            match byte {
                b'"' => return Some(output),
                b'\\' => {
                    let escape = self.peek()?;
                    self.position += 1;
                    match escape {
                        b'"' => output.push('"'),
                        b'\\' => output.push('\\'),
                        b'/' => output.push('/'),
                        b'b' => output.push('\u{08}'),
                        b'f' => output.push('\u{0c}'),
                        b'n' => output.push('\n'),
                        b'r' => output.push('\r'),
                        b't' => output.push('\t'),
                        b'u' => output.push(self.parse_unicode_escape()?),
                        _ => return None,
                    }
                }
                // 多字节 UTF-8 序列：按首字节长度整体复制
                other if other < 0x80 => output.push(other as char),
                other => {
                    let extra = match other {
                        0xC0..=0xDF => 1,
                        0xE0..=0xEF => 2,
                        0xF0..=0xF7 => 3,
                        _ => return None,
                    };
                    let start = self.position - 1;
                    let end = start.checked_add(extra + 1)?;
                    let slice = self.bytes.get(start..end)?;
                    output.push_str(std::str::from_utf8(slice).ok()?);
                    self.position = end;
                }
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Option<char> {
        let hex = self.bytes.get(self.position..self.position + 4)?;
        let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
        self.position += 4;
        // 代理对：高代理后必须紧跟 `\uXXXX` 低代理
        if (0xD800..0xDC00).contains(&code) {
            if self.bytes.get(self.position..self.position + 2)? != b"\\u" {
                return None;
            }
            self.position += 2;
            let low_hex = self.bytes.get(self.position..self.position + 4)?;
            let low = u32::from_str_radix(std::str::from_utf8(low_hex).ok()?, 16).ok()?;
            self.position += 4;
            let combined = 0x1_0000 + ((code - 0xD800) << 10) + (low - 0xDC00);
            char::from_u32(combined)
        } else {
            char::from_u32(code)
        }
    }

    fn parse_number(&mut self) -> Option<JsonValue> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                self.position += 1;
            } else {
                break;
            }
        }
        if start == self.position {
            return None;
        }
        let text = std::str::from_utf8(self.bytes.get(start..self.position)?).ok()?;
        Some(JsonValue::Number(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_objects_and_arrays() {
        // Given: 带嵌套的响应片段。
        let text = r#"{"a":{"b":[1,2,{"c":"d"}]},"e":null}"#;

        // When: 解析后按键逐层取值。
        let value = parse(text).expect("有效 JSON 必须解析成功");

        // Then: 结构与取值符合预期。
        let array = value
            .get("a")
            .and_then(|a| a.get("b"))
            .and_then(JsonValue::as_array)
            .expect("嵌套数组必须存在");
        assert_eq!(array.len(), 3);
        assert_eq!(array[0].as_i64(), Some(1));
        assert_eq!(array[2].get("c").and_then(JsonValue::as_str), Some("d"));
        assert_eq!(value.get("e"), Some(&JsonValue::Null));
    }

    #[test]
    fn round_trips_without_changing_bytes() {
        // Given: 一段格式紧凑的响应。
        let text = r#"{"gameSetting":{"dataVersion":"2.50.01","maxCountMusic":300},"isAou":false}"#;

        // When: 解析再序列化。
        let value = parse(text).expect("有效 JSON 必须解析成功");

        // Then: 输出与输入逐字节一致（键顺序与数字字面量都保留）。
        assert_eq!(value.to_string(), text);
    }

    #[test]
    fn accepts_numbers_serialized_as_strings() {
        // Given: MinatoCore 会把整数写成字符串。
        let text = r#"{"minimumGameVersion":"25000"}"#;

        // When: 读取该字段。
        let value = parse(text).expect("有效 JSON 必须解析成功");

        // Then: 宽松读取仍能得到整数。
        assert_eq!(
            value.get("minimumGameVersion").and_then(JsonValue::as_i64),
            Some(25000)
        );
    }

    #[test]
    fn decodes_escapes_and_surrogate_pairs() {
        // Given: 含转义与代理对的字符串。
        let text = r#"{"s":"a\"b\\c中😀"}"#;

        // When: 解析。
        let value = parse(text).expect("有效 JSON 必须解析成功");

        // Then: 转义与代理对都被正确还原。
        assert_eq!(
            value.get("s").and_then(JsonValue::as_str),
            Some("a\"b\\c中😀")
        );
    }

    #[test]
    fn rejects_trailing_garbage() {
        // Given: 合法值后跟随多余内容。
        // Then: 解析失败，避免把截断响应当成完整数据。
        assert!(parse(r#"{"a":1} extra"#).is_none());
        assert!(parse(r#"{"a":}"#).is_none());
    }

    #[test]
    fn missing_field_is_none() {
        // Given: 服务器未下发该字段的响应。
        let value = parse(r#"{"gameSetting":{}}"#).expect("有效 JSON 必须解析成功");

        // Then: 缺失字段返回 None，调用方据此跳过处理。
        assert!(value.get("_appleChuResources").is_none());
    }
}
