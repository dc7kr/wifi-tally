use super::log::Log;
use super::tally_config::TallyConfiguration;
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TallyType {
    Udp,
    Web,
}

impl TallyType {
    pub fn as_str(self) -> &'static str {
        match self {
            TallyType::Udp => "udp",
            TallyType::Web => "web",
        }
    }
    pub fn parse(s: &str) -> Option<TallyType> {
        match s {
            "udp" => Some(TallyType::Udp),
            "web" => Some(TallyType::Web),
            _ => None,
        }
    }
}

/// The numeric values are part of the socket protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected = 0,
    Connected = 1,
    Missing = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TallyKind {
    Udp {
        address: Option<String>,
        port: Option<u16>,
        state: ConnectionState,
    },
    Web {
        /// Addresses of the browsers that currently show this tally.
        connected_clients: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tally {
    pub name: String,
    pub channel_id: Option<String>,
    pub highlight: bool,
    pub logs: Vec<Log>,
    pub configuration: TallyConfiguration,
    pub kind: TallyKind,
}

impl Tally {
    pub fn new_udp(name: impl Into<String>, channel_id: Option<String>) -> Self {
        Tally {
            name: name.into(),
            channel_id,
            highlight: false,
            logs: Vec::new(),
            configuration: TallyConfiguration::default(),
            kind: TallyKind::Udp {
                address: None,
                port: None,
                state: ConnectionState::Disconnected,
            },
        }
    }

    pub fn new_web(name: impl Into<String>, channel_id: Option<String>) -> Self {
        Tally {
            kind: TallyKind::Web {
                connected_clients: Vec::new(),
            },
            ..Tally::new_udp(name, channel_id)
        }
    }

    pub fn new(ty: TallyType, name: impl Into<String>) -> Self {
        match ty {
            TallyType::Udp => Tally::new_udp(name, None),
            TallyType::Web => Tally::new_web(name, None),
        }
    }

    pub fn tally_type(&self) -> TallyType {
        match self.kind {
            TallyKind::Udp { .. } => TallyType::Udp,
            TallyKind::Web { .. } => TallyType::Web,
        }
    }

    /// Unique over both tally types, e.g. `udp-Camera 1`.
    pub fn id(&self) -> String {
        format!("{}-{}", self.tally_type().as_str(), self.name)
    }

    pub fn is_patched(&self) -> bool {
        self.channel_id.is_some()
    }

    /// Whether the mixer shows this tally on one of the given channels.
    pub fn is_in(&self, channels: &[String]) -> bool {
        match &self.channel_id {
            Some(id) => channels.iter().any(|c| c == id),
            None => false,
        }
    }

    pub fn is_connected(&self) -> bool {
        match &self.kind {
            TallyKind::Udp {
                address,
                port,
                state,
            } => address.is_some() && port.is_some() && *state == ConnectionState::Connected,
            TallyKind::Web { connected_clients } => !connected_clients.is_empty(),
        }
    }

    /// An active tally is one we should send state updates to.
    pub fn is_active(&self) -> bool {
        match &self.kind {
            TallyKind::Udp {
                address,
                port,
                state,
            } => address.is_some() && port.is_some() && *state != ConnectionState::Disconnected,
            TallyKind::Web { connected_clients } => !connected_clients.is_empty(),
        }
    }

    pub fn is_missing(&self) -> bool {
        match &self.kind {
            TallyKind::Udp {
                address,
                port,
                state,
            } => address.is_some() && port.is_some() && *state == ConnectionState::Missing,
            TallyKind::Web { .. } => false,
        }
    }

    fn base_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("name".into(), self.name.clone().into());
        m.insert("type".into(), self.tally_type().as_str().into());
        if let Some(c) = &self.channel_id {
            m.insert("channelId".into(), c.clone().into());
        }
        m.extend(self.configuration.to_json_map());
        m
    }

    /// What is written to the configuration file.
    pub fn to_json_for_save(&self) -> Value {
        Value::Object(self.base_json())
    }

    /// What is sent to the browser.
    pub fn to_json(&self) -> Value {
        let mut m = self.base_json();
        match &self.kind {
            TallyKind::Udp {
                address,
                port,
                state,
            } => {
                if let Some(a) = address {
                    m.insert("address".into(), a.clone().into());
                }
                if let Some(p) = port {
                    m.insert("port".into(), (*p).into());
                }
                m.insert("state".into(), (*state as u8).into());
            }
            TallyKind::Web { connected_clients } => {
                let clients: Vec<Value> = connected_clients
                    .iter()
                    .map(|a| json!({ "address": a }))
                    .collect();
                m.insert("connectedClients".into(), clients.into());
            }
        }
        Value::Object(m)
    }

    pub fn from_json_for_save(v: &Value) -> Result<Tally, String> {
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("tally has no name: {v}"))?;
        let channel_id = match v.get("channelId") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        // Udp was the only type in the beginning, so no type means "udp".
        let mut tally = match v.get("type").and_then(Value::as_str) {
            Some("web") => Tally::new_web(name, channel_id),
            _ => Tally::new_udp(name, channel_id),
        };
        tally.configuration = TallyConfiguration::from_json(v);
        Ok(tally)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_in() {
        let t = Tally::new_udp("t", Some("1".into()));
        assert!(t.is_in(&["1".into(), "2".into()]));
        assert!(!t.is_in(&["2".into()]));
        assert!(!Tally::new_udp("t", None).is_in(&["1".into()]));
    }

    #[test]
    fn udp_activity() {
        let mut t = Tally::new_udp("t", None);
        assert!(!t.is_active() && !t.is_connected() && !t.is_missing());
        t.kind = TallyKind::Udp {
            address: Some("10.0.0.1".into()),
            port: Some(1234),
            state: ConnectionState::Missing,
        };
        assert!(t.is_active() && !t.is_connected() && t.is_missing());
    }

    #[test]
    fn save_round_trip_keeps_type_and_settings() {
        let saved = json!({
            "name": "Tally04", "type": "web", "channelId": "4",
            "stBrightness": 100, "stPreview": true
        });
        let t = Tally::from_json_for_save(&saved).unwrap();
        assert_eq!(t.tally_type(), TallyType::Web);
        assert_eq!(t.to_json_for_save(), saved);
    }

    #[test]
    fn missing_type_means_udp() {
        let t = Tally::from_json_for_save(&json!({"name": "old"})).unwrap();
        assert_eq!(t.tally_type(), TallyType::Udp);
    }

    #[test]
    fn json_for_browser() {
        let mut t = Tally::new_web("w", None);
        if let TallyKind::Web { connected_clients } = &mut t.kind {
            connected_clients.push("1.2.3.4".into());
        }
        assert_eq!(
            t.to_json(),
            json!({"name": "w", "type": "web", "connectedClients": [{"address": "1.2.3.4"}]})
        );
        assert_eq!(
            Tally::new_udp("u", Some("1".into())).to_json(),
            json!({"name": "u", "type": "udp", "channelId": "1", "state": 0})
        );
    }
}
