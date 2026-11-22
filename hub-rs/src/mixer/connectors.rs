//! Connections to video mixers.
//!
//! A connector reports what the mixer does through the [`Hub`]'s `notify_*` methods.

use super::atem::AtemConnector;
use super::config::{self, MixerSettings, MockConfig, TestConfig};
use super::vmix::VmixConnector;
use crate::hub::Hub;
use rand::Rng;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{error, info};

/// Stops a task when it is dropped.
pub struct TaskGuard(JoinHandle<()>);

impl TaskGuard {
    pub fn spawn(future: impl std::future::Future<Output = ()> + Send + 'static) -> Self {
        TaskGuard(tokio::spawn(future))
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub trait Connector: Send {
    fn connect(&mut self);
    fn disconnect(&mut self);
    fn is_connected(&self) -> bool;
}

/// Creates the connector for the mixer with the given id.
pub fn create(id: &str, hub: Arc<Hub>, settings: &MixerSettings) -> Option<Box<dyn Connector>> {
    Some(match id {
        config::NULL => Box::new(NullConnector { hub }),
        config::MOCK => Box::new(MockConnector {
            hub,
            config: settings.mock.clone(),
            task: None,
        }),
        config::TEST => Box::new(TestConnector {
            hub,
            config: settings.test.clone(),
            connected: false,
        }),
        config::ATEM => Box::new(AtemConnector::new(hub, settings.atem.clone())),
        config::VMIX => Box::new(VmixConnector::new(hub, settings.vmix.clone())),
        config::OBS | config::ROLAND_V8HD | config::ROLAND_V60HD => {
            Box::new(UnsupportedConnector {
                hub,
                id: id.to_string(),
            })
        }
        _ => return None,
    })
}

/// A connector that does not do anything.
struct NullConnector {
    hub: Arc<Hub>,
}

impl Connector for NullConnector {
    fn connect(&mut self) {
        info!("No video mixer connected.");
        self.hub.notify_program_preview_changed(None, None);
    }
    fn disconnect(&mut self) {}
    fn is_connected(&self) -> bool {
        false
    }
}

/// Stands in for mixers that the Rust hub can not talk to yet. The hub keeps
/// running, but tallies show the "unknown" state.
struct UnsupportedConnector {
    hub: Arc<Hub>,
    id: String,
}

impl Connector for UnsupportedConnector {
    fn connect(&mut self) {
        error!(
            "The \"{}\" mixer is not supported by the Rust hub yet. Tallies will show an unknown state.",
            self.id
        );
        self.hub.notify_program_preview_changed(None, None);
    }
    fn disconnect(&mut self) {}
    fn is_connected(&self) -> bool {
        false
    }
}

/// Simulates a mixer that switches to random programs and previews.
struct MockConnector {
    hub: Arc<Hub>,
    config: MockConfig,
    task: Option<JoinHandle<()>>,
}

impl Connector for MockConnector {
    fn connect(&mut self) {
        info!(
            "Simulating a mock video mixer with {} channels that changes programs every {}ms",
            self.config.channel_count, self.config.tick_time
        );
        self.hub.notify_mixer_is_connected();
        self.hub.notify_channels(Some(self.config.channels()));

        let hub = Arc::clone(&self.hub);
        let count = self.config.channel_count;
        let tick = Duration::from_secs_f64(self.config.tick_time / 1000.0);
        self.task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick);
            loop {
                interval.tick().await; // the first tick is immediate
                let (program, preview) = {
                    let mut rng = rand::rng();
                    (rng.random_range(0..=count), rng.random_range(0..=count))
                };
                hub.notify_program_preview_changed(
                    Some(vec![program.to_string()]),
                    Some(vec![preview.to_string()]),
                );
            }
        }));
    }

    fn disconnect(&mut self) {
        info!("Stopping mock video mixer");
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.hub.notify_mixer_is_disconnected();
    }

    fn is_connected(&self) -> bool {
        self.task.is_some()
    }
}

/// A mixer whose state is set from the outside; used by the end-to-end tests.
struct TestConnector {
    hub: Arc<Hub>,
    config: TestConfig,
    connected: bool,
}

impl Connector for TestConnector {
    fn connect(&mut self) {
        info!("starting mock server");
        self.connected = true;
        self.hub.notify_mixer_is_connected();
        self.hub.notify_channel_names(Some(4));
        self.hub.notify_program_preview_changed(
            self.config.programs.clone(),
            self.config.previews.clone(),
        );
    }
    fn disconnect(&mut self) {
        self.connected = false;
        self.hub.notify_mixer_is_disconnected();
    }
    fn is_connected(&self) -> bool {
        self.connected
    }
}
