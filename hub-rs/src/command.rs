//! The tiny text protocol between hub and tallies.
//!
//! Tally -> hub: `tally-ho "<name>"` and `log "<name>" <INFO|WARN|ERROR> "<message>"`.
//! Hub -> tally: `O<r>/<g>/<b> S<r>/<g>/<b> [0x<pattern> <step ms>]`.

use crate::domain::color::Color;
use crate::domain::log::{Log, Severity};
use crate::domain::tally::Tally;
use crate::domain::tally_config::DefaultTallyConfiguration;
use regex::Regex;
use std::fmt;
use std::sync::OnceLock;
use tracing::warn;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateCommand {
    Highlight,
    Unknown,
    OnAir,
    Preview,
    Release,
}

impl StateCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            StateCommand::Highlight => "highlight",
            StateCommand::Unknown => "unknown",
            StateCommand::OnAir => "on-air",
            StateCommand::Preview => "preview",
            StateCommand::Release => "release",
        }
    }
}

/// `None` means the mixer is not connected, so nothing is known.
pub type Channels<'a> = Option<&'a [String]>;

pub fn get_state(tally: &Tally, programs: Channels, previews: Channels) -> StateCommand {
    if tally.highlight {
        StateCommand::Highlight
    } else if programs.is_none() && tally.is_patched() {
        StateCommand::Unknown
    } else if programs.is_some_and(|p| tally.is_in(p)) {
        StateCommand::OnAir
    } else if previews.is_some_and(|p| tally.is_in(p)) {
        StateCommand::Preview
    } else {
        StateCommand::Release
    }
}

pub fn create_state_command(
    tally: &Tally,
    programs: Channels,
    previews: Channels,
    defaults: &DefaultTallyConfiguration,
) -> String {
    let conf = &tally.configuration;
    let state = get_state(tally, programs, previews);
    let st_scheme = conf.stage_color.unwrap_or(defaults.stage_color).scheme();
    let op_scheme = conf
        .operator_color
        .unwrap_or(defaults.operator_color)
        .scheme();

    let mut flash_pattern: u8 = 0;
    let mut step_duration: u32 = 0;
    let (op_color, st_color) = match state {
        StateCommand::Highlight => {
            flash_pattern = 0xAA;
            step_duration = 125;
            (op_scheme.highlight, st_scheme.highlight)
        }
        StateCommand::OnAir => (op_scheme.program, st_scheme.program),
        StateCommand::Preview => {
            let show = conf
                .stage_shows_preview
                .unwrap_or(defaults.stage_shows_preview);
            (
                op_scheme.preview,
                if show {
                    st_scheme.preview
                } else {
                    Color::BLACK
                },
            )
        }
        StateCommand::Release => {
            let show = conf
                .operator_shows_idle
                .unwrap_or(defaults.operator_shows_idle);
            (
                if show { op_scheme.idle } else { Color::BLACK },
                Color::BLACK,
            )
        }
        StateCommand::Unknown => {
            flash_pattern = 0x80;
            step_duration = 250;
            (op_scheme.unknown, Color::BLACK)
        }
    };

    let op_color = op_color.with_brightness(
        conf.operator_brightness()
            .unwrap_or(defaults.operator_brightness()),
    );
    let st_color = st_color.with_brightness(
        conf.stage_brightness()
            .unwrap_or(defaults.stage_brightness()),
    );

    let mut command = format!(
        "O{:03}/{:03}/{:03} S{:03}/{:03}/{:03}",
        op_color.r, op_color.g, op_color.b, st_color.r, st_color.g, st_color.b
    );
    if flash_pattern > 0 && step_duration > 0 {
        command.push_str(&format!(" 0x{flash_pattern:02X} {step_duration:03}"));
    }
    command
}

#[derive(Debug, PartialEq)]
pub enum TallyCommand {
    TallyHo { tally_name: String },
    Log { tally_name: String, log: Log },
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidCommand(pub String);

impl fmt::Display for InvalidCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Received an invalid command: \"{}\"", self.0)
    }
}

impl std::error::Error for InvalidCommand {}

fn tally_ho_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^([^ ]+) "(.+)""#).unwrap())
}

fn log_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^([^ ]+) "(.+)" ([^ ]+) "(.*)""#).unwrap())
}

