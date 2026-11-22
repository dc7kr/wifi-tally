//! Settings of every supported video mixer.
//!
//! They live here even for mixers whose connector is not ported yet: the
//! browser needs `config.state.<mixer>` for its settings pages and the
//! configuration file must keep them intact.

use crate::domain::channel::Channel;
use crate::jsonutil::{self, channel_list, load_ip, load_number, load_port, number};
use serde_json::{json, Map, Value};
use tracing::error;

pub const ATEM: &str = "atem";
pub const MOCK: &str = "mock";
pub const NULL: &str = "null";
pub const OBS: &str = "obs";
pub const ROLAND_V8HD: &str = "rolandV8HD";
pub const ROLAND_V60HD: &str = "rolandV60HD";
pub const TEST: &str = "test";
pub const VMIX: &str = "vmix";

/// Where a mixer can be reached.
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub ip: String,
    pub port: u16,
}

impl Endpoint {
    fn new(ip: &str, port: u16) -> Self {
        Endpoint {
            ip: ip.to_string(),
            port,
        }
    }

    fn load(&mut self, data: &Value) {
        if let Some(ip) = load_ip(data, "ip") {
            self.ip = ip;
        }
        if let Some(port) = load_port(data, "port") {
            self.port = port;
        }
    }

    fn write(&self, m: &mut Map<String, Value>) {
        m.insert("ip".into(), self.ip.clone().into());
        m.insert("port".into(), self.port.into());
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObsConfig {
    pub endpoint: Endpoint,
    /// "always", "stream", "record" or "streamOrRecord"
    pub live_mode: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RolandV8HDConfig {
    pub request_interval: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RolandV60HDConfig {
    pub endpoint: Endpoint,
    pub request_interval: f64,
}

/// Settings of the simulated mixer that is only available during development.
#[derive(Clone, Debug, PartialEq)]
pub struct MockConfig {
    pub channel_count: usize,
    pub channel_names: Vec<String>,
    pub tick_time: f64,
}

impl MockConfig {
    pub fn channels(&self) -> Vec<Channel> {
        (0..self.channel_count)
            .map(|i| Channel::new((i + 1).to_string(), self.channel_names.get(i).cloned()))
            .collect()
    }

    fn load(&mut self, data: &Value) {
        if let Some(n) = load_number(data, "channelCount") {
            if n < 0.0 || n.fract() != 0.0 {
                error!(
                    "error loading property \"channelCount\" of configuration: \
                     channel count needs to be a positive integer, but got {n}"
                );
            } else {
                self.channel_count = n as usize;
            }
        }
        match data.get("channelNames") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) => {
                self.channel_names = s.split(',').map(|n| n.trim().to_string()).collect();
            }
            Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
                self.channel_names = items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|n| n.trim().to_string())
                    .collect();
            }
            Some(_) => error!("error loading property \"channelNames\": invalid type"),
        }
        if let Some(t) = load_number(data, "tickTime") {
            if t <= 0.0 {
                error!(
                    "error loading property \"tickTime\" of configuration: \
                     tickTime needs to be a positive number, but got {t}"
                );
            } else {
                self.tick_time = t;
            }
        }
    }
}

/// Settings of the scripted mixer used by the end-to-end tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestConfig {
    pub programs: Option<Vec<String>>,
    pub previews: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MixerSettings {
    pub atem: Endpoint,
    pub mock: MockConfig,
    pub obs: ObsConfig,
    pub roland_v8hd: RolandV8HDConfig,
    pub roland_v60hd: RolandV60HDConfig,
    pub test: TestConfig,
    pub vmix: Endpoint,
}

impl Default for MixerSettings {
    fn default() -> Self {
        MixerSettings {
            atem: Endpoint::new("127.0.0.1", 9910),
            mock: MockConfig {
                channel_count: 8,
                channel_names: Vec::new(),
                tick_time: 3000.0,
            },
            obs: ObsConfig {
                endpoint: Endpoint::new("127.0.0.1", 4444),
                live_mode: "always".into(),
            },
            roland_v8hd: RolandV8HDConfig {
                request_interval: 100.0,
            },
            roland_v60hd: RolandV60HDConfig {
                endpoint: Endpoint::new("127.0.0.1", 80),
                request_interval: 250.0,
            },
            test: TestConfig::default(),
            vmix: Endpoint::new("127.0.0.1", 8099),
        }
    }
}

fn opt_list(v: &Option<Vec<String>>) -> Value {
    match v {
        Some(items) => json!(items),
        None => Value::Null,
    }
}

impl MixerSettings {
    /// The JSON the browser sees and the file stores. `None` for mixers without settings.
    pub fn to_json(&self, id: &str) -> Option<Value> {
        let mut m = Map::new();
        match id {
            ATEM => self.atem.write(&mut m),
            VMIX => self.vmix.write(&mut m),
            OBS => {
                self.obs.endpoint.write(&mut m);
                m.insert("liveMode".into(), self.obs.live_mode.clone().into());
            }
            ROLAND_V8HD => {
                m.insert(
                    "requestInterval".into(),
                    number(self.roland_v8hd.request_interval),
                );
            }
            ROLAND_V60HD => {
                self.roland_v60hd.endpoint.write(&mut m);
                m.insert(
                    "requestInterval".into(),
                    number(self.roland_v60hd.request_interval),
                );
            }
            MOCK => {
                m.insert("channelCount".into(), self.mock.channel_count.into());
                m.insert("channelNames".into(), json!(self.mock.channel_names));
                m.insert("tickTime".into(), number(self.mock.tick_time));
            }
            TEST => {
                m.insert("programs".into(), opt_list(&self.test.programs));
                m.insert("previews".into(), opt_list(&self.test.previews));
            }
            NULL => {}
            _ => return None,
        }
        Some(Value::Object(m))
    }

