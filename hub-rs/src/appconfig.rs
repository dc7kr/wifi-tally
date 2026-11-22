//! Everything that is persisted in the configuration file.

use crate::domain::channel::{default_channels, Channel};
use crate::domain::tally::Tally;
use crate::domain::tally_config::DefaultTallyConfiguration;
use crate::jsonutil::load_string;
use crate::mixer::config::{self, MixerSettings};
use serde_json::{Map, Value};
use tracing::error;

#[derive(Clone, Debug, PartialEq)]
pub struct AppConfig {
    pub mixer: Option<String>,
    pub mixers: MixerSettings,
    pub tally_defaults: DefaultTallyConfiguration,
    pub channels: Vec<Channel>,
    pub tallies: Vec<Tally>,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            mixer: None,
            mixers: MixerSettings::default(),
            tally_defaults: DefaultTallyConfiguration::default(),
            channels: default_channels(),
            tallies: Vec::new(),
        }
    }
}

/// Order matters: it is the order of the keys in the file.
const MIXER_KEYS: [&str; 8] = [
    config::ATEM,
    config::MOCK,
    config::NULL,
    config::OBS,
    config::ROLAND_V8HD,
    config::ROLAND_V60HD,
    config::TEST,
    config::VMIX,
];

fn load_array<T>(
    data: &Value,
    field: &str,
    parse: impl Fn(&Value) -> Result<T, String>,
) -> Option<Vec<T>> {
    match data.get(field) {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => match items.iter().map(parse).collect() {
            Ok(v) => Some(v),
            Err(e) => {
                error!("error loading property \"{field}\" of configuration: {e}");
                None
            }
        },
        Some(_) => {
            error!("error loading property \"{field}\": invalid type");
            None
        }
    }
}

impl AppConfig {
    /// Loads whatever is valid. Problems are logged and the affected property
    /// keeps its current value.
    pub fn load_json(&mut self, data: &Value) {
        for id in MIXER_KEYS {
            // like the JS hub: missing, null and false mean "not set"
            if let Some(v) = data.get(id) {
                if !v.is_null() && v != &Value::Bool(false) {
                    self.mixers.load(id, v);
                }
            }
        }
        if let Some(v) = data.get("tallyDefaults") {
            self.tally_defaults.load_json(v);
        }
        if let Some(mixer) = load_string(data, "mixer") {
            self.mixer = Some(mixer);
        }
        if let Some(channels) = load_array(data, "channels", Channel::from_json) {
            self.channels = channels;
        }
        if let Some(tallies) = load_array(data, "tallies", Tally::from_json_for_save) {
            self.tallies = tallies;
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        if let Some(mixer) = &self.mixer {
            m.insert("mixer".into(), mixer.clone().into());
        }
        for id in MIXER_KEYS {
            m.insert(id.into(), self.mixers.to_json(id).unwrap_or(Value::Null));
        }
        m.insert("tallyDefaults".into(), self.tally_defaults.to_json());
        m.insert(
            "tallies".into(),
            self.tallies.iter().map(Tally::to_json_for_save).collect(),
        );
        m.insert(
            "channels".into(),
            self.channels.iter().map(Channel::to_json).collect(),
        );
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::color::ColorSchemeId;
    use crate::domain::tally::TallyType;
    use serde_json::json;

    #[test]
    fn round_trips() {
        let mut c = AppConfig {
            mixer: Some("vmix".into()),
            ..AppConfig::default()
        };
        c.mixers.atem.ip = "10.1.1.42".into();
        c.tally_defaults.operator_color = ColorSchemeId::YellowPink;
        c.channels = vec![
            Channel::new("1", Some("Cam".into())),
            Channel::new("2", None),
        ];
        c.tallies = vec![
            Tally::new_udp("a", Some("1".into())),
            Tally::new_web("b", None),
        ];

        let mut other = AppConfig::default();
        other.load_json(&c.to_json());
        assert_eq!(other, c);
    }

    #[test]
    fn missing_properties_keep_defaults() {
        let mut c = AppConfig::default();
        c.load_json(&json!({}));
        assert_eq!(c, AppConfig::default());
        assert!(c.mixer.is_none());
    }

    #[test]
    fn invalid_properties_are_ignored() {
        let mut c = AppConfig::default();
        c.load_json(&json!({"mixer": 5, "channels": "nope", "tallies": [{"type": "udp"}]}));
        assert_eq!(c, AppConfig::default());
    }

    #[test]
    fn numeric_channel_ids_become_strings() {
        let mut c = AppConfig::default();
        c.load_json(&json!({"channels": [{"id": 1, "name": "x"}]}));
        assert_eq!(c.channels, vec![Channel::new("1", Some("x".into()))]);
    }

    /// Files written by older versions of the TypeScript hub must keep loading.
    #[test]
    fn loads_the_fixtures_of_older_versions() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../hub/fixtures");
        for file in [
            "settings-v0.2.1.json",
            "settings-v0.3.0.json",
            "settings-v0.4.0.json",
        ] {
            let raw = std::fs::read_to_string(format!("{dir}/{file}")).unwrap();
            let mut c = AppConfig::default();
            c.load_json(&serde_json::from_str(&raw).unwrap());
            assert!(c.mixer.is_some(), "{file}");
            assert!(!c.tallies.is_empty(), "{file}");
        }
    }

    #[test]
    fn fixture_v0_4_0_is_understood_in_detail() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../hub/fixtures/settings-v0.4.0.json"
        ))
        .unwrap();
        let mut c = AppConfig::default();
        c.load_json(&serde_json::from_str(&raw).unwrap());
        assert_eq!(c.mixer.as_deref(), Some("vmix"));
        assert_eq!(c.mixers.atem.port, 9910);
        assert_eq!(c.tally_defaults.stage_brightness(), 60.0);
        assert_eq!(c.tally_defaults.operator_color, ColorSchemeId::YellowPink);
        assert!(!c.tally_defaults.stage_shows_preview);
        assert_eq!(c.tallies.len(), 4);
        assert_eq!(c.tallies[3].tally_type(), TallyType::Web);
        assert_eq!(c.tallies[0].configuration.stage_brightness(), Some(100.0));
        assert_eq!(c.channels[0].name.as_deref(), Some("Dave's Cam"));
    }
}
