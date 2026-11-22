use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: String,
    pub name: Option<String>,
}

impl Channel {
    pub fn new(id: impl Into<String>, name: Option<String>) -> Self {
        Channel {
            id: id.into(),
            name,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), Value::String(self.id.clone()));
        if let Some(name) = &self.name {
            m.insert("name".into(), Value::String(name.clone()));
        }
        Value::Object(m)
    }

    pub fn from_json(v: &Value) -> Result<Channel, String> {
        let obj = v
            .as_object()
            .ok_or_else(|| format!("expected a channel object, got {v}"))?;
        let id = match obj.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            other => return Err(format!("channel has no valid id: {other:?}")),
        };
        let name = match obj.get("name") {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        };
        Ok(Channel { id, name })
    }
}

/// The channels used when no mixer provides any: "1" to "8".
pub fn default_channels() -> Vec<Channel> {
    (1..=8).map(|i| Channel::new(i.to_string(), None)).collect()
}