    /// Replaces the settings of one mixer with the defaults plus whatever
    /// valid values `data` contains. Invalid fields are logged and skipped.
    pub fn load(&mut self, id: &str, data: &Value) {
        let defaults = MixerSettings::default();
        match id {
            ATEM => {
                self.atem = defaults.atem;
                self.atem.load(data);
            }
            VMIX => {
                self.vmix = defaults.vmix;
                self.vmix.load(data);
            }
            OBS => {
                self.obs = defaults.obs;
                self.obs.endpoint.load(data);
                if let Some(mode) = jsonutil::load_string(data, "liveMode") {
                    if ["always", "stream", "record", "streamOrRecord"].contains(&mode.as_str()) {
                        self.obs.live_mode = mode;
                    } else {
                        error!("error loading property \"liveMode\": unknown mode \"{mode}\"");
                    }
                }
            }
            ROLAND_V8HD => {
                self.roland_v8hd = defaults.roland_v8hd;
                if let Some(n) = load_number(data, "requestInterval") {
                    self.roland_v8hd.request_interval = n;
                }
            }
            ROLAND_V60HD => {
                self.roland_v60hd = defaults.roland_v60hd;
                self.roland_v60hd.endpoint.load(data);
                if let Some(n) = load_number(data, "requestInterval") {
                    self.roland_v60hd.request_interval = n;
                }
            }
            MOCK => {
                self.mock = defaults.mock;
                self.mock.load(data);
            }
            TEST => {
                self.test = TestConfig {
                    programs: data.get("programs").and_then(channel_list),
                    previews: data.get("previews").and_then(channel_list),
                };
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(id: &str, s: &MixerSettings) -> MixerSettings {
        let mut other = MixerSettings::default();
        other.load(id, &s.to_json(id).unwrap());
        other
    }

    #[test]
    fn every_mixer_persists_its_settings() {
        let mut s = MixerSettings::default();
        s.atem.ip = "10.1.1.42".into();
        s.obs.endpoint.ip = "10.1.1.43".into();
        s.obs.live_mode = "stream".into();
        s.vmix.port = 8088;
        s.roland_v8hd.request_interval = 500.0;
        s.roland_v60hd.endpoint.ip = "10.1.1.45".into();
        s.roland_v60hd.request_interval = 42.0;
        s.mock.channel_count = 42;
        s.mock.channel_names = vec!["a".into(), "b".into()];
        s.test.programs = Some(vec!["1".into()]);
        for id in [ATEM, OBS, VMIX, ROLAND_V8HD, ROLAND_V60HD, MOCK, TEST] {
            assert_eq!(round_trip(id, &s).to_json(id), s.to_json(id), "{id}");
        }
    }

    #[test]
    fn defaults_match_the_typescript_hub() {
        let s = MixerSettings::default();
        assert_eq!(
            s.to_json(ATEM).unwrap(),
            json!({"ip": "127.0.0.1", "port": 9910})
        );
        assert_eq!(
            s.to_json(OBS).unwrap(),
            json!({"ip": "127.0.0.1", "port": 4444, "liveMode": "always"})
        );
        assert_eq!(
            s.to_json(VMIX).unwrap(),
            json!({"ip": "127.0.0.1", "port": 8099})
        );
        assert_eq!(
            s.to_json(ROLAND_V8HD).unwrap(),
            json!({"requestInterval": 100})
        );
        assert_eq!(
            s.to_json(ROLAND_V60HD).unwrap(),
            json!({"ip": "127.0.0.1", "port": 80, "requestInterval": 250})
        );
        assert_eq!(s.to_json(NULL).unwrap(), json!({}));
        assert_eq!(s.to_json("nonsense"), None);
    }

    #[test]
    fn invalid_fields_are_skipped_but_valid_ones_kept() {
        let mut s = MixerSettings::default();
        s.load(ATEM, &json!({"ip": "999.1.1.1", "port": 1234}));
        assert_eq!(s.atem, Endpoint::new("127.0.0.1", 1234));
        s.load(ATEM, &json!({"ip": "localhost", "port": "80"}));
        assert_eq!(s.atem, Endpoint::new("localhost", 9910));
        s.load(ATEM, &json!({"port": 70000}));
        assert_eq!(s.atem.port, 9910);
    }

    #[test]
    fn mock_settings() {
        let mut s = MixerSettings::default();
        s.load(
            MOCK,
            &json!({"channelCount": 3, "channelNames": "Cam A, Cam B", "tickTime": 500}),
        );
        assert_eq!(s.mock.channel_count, 3);
        let names: Vec<_> = s.mock.channels().into_iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            vec![Some("Cam A".into()), Some("Cam B".into()), None]
        );
        s.load(MOCK, &json!({"channelCount": -1, "tickTime": 0}));
        assert_eq!((s.mock.channel_count, s.mock.tick_time), (8, 3000.0));
    }
}
