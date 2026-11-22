//! Helpers for reading and writing the lenient JSON the hub exchanges with the
//! browser and stores on disk.
//!
//! The TypeScript hub never rejects a whole configuration because of one bad
//! field: it logs the problem and keeps the previous value. The `load_*`
//! functions mirror that behaviour.

use serde_json::Value;
use tracing::error;

/// Serializes a number the way JavaScript does: `100`, not `100.0`.
pub fn number(v: f64) -> Value {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 9.0e15 {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "object",
        Value::Object(_) => "object",
    }
}

fn present<'a>(data: &'a Value, field: &str) -> Option<&'a Value> {
    match data.get(field) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v),
    }
}

fn invalid_type(field: &str, v: &Value) {
    error!(
        "error loading property \"{field}\": invalid type {}",
        type_name(v)
    );
}

pub fn load_string(data: &Value, field: &str) -> Option<String> {
    let v = present(data, field)?;
    match v.as_str() {
        Some(s) => Some(s.to_string()),
        None => {
            invalid_type(field, v);
            None
        }
    }
}

pub fn load_number(data: &Value, field: &str) -> Option<f64> {
    let v = present(data, field)?;
    match v.as_f64() {
        Some(n) => Some(n),
        None => {
            invalid_type(field, v);
            None
        }
    }
}

pub fn is_valid_ip(address: &str) -> bool {
    if address == "localhost" {
        return true;
    }
    let segments: Vec<&str> = address.split('.').collect();
    segments.len() == 4
        && segments.iter().all(|s| {
            !s.is_empty()
                && s.len() <= 3
                && s.bytes().all(|b| b.is_ascii_digit())
                && s.parse::<u16>().map(|n| n <= 255).unwrap_or(false)
        })
}

pub fn parse_port(n: f64) -> Result<u16, String> {
    if n.fract() == 0.0 && (1.0..=65535.0).contains(&n) {
        Ok(n as u16)
    } else {
        Err(format!("Invalid Port: {n}"))
    }
}

pub fn load_ip(data: &Value, field: &str) -> Option<String> {
    let s = load_string(data, field)?;
    if is_valid_ip(&s) {
        Some(s)
    } else {
        error!("error loading property \"{field}\" of configuration: Invalid IP address: {s}");
        None
    }
}

pub fn load_port(data: &Value, field: &str) -> Option<u16> {
    let n = load_number(data, field)?;
    match parse_port(n) {
        Ok(p) => Some(p),
        Err(e) => {
            error!("error loading property \"{field}\" of configuration: {e}");
            None
        }
    }
}

/// Reads an optional list of strings. Numbers are converted to strings, like
/// the mixer communicator does.
pub fn channel_list(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Array(items) => Some(
            items
                .iter()
                .map(|i| match i {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect(),
        ),
        _ => None,
    }
}
