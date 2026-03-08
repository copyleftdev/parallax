//! Property value types — the flat property model for entities and relationships.
//!
//! **Spec reference:** `specs/01-data-model.md` §1.5

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// A property value. Flat — no nested objects or arrays of objects.
///
/// INV-03: Properties are flat. No nested objects.
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Value {
    String(CompactString),
    Int(i64),
    Float(f64),
    Bool(bool),
    StringList(Vec<CompactString>),
    Null,
}

impl Value {
    /// Approximate heap size in bytes.
    pub fn approx_size(&self) -> usize {
        match self {
            Value::String(s) => s.len(),
            Value::Int(_) => 8,
            Value::Float(_) => 8,
            Value::Bool(_) => 1,
            Value::StringList(v) => v.iter().map(|s| s.len()).sum::<usize>() + 24,
            Value::Null => 0,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(CompactString::new(s))
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(CompactString::from(s))
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Int(n)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Float(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => v.into(),
            None => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_from_str() {
        let v: Value = "hello".into();
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn value_from_bool() {
        let v: Value = true.into();
        assert_eq!(v.as_bool(), Some(true));
    }

    #[test]
    fn value_from_none() {
        let v: Value = Option::<&str>::None.into();
        assert_eq!(v, Value::Null);
    }
}
