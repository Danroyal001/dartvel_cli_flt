//! Just enough of Flutter's StandardMessageCodec to read the method calls
//! the runner answers on channels that use it (`flutter/mousecursor`).

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
    /// A type this reader does not decode; reading stops there.
    Unsupported(u8),
}

impl Value {
    /// The value under string key `key` of a map.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries.iter().find(|(k, _)| matches!(k, Value::String(s) if s == key)).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }
}

/// A method call: method name, then arguments.
pub fn decode_method_call(bytes: &[u8]) -> Option<(String, Value)> {
    let mut reader = Reader { bytes, at: 0 };
    let method = match reader.value()? {
        Value::String(method) => method,
        _ => return None,
    };
    let arguments = if reader.at < bytes.len() { reader.value()? } else { Value::Null };
    Some((method, arguments))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let slice = self.bytes.get(self.at..self.at + count)?;
        self.at += count;
        Some(slice)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn size(&mut self) -> Option<usize> {
        Some(match self.byte()? {
            254 => u16::from_le_bytes(self.take(2)?.try_into().ok()?) as usize,
            255 => u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize,
            small => small as usize,
        })
    }

    fn align(&mut self, to: usize) {
        let remainder = self.at % to;
        if remainder != 0 {
            self.at += to - remainder;
        }
    }

    fn value(&mut self) -> Option<Value> {
        Some(match self.byte()? {
            0 => Value::Null,
            1 => Value::Bool(true),
            2 => Value::Bool(false),
            3 => Value::Int(i32::from_le_bytes(self.take(4)?.try_into().ok()?) as i64),
            4 => Value::Int(i64::from_le_bytes(self.take(8)?.try_into().ok()?)),
            6 => {
                self.align(8);
                Value::Float(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
            }
            7 => {
                let length = self.size()?;
                Value::String(String::from_utf8(self.take(length)?.to_vec()).ok()?)
            }
            12 => {
                let length = self.size()?;
                let mut items = Vec::with_capacity(length.min(1024));
                for _ in 0..length {
                    items.push(self.value()?);
                }
                Value::List(items)
            }
            13 => {
                let length = self.size()?;
                let mut entries = Vec::with_capacity(length.min(1024));
                for _ in 0..length {
                    let key = self.value()?;
                    let value = self.value()?;
                    entries.push((key, value));
                }
                Value::Map(entries)
            }
            other => Value::Unsupported(other),
        })
    }
}

/// The success envelope with a null result.
pub const SUCCESS_NULL: [u8; 2] = [0, 0];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mouse_cursor_call_is_read() {
        // activateSystemCursor({device: 0, kind: "click"})
        let mut bytes = vec![7, 20];
        bytes.extend_from_slice(b"activateSystemCursor");
        bytes.extend_from_slice(&[13, 2, 7, 6]);
        bytes.extend_from_slice(b"device");
        bytes.extend_from_slice(&[3, 0, 0, 0, 0, 7, 4]);
        bytes.extend_from_slice(b"kind");
        bytes.extend_from_slice(&[7, 5]);
        bytes.extend_from_slice(b"click");
        let (method, arguments) = decode_method_call(&bytes).unwrap();
        assert_eq!(method, "activateSystemCursor");
        assert_eq!(arguments.get("kind").and_then(Value::as_str), Some("click"));
        assert_eq!(arguments.get("device"), Some(&Value::Int(0)));
    }

    #[test]
    fn long_strings_use_the_wide_size_and_floats_are_aligned() {
        let text = "x".repeat(300);
        let mut bytes = vec![7, 254];
        bytes.extend_from_slice(&300u16.to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
        // list [1.5]: type 12, size 1, then a float aligned to 8 from the start.
        bytes.extend_from_slice(&[12, 1, 6]);
        while bytes.len() % 8 != 0 {
            bytes.push(0);
        }
        bytes.extend_from_slice(&1.5f64.to_le_bytes());
        let (method, arguments) = decode_method_call(&bytes).unwrap();
        assert_eq!(method.len(), 300);
        assert_eq!(arguments, Value::List(vec![Value::Float(1.5)]));
    }

    #[test]
    fn truncated_input_is_none() {
        assert_eq!(decode_method_call(&[7, 10, b'a']), None);
    }
}