pub fn parse(msg: &str) -> Result<TallyCommand, InvalidCommand> {
    let invalid = || InvalidCommand(msg.to_string());
    if msg.starts_with("tally-ho ") {
        let caps = tally_ho_re().captures(msg).ok_or_else(invalid)?;
        Ok(TallyCommand::TallyHo {
            tally_name: caps[2].to_string(),
        })
    } else if msg.starts_with("log ") {
        let caps = log_re().captures(msg).ok_or_else(invalid)?;
        let severity = match &caps[3] {
            s @ ("INFO" | "WARN" | "ERROR") => Log::severity_from_str(s),
            other => {
                warn!("Invalid severity \"{other}\". Using ERROR instead.");
                Severity::Error
            }
        };
        Ok(TallyCommand::Log {
            tally_name: caps[2].to_string(),
            log: Log::new(severity, &caps[4]),
        })
    } else {
        Err(invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::color::ColorSchemeId;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn cmd(
        tally: &Tally,
        programs: Option<&[&str]>,
        previews: Option<&[&str]>,
        d: &DefaultTallyConfiguration,
    ) -> String {
        let p = programs.map(strs);
        let v = previews.map(strs);
        create_state_command(tally, p.as_deref(), v.as_deref(), d)
    }

    fn patched() -> Tally {
        Tally::new_udp("test", Some("channel".into()))
    }

    #[test]
    fn default_case() {
        let d = DefaultTallyConfiguration::default();
        let mut t = patched();
        t.highlight = true;
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&[]), &d),
            "O255/255/255 S255/255/255 0xAA 125"
        );
        let t = patched();
        assert_eq!(
            cmd(&t, Some(&["other"]), Some(&["channel"]), &d),
            "O000/255/000 S000/255/000"
        );
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&["other"]), &d),
            "O255/000/000 S255/000/000"
        );
        // program wins over preview
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&["channel"]), &d),
            "O255/000/000 S255/000/000"
        );
        assert_eq!(
            cmd(&t, Some(&["other"]), Some(&[]), &d),
            "O000/001/000 S000/000/000"
        );
        // unpatched tally is released
        let unpatched = Tally::new_udp("test", None);
        assert_eq!(
            cmd(&unpatched, Some(&["other"]), Some(&["channel"]), &d),
            "O000/001/000 S000/000/000"
        );
        // mixer not connected
        assert_eq!(
            cmd(&t, None, None, &d),
            "O000/000/255 S000/000/000 0x80 250"
        );
        assert_eq!(cmd(&unpatched, None, None, &d), "O000/001/000 S000/000/000");
    }

    #[test]
    fn stage_brightness() {
        let mut d = DefaultTallyConfiguration::default();
        d.set_stage_brightness(Some(50.0));
        let mut t = patched();
        assert_eq!(
            cmd(&t, Some(&["other"]), Some(&["channel"]), &d),
            "O000/255/000 S000/128/000"
        );
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&[]), &d),
            "O255/000/000 S128/000/000"
        );
        assert_eq!(
            cmd(&t, Some(&[]), Some(&[]), &d),
            "O000/001/000 S000/000/000"
        );
        t.configuration.set_stage_brightness(Some(25.0));
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&[]), &d),
            "O255/000/000 S064/000/000"
        );
        t.configuration.set_stage_brightness(Some(0.0));
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&[]), &d),
            "O255/000/000 S000/000/000"
        );
    }

    #[test]
    fn operator_brightness() {
        let mut d = DefaultTallyConfiguration::default();
        d.set_operator_brightness(Some(50.0));
        let mut t = patched();
        assert_eq!(
            cmd(&t, Some(&["other"]), Some(&["channel"]), &d),
            "O000/128/000 S000/255/000"
        );
        t.configuration.set_operator_brightness(Some(25.0));
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&[]), &d),
            "O064/000/000 S255/000/000"
        );
    }

    #[test]
    fn color_schemes() {
        let mut d = DefaultTallyConfiguration::default();
        d.stage_color = ColorSchemeId::YellowPink;
        let mut t = patched();
        assert_eq!(
            cmd(&t, Some(&["other"]), Some(&["channel"]), &d),
            "O000/255/000 S255/000/255"
        );
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&[]), &d),
            "O255/000/000 S255/255/000"
        );
        t.configuration.stage_color = Some(ColorSchemeId::Default);
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&[]), &d),
            "O255/000/000 S255/000/000"
        );

        let mut d = DefaultTallyConfiguration::default();
        d.operator_color = ColorSchemeId::YellowPink;
        let t = patched();
        assert_eq!(
            cmd(&t, Some(&["channel"]), Some(&[]), &d),
            "O255/255/000 S255/000/000"
        );
    }

    #[test]
    fn show_preview_and_idle_flags() {
        let mut d = DefaultTallyConfiguration::default();
        d.stage_shows_preview = false;
        let mut t = patched();
        assert_eq!(
            cmd(&t, Some(&["other"]), Some(&["channel"]), &d),
            "O000/255/000 S000/000/000"
        );
        d.stage_shows_preview = true;
        t.configuration.stage_shows_preview = Some(false);
        assert_eq!(
            cmd(&t, Some(&["other"]), Some(&["channel"]), &d),
            "O000/255/000 S000/000/000"
        );

        let mut d = DefaultTallyConfiguration::default();
        d.operator_shows_idle = false;
        let t = patched();
        assert_eq!(
            cmd(&t, Some(&[]), Some(&[]), &d),
            "O000/000/000 S000/000/000"
        );
        let d = DefaultTallyConfiguration::default();
        let mut t = patched();
        t.configuration.operator_shows_idle = Some(false);
        assert_eq!(
            cmd(&t, Some(&[]), Some(&[]), &d),
            "O000/000/000 S000/000/000"
        );
    }

    #[test]
    fn invalid_commands_are_rejected() {
        assert_eq!(
            parse("this is an invalid command").unwrap_err().to_string(),
            "Received an invalid command: \"this is an invalid command\""
        );
        assert_eq!(
            parse("tally-ho Tally").unwrap_err().to_string(),
            "Received an invalid command: \"tally-ho Tally\""
        );
        assert_eq!(
            parse("log this invalid log").unwrap_err().to_string(),
            "Received an invalid command: \"log this invalid log\""
        );
    }

    #[test]
    fn parses_tally_ho() {
        assert_eq!(
            parse("tally-ho \"Tally Name\"").unwrap(),
            TallyCommand::TallyHo {
                tally_name: "Tally Name".into()
            }
        );
    }

    #[test]
    fn parses_log() {
        match parse("log \"Tally Name\" INFO \"Hello World\"").unwrap() {
            TallyCommand::Log { tally_name, log } => {
                assert_eq!(tally_name, "Tally Name");
                assert!(log.is_info());
                assert_eq!(log.message, "Hello World");
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse("log \"Tally Name\" UNKNOWN \"Hello World\"").unwrap() {
            TallyCommand::Log { log, .. } => assert!(log.is_error()),
            other => panic!("unexpected {other:?}"),
        }
        match parse("log \"T\" WARN \"\"").unwrap() {
            TallyCommand::Log { log, .. } => {
                assert!(log.is_warning());
                assert_eq!(log.message, "");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
