use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

/// The numeric values are part of the socket protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info = 0,
    Warning = 1,
    Error = 2,
    Status = 3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Log {
    pub severity: Severity,
    pub date_time: DateTime<Utc>,
    pub message: String,
}

impl Log {
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Log {
            severity,
            date_time: Utc::now(),
            message: message.into(),
        }
    }

    /// Parses the severity names tallies send ("INFO", "WARN", "ERROR").
    pub fn severity_from_str(s: &str) -> Severity {
        match s {
            "INFO" => Severity::Info,
            "WARN" => Severity::Warning,
            "ERROR" => Severity::Error,
            _ => Severity::Status,
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
    pub fn is_warning(&self) -> bool {
        self.severity == Severity::Warning
    }
    pub fn is_info(&self) -> bool {
        self.severity == Severity::Info
    }

    pub fn to_json(&self) -> Value {
        json!({
            "date": self.date_time.to_rfc3339_opts(SecondsFormat::Millis, true),
            "severity": self.severity as u8,
            "message": self.message,
        })
    }
}
