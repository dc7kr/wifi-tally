use super::color::ColorSchemeId;
use crate::jsonutil::number;
use serde_json::{Map, Value};
use tracing::warn;

pub const DEFAULT_BRIGHTNESS: f64 = 100.0;
pub const MIN_OPERATOR_BRIGHTNESS: f64 = 1.0;

fn clamp_operator(value: f64) -> f64 {
    if value < MIN_OPERATOR_BRIGHTNESS {
        warn!(
            "Operator Light Brightness can not be below {MIN_OPERATOR_BRIGHTNESS} to prevent a \
             tally light that stays dark because of configuration, but got {value}. \
             Using {MIN_OPERATOR_BRIGHTNESS} instead."
        );
        MIN_OPERATOR_BRIGHTNESS
    } else {
        value.min(100.0)
    }
}

fn clamp_stage(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

fn opt_number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}
fn opt_bool(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}
fn opt_scheme(v: &Value, key: &str) -> Option<ColorSchemeId> {
    let s = v.get(key)?.as_str()?;
    let id = ColorSchemeId::parse(s);
    if id.is_none() {
        warn!("ignoring unknown color scheme \"{s}\"");
    }
    id
}

/// Settings of one tally. Everything that is not set falls back to
/// [`DefaultTallyConfiguration`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TallyConfiguration {
    stage_brightness: Option<f64>,
    operator_brightness: Option<f64>,
    pub stage_color: Option<ColorSchemeId>,
    pub operator_color: Option<ColorSchemeId>,
    pub stage_shows_preview: Option<bool>,
    pub operator_shows_idle: Option<bool>,
}

impl TallyConfiguration {
    pub fn stage_brightness(&self) -> Option<f64> {
        self.stage_brightness
    }
    pub fn set_stage_brightness(&mut self, v: Option<f64>) {
        self.stage_brightness = v.map(clamp_stage);
    }
    pub fn operator_brightness(&self) -> Option<f64> {
        self.operator_brightness
    }
    pub fn set_operator_brightness(&mut self, v: Option<f64>) {
        self.operator_brightness = v.map(clamp_operator);
    }

    pub fn to_json_map(&self) -> Map<String, Value> {
        let mut m = Map::new();
        if let Some(v) = self.stage_brightness {
            m.insert("stBrightness".into(), number(v));
        }
        if let Some(v) = self.operator_brightness {
            m.insert("opBrightness".into(), number(v));
        }
        if let Some(v) = self.stage_color {
            m.insert("stColor".into(), v.as_str().into());
        }
        if let Some(v) = self.operator_color {
            m.insert("opColor".into(), v.as_str().into());
        }
        if let Some(v) = self.stage_shows_preview {
            m.insert("stPreview".into(), v.into());
        }
        if let Some(v) = self.operator_shows_idle {
            m.insert("opIdle".into(), v.into());
        }
        m
    }

    /// Replaces every field; a missing field resets it to "inherit the default".
    pub fn from_json(v: &Value) -> Self {
        let mut c = TallyConfiguration::default();
        c.set_operator_brightness(opt_number(v, "opBrightness"));
        c.set_stage_brightness(opt_number(v, "stBrightness"));
        c.stage_color = opt_scheme(v, "stColor");
        c.operator_color = opt_scheme(v, "opColor");
        c.stage_shows_preview = opt_bool(v, "stPreview");
        c.operator_shows_idle = opt_bool(v, "opIdle");
        c
    }
}

/// Values used when an individual tally does not override them.
#[derive(Clone, Debug, PartialEq)]
pub struct DefaultTallyConfiguration {
    stage_brightness: f64,
    operator_brightness: f64,
    pub stage_color: ColorSchemeId,
    pub operator_color: ColorSchemeId,
    pub stage_shows_preview: bool,
    pub operator_shows_idle: bool,
}

impl Default for DefaultTallyConfiguration {
    fn default() -> Self {
        DefaultTallyConfiguration {
            stage_brightness: DEFAULT_BRIGHTNESS,
            operator_brightness: DEFAULT_BRIGHTNESS,
            stage_color: ColorSchemeId::Default,
            operator_color: ColorSchemeId::Default,
            stage_shows_preview: true,
            operator_shows_idle: true,
        }
    }
}

