//! Keeps exactly one mixer connector running: the one that is selected in the
//! configuration. Restarts it when the selection or its settings change.

use super::connectors::{self, Connector};
use crate::domain::channel::default_channels;
use crate::events::Event;
use crate::hub::Hub;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, error, info};

struct Driver {
    hub: Arc<Hub>,
    current_id: Option<String>,
    current_settings: Option<Value>,
    connector: Option<Box<dyn Connector>>,
    /// Remembered so that an unknown id is only complained about once.
    rejected_id: Option<String>,
}

impl Driver {
    fn config_changed(&mut self) {
        let selection = self.hub.mixer_selection();
        if selection == self.rejected_id && selection.is_some() {
            return;
        }
        if selection != self.current_id {
            debug!("A different mixer was selected");
            self.change_mixer(selection);
        } else if let Some(id) = &self.current_id {
            if self.hub.mixer_settings_json(id) != self.current_settings {
                debug!("mixer connection is restarted, because settings were changed");
                self.change_mixer(selection);
            }
        }
    }

    fn change_mixer(&mut self, new_id: Option<String>) {
        let Some(id) = new_id else { return };
        if !self.hub.allowed_mixers().contains(&id.as_str()) {
            error!("Can not switch to unknown mixer with id {id}");
            self.rejected_id = Some(id);
            return;
        }
        self.rejected_id = None;

        if let Some(mut old) = self.connector.take() {
            old.disconnect();
            self.hub.notify_program_preview_changed(None, None);
            self.hub.notify_channels(Some(default_channels()));
        }
        self.hub.notify_mixer_is_disconnected();

        info!("Using mixer configuration \"{id}\"");
        let settings = self.hub.mixer_settings();
        let Some(mut connector) = connectors::create(&id, Arc::clone(&self.hub), &settings) else {
            error!("Someone(TM) forgot to implement the {id} mixer.");
            return;
        };
        self.current_settings = settings.to_json(&id);
        self.current_id = Some(id);
        connector.connect();
        self.connector = Some(connector);
    }
}

pub async fn run(hub: Arc<Hub>) {
    // subscribe first, so that no change is missed while connecting
    let mut events = hub.subscribe();
    let initial = hub.mixer_selection();
    let mut driver = Driver {
        hub,
        current_id: None,
        current_settings: None,
        connector: None,
        rejected_id: None,
    };
    driver.change_mixer(initial);

    loop {
        match events.recv().await {
            Ok(Event::ConfigChanged) | Err(RecvError::Lagged(_)) => driver.config_changed(),
            Ok(_) => {}
            Err(RecvError::Closed) => break,
        }
    }
    if let Some(mut connector) = driver.connector.take() {
        connector.disconnect();
    }
}
