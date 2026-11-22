//! Loads and saves the configuration file.

use crate::appconfig::AppConfig;
use crate::events::Event;
use crate::hub::Hub;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tracing::{error, warn};

/// Don't save instantly, but wait a bit to prevent too many writes in short succession.
const SAVE_DELAY: Duration = Duration::from_millis(500);

pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("CONFIG_FILE") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    PathBuf::from(home.unwrap_or_default()).join(".wifi-tally.json")
}

/// Loads the file into `config`. A missing or empty file is fine; a file with
/// broken content is an error, so that the hub does not silently overwrite it.
pub fn load(path: &Path, config: &mut AppConfig) -> Result<()> {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                "Configuration File {} does not exist. Using defaults.",
                path.display()
            );
            return Ok(());
        }
        Err(e) => return Err(e).with_context(|| format!("could not read {}", path.display())),
    };
    if raw.is_empty() {
        warn!(
            "Could not parse {}, because file is empty. Using defaults.",
            path.display()
        );
        return Ok(());
    }
    let data: Value = serde_json::from_slice(&raw)
        .with_context(|| format!("Error when parsing {}", path.display()))?;
    if !data.is_object() {
        bail!("Expected {} to contain a JSON object", path.display());
    }
    config.load_json(&data);
    Ok(())
}

pub fn save(path: &Path, config: &Value) -> Result<()> {
    let mut data = serde_json::Map::new();
    data.insert(
        "_warning".into(),
        json!("This file was automatically generated."),
    );
    data.insert(
        "_warning2".into(),
        json!("Do not edit it while the hub is running. Your changes will be lost."),
    );
    if let Value::Object(m) = config {
        data.extend(m.clone());
    }
    let mut text = String::new();
    {
        use serde::Serialize;
        let mut out = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
        let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
        Value::Object(data).serialize(&mut ser)?;
        text.push_str(&String::from_utf8(out)?);
    }
    // write to a temporary file first, so a crash can not leave a half-written configuration behind
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text).with_context(|| format!("could not write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

/// Saves the configuration (debounced) whenever it changes. Saves once more when `shutdown` resolves.
pub async fn run(hub: Arc<Hub>, path: PathBuf, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    let mut events = hub.subscribe();
    let write = |hub: &Hub| {
        if let Err(e) = save(&path, &hub.config_json()) {
            error!("error when saving file {}: {e:#}", path.display());
        }
    };
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(Event::ConfigChanged) | Err(RecvError::Lagged(_)) => {
                    // collect more changes before writing
                    tokio::time::sleep(SAVE_DELAY).await;
                    // everything that happened meanwhile is part of this write
                    while !matches!(events.try_recv(), Err(TryRecvError::Empty | TryRecvError::Closed)) {}
                    write(&hub);
                }
                Ok(_) => {}
                Err(RecvError::Closed) => break,
            },
            _ = shutdown.changed() => {
                write(&hub);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_uses_defaults() {
        let mut c = AppConfig::default();
        load(Path::new("/tmp/this/file/does/not/exist.json"), &mut c).unwrap();
        assert_eq!(c, AppConfig::default());
    }

    #[test]
    fn empty_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, "").unwrap();
        let mut c = AppConfig::default();
        load(&path, &mut c).unwrap();
        assert_eq!(c, AppConfig::default());
    }

    #[test]
    fn broken_content_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        for content in ["Hello World", "{\"mixer\": ", "42", "[]"] {
            std::fs::write(&path, content).unwrap();
            let mut c = AppConfig::default();
            assert!(load(&path, &mut c).is_err(), "{content}");
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        let mut c = AppConfig {
            mixer: Some("obs".into()),
            ..AppConfig::default()
        };
        c.mixers.obs.endpoint.ip = "10.0.0.7".into();
        save(&path, &c.to_json()).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"_warning\""));
        assert!(
            text.contains("\n\t\"mixer\": \"obs\""),
            "tab indented: {text}"
        );

        let mut loaded = AppConfig::default();
        load(&path, &mut loaded).unwrap();
        assert_eq!(loaded, c);
    }
}