impl DefaultTallyConfiguration {
    pub fn stage_brightness(&self) -> f64 {
        self.stage_brightness
    }
    /// `None` resets to the default.
    pub fn set_stage_brightness(&mut self, v: Option<f64>) {
        self.stage_brightness = clamp_stage(v.unwrap_or(DEFAULT_BRIGHTNESS));
    }
    pub fn operator_brightness(&self) -> f64 {
        self.operator_brightness
    }
    pub fn set_operator_brightness(&mut self, v: Option<f64>) {
        self.operator_brightness = clamp_operator(v.unwrap_or(DEFAULT_BRIGHTNESS));
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("stBrightness".into(), number(self.stage_brightness));
        m.insert("opBrightness".into(), number(self.operator_brightness));
        m.insert("stColor".into(), self.stage_color.as_str().into());
        m.insert("opColor".into(), self.operator_color.as_str().into());
        m.insert("stPreview".into(), self.stage_shows_preview.into());
        m.insert("opIdle".into(), self.operator_shows_idle.into());
        Value::Object(m)
    }

    /// Only touches the fields that are present.
    pub fn load_json(&mut self, v: &Value) {
        if v.get("opBrightness").is_some() {
            self.set_operator_brightness(opt_number(v, "opBrightness"));
        }
        if v.get("stBrightness").is_some() {
            self.set_stage_brightness(opt_number(v, "stBrightness"));
        }
        if let Some(s) = opt_scheme(v, "stColor") {
            self.stage_color = s;
        }
        if let Some(s) = opt_scheme(v, "opColor") {
            self.operator_color = s;
        }
        if let Some(b) = opt_bool(v, "stPreview") {
            self.stage_shows_preview = b;
        }
        if let Some(b) = opt_bool(v, "opIdle") {
            self.operator_shows_idle = b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults() {
        let c = DefaultTallyConfiguration::default();
        assert_eq!(c.stage_brightness(), 100.0);
        assert_eq!(c.operator_brightness(), 100.0);
        assert_eq!(c.stage_color, ColorSchemeId::Default);
        assert!(c.stage_shows_preview && c.operator_shows_idle);
    }

    #[test]
    fn default_stage_brightness_is_clamped() {
        for (input, expected) in [
            (Some(-1.0), 0.0),
            (Some(0.0), 0.0),
            (Some(42.0), 42.0),
            (Some(101.0), 100.0),
            (None, 100.0),
        ] {
            let mut c = DefaultTallyConfiguration::default();
            c.set_stage_brightness(input);
            assert_eq!(c.stage_brightness(), expected, "input {input:?}");
        }
    }

    #[test]
    fn operator_light_can_never_be_turned_off() {
        for (input, expected) in [
            (Some(0.0), 1.0),
            (Some(1.0), 1.0),
            (Some(42.0), 42.0),
            (Some(101.0), 100.0),
            (None, 100.0),
        ] {
            let mut c = DefaultTallyConfiguration::default();
            c.set_operator_brightness(input);
            assert_eq!(c.operator_brightness(), expected, "input {input:?}");
            let mut t = TallyConfiguration::default();
            t.set_operator_brightness(input);
            assert_eq!(
                t.operator_brightness(),
                input.map(|_| expected),
                "tally, input {input:?}"
            );
        }
    }

    #[test]
    fn default_config_round_trips_falsy_values() {
        let mut c = DefaultTallyConfiguration::default();
        c.set_stage_brightness(Some(0.0));
        c.stage_shows_preview = false;
        c.operator_shows_idle = false;
        c.operator_color = ColorSchemeId::YellowPink;
        let mut loaded = DefaultTallyConfiguration::default();
        loaded.load_json(&c.to_json());
        assert_eq!(loaded, c);
    }

    #[test]
    fn default_config_only_touches_present_fields() {
        let mut c = DefaultTallyConfiguration::default();
        c.set_operator_brightness(Some(42.0));
        c.load_json(&json!({"stBrightness": 60}));
        assert_eq!(c.operator_brightness(), 42.0);
        assert_eq!(c.stage_brightness(), 60.0);
    }

    #[test]
    fn tally_config_round_trips_and_keeps_unset_fields_unset() {
        let mut c = TallyConfiguration::default();
        c.set_stage_brightness(Some(0.0));
        c.stage_shows_preview = Some(false);
        let json = Value::Object(c.to_json_map());
        assert_eq!(json, json!({"stBrightness": 0, "stPreview": false}));
        assert_eq!(TallyConfiguration::from_json(&json), c);
    }
}
