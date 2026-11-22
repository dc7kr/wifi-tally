//! The heart of the hub: owns all state and keeps tallies in sync with the mixer.
//!
//! All state lives behind one mutex that is never held across an `await`.
//! Whatever happens is announced as an [`Event`] on a broadcast channel; the
//! only side effects performed directly are datagrams to UDP tallies, because
//! they must follow state changes immediately.

use crate::appconfig::AppConfig;
use crate::command;
use crate::domain::channel::Channel;
use crate::domain::log::{Log, Severity};
use crate::domain::tally::{ConnectionState, Tally, TallyKind, TallyType};
use crate::domain::tally_config::{DefaultTallyConfiguration, TallyConfiguration};
use crate::events::Event;
use crate::mixer;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

#[derive(Clone, Debug)]
pub struct HubOptions {
    pub http_port: u16,
    pub tally_port: u16,
    /// Development mode enables the mock mixer.
    pub is_dev: bool,
    /// Test mode enables the scripted test mixer and skips persistence.
    pub is_test: bool,
    pub highlight_time: Duration,
    /// The more keep alives are sent, the less likely a tally shows a wrong
    /// state, but the more packets travel through the network.
    pub keepalives_per_second: u32,
}

impl Default for HubOptions {
    fn default() -> Self {
        HubOptions {
            http_port: 3000,
            tally_port: 7411,
            is_dev: false,
            is_test: false,
            highlight_time: Duration::from_millis(1000),
            keepalives_per_second: 10,
        }
    }
}

struct WebClient {
    /// socket.io session id
    sid: String,
    address: String,
}

struct Inner {
    config: AppConfig,
    programs: Option<Vec<String>>,
    previews: Option<Vec<String>>,
    /// `None` until the mixer reported anything.
    mixer_connected: Option<bool>,
    timeout_disconnected: Duration,
    timeout_missing: Duration,
    last_report: HashMap<String, Instant>,
    web_clients: HashMap<String, Vec<WebClient>>,
}

/// A datagram for a tally. The UDP driver sends it.
pub type UdpMessage = (SocketAddr, String);

pub struct Hub {
    inner: Mutex<Inner>,
    events: broadcast::Sender<Event>,
    udp: OnceLock<mpsc::UnboundedSender<UdpMessage>>,
    pub options: HubOptions,
}

fn find(g: &Inner, name: &str, ty: TallyType) -> Option<usize> {
    g.config
        .tallies
        .iter()
        .position(|t| t.name == name && t.tally_type() == ty)
}

impl Hub {
    pub fn new(options: HubOptions, config: AppConfig) -> Arc<Hub> {
        let (events, _) = broadcast::channel(4096);
        Arc::new(Hub {
            inner: Mutex::new(Inner {
                config,
                programs: None,
                previews: None,
                mixer_connected: None,
                timeout_disconnected: Duration::from_millis(30_000),
                timeout_missing: Duration::from_millis(3_000),
                last_report: HashMap::new(),
                web_clients: HashMap::new(),
            }),
            events,
            udp: OnceLock::new(),
            options,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, event: Event) {
        // An error only means that nobody listens (yet).
        let _ = self.events.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Where datagrams for tallies go to. Without one, tallies get nothing.
    pub fn set_udp_outbox(&self, outbox: mpsc::UnboundedSender<UdpMessage>) {
        if self.udp.set(outbox).is_err() {
            warn!("UDP outbox was already set");
        }
    }

    // ------------------------------------------------------------------
    // timeouts
    // ------------------------------------------------------------------

    pub fn set_tally_timeouts(&self, missing: Duration, disconnected: Duration) {
        let min = Duration::from_millis(1000);
        assert!(
            missing >= min && disconnected >= min,
            "timeouts must be at least 1000ms"
        );
        let mut g = self.lock();
        g.timeout_missing = missing;
        g.timeout_disconnected = disconnected;
    }

    // ------------------------------------------------------------------
    // read access
    // ------------------------------------------------------------------

    pub fn config_json(&self) -> Value {
        self.lock().config.to_json()
    }

    pub fn mixer_selection(&self) -> Option<String> {
        self.lock().config.mixer.clone()
    }

    pub fn is_mixer_connected(&self) -> bool {
        self.lock().mixer_connected == Some(true)
    }

    pub fn programs(&self) -> Option<Vec<String>> {
        self.lock().programs.clone()
    }

    pub fn previews(&self) -> Option<Vec<String>> {
        self.lock().previews.clone()
    }

    pub fn channels(&self) -> Vec<Channel> {
        self.lock().config.channels.clone()
    }

    pub fn channels_json(&self) -> Value {
        Value::Array(
            self.lock()
                .config
                .channels
                .iter()
                .map(Channel::to_json)
                .collect(),
        )
    }

    pub fn mixer_settings_json(&self, id: &str) -> Option<Value> {
        self.lock().config.mixers.to_json(id)
    }

    pub fn mixer_settings(&self) -> mixer::config::MixerSettings {
        self.lock().config.mixers.clone()
    }

    pub fn tally_defaults_json(&self) -> Value {
        self.lock().config.tally_defaults.to_json()
    }

    pub fn allowed_mixers(&self) -> Vec<&'static str> {
        mixer::allowed_mixers(self.options.is_dev, self.options.is_test)
    }

    pub fn tallies_json(&self) -> Value {
        Value::Array(
            self.lock()
                .config
                .tallies
                .iter()
                .map(Tally::to_json)
                .collect(),
        )
    }

    pub fn tally(&self, name: &str, ty: TallyType) -> Option<Tally> {
        let g = self.lock();
        find(&g, name, ty).map(|i| g.config.tallies[i].clone())
    }

    /// `[{"tallyId": ..., "logs": [...]}]`
    pub fn tally_logs_json(&self) -> Value {
        let g = self.lock();
        g.config
            .tallies
            .iter()
            .map(|t| json!({"tallyId": t.id(), "logs": t.logs.iter().map(Log::to_json).collect::<Vec<_>>()}))
            .collect()
    }

    // ------------------------------------------------------------------
    // configuration
    // ------------------------------------------------------------------

    pub fn set_mixer_settings(&self, id: &str, data: &Value) {
        {
            let mut g = self.lock();
            g.config.mixers.load(id, data);
        }
        self.emit(Event::ConfigChanged);
        self.emit(Event::MixerSettingsChanged(id.to_string()));
    }

    pub fn set_mixer_selection(&self, id: &str) {
        self.lock().config.mixer = Some(id.to_string());
        self.emit(Event::ConfigChanged);
        self.emit(Event::MixerSelectionChanged);
    }

    pub fn set_tally_defaults(&self, data: &Value) {
        {
            let mut g = self.lock();
            let mut defaults = DefaultTallyConfiguration::default();
            defaults.load_json(data);
            g.config.tally_defaults = defaults;
            self.push_all(&g);
        }
        self.emit(Event::ConfigChanged);
        self.emit(Event::TallyDefaultsChanged);
    }

    pub fn set_channels(&self, channels: Vec<Channel>) {
        self.lock().config.channels = channels;
        self.emit(Event::ConfigChanged);
        self.emit(Event::ChannelsChanged);
    }

    // ------------------------------------------------------------------
    // what the mixer connectors report
    // ------------------------------------------------------------------

    pub fn notify_program_preview_changed(
        &self,
        programs: Option<Vec<String>>,
        previews: Option<Vec<String>>,
    ) {
        let mut g = self.lock();
        let changed = g.programs != programs || g.previews != previews;
        if changed {
            g.programs = programs;
            g.previews = previews;
            self.program_changed(&g);
        }
    }

    pub fn notify_program_changed(&self, programs: Option<Vec<String>>) {
        let mut g = self.lock();
        if g.programs != programs {
            g.programs = programs;
            self.program_changed(&g);
        }
    }

    pub fn notify_preview_changed(&self, previews: Option<Vec<String>>) {
        let mut g = self.lock();
        if g.previews != previews {
            g.previews = previews;
            self.program_changed(&g);
        }
    }

    fn program_changed(&self, g: &Inner) {
        info!(
            "Program/Preview was changed to {:?} {:?}",
            g.programs, g.previews
        );
        self.emit(Event::ProgramChanged);
        self.push_all(g);
    }

    /// Announces channels "1" to `count`. `None` means the mixer has no channels.
    pub fn notify_channel_names(&self, count: Option<usize>) {
        let channels = count.map(|c| {
            (1..=c)
                .map(|i| Channel::new(i.to_string(), None))
                .collect::<Vec<_>>()
        });
        self.notify_channels(channels);
    }

    pub fn notify_channels(&self, channels: Option<Vec<Channel>>) {
        let channels = channels.unwrap_or_default();
        let changed = self.lock().config.channels != channels;
        if changed {
            self.set_channels(channels);
        }
    }

    pub fn notify_mixer_is_connected(&self) {
        if self.set_mixer_connected(true) {
            self.emit(Event::MixerConnected);
        }
    }

    pub fn notify_mixer_is_disconnected(&self) {
        if self.set_mixer_connected(false) {
            self.emit(Event::MixerDisconnected);
        }
    }

    fn set_mixer_connected(&self, connected: bool) -> bool {
        let mut g = self.lock();
        if g.mixer_connected == Some(connected) {
            false
        } else {
            g.mixer_connected = Some(connected);
            true
        }
    }

    // ------------------------------------------------------------------
    // pushing state to tallies
    // ------------------------------------------------------------------

    fn push_all(&self, g: &Inner) {
        for tally in &g.config.tallies {
            self.push_tally(g, tally);
        }
    }

    fn push_tally(&self, g: &Inner, tally: &Tally) {
        let programs = g.programs.as_deref();
        let previews = g.previews.as_deref();
        match &tally.kind {
            TallyKind::Udp { address, port, .. } => {
                if !tally.is_active() {
                    return;
                }
                let (Some(address), Some(port), Some(outbox)) = (address, port, self.udp.get())
                else {
                    return;
                };
                let Ok(ip) = address.parse() else { return };
                let command = command::create_state_command(
                    tally,
                    programs,
                    previews,
                    &g.config.tally_defaults,
                );
                if outbox.send((SocketAddr::new(ip, *port), command)).is_err() {
                    debug!(
                        "UDP driver is gone, could not send to tally \"{}\"",
                        tally.name
                    );
                }
            }
            TallyKind::Web { connected_clients } => {
                if connected_clients.is_empty() {
                    return;
                }
                self.emit(Event::WebTallyState {
                    tally_name: tally.name.clone(),
                    tally: tally.to_json(),
                    command: command::get_state(tally, programs, previews),
                });
            }
        }
    }

    /// Sends a keep alive to every UDP tally. It shows the tally that the hub
    /// is still there and compensates for lost packets.
    pub fn send_keepalives(&self) {
        let g = self.lock();
        for tally in g
            .config
            .tallies
            .iter()
            .filter(|t| t.tally_type() == TallyType::Udp)
        {
            self.push_tally(&g, tally);
        }
    }

    // ------------------------------------------------------------------
    // tally container
    // ------------------------------------------------------------------

    fn get_or_create(&self, g: &mut Inner, name: &str, ty: TallyType) -> usize {
        if let Some(i) = find(g, name, ty) {
            return i;
        }
        g.config.tallies.push(Tally::new(ty, name));
        let i = g.config.tallies.len() - 1;
        self.push_tally(g, &g.config.tallies[i]);
        debug!("Tally \"{name}\" created");
        self.emit(Event::ConfigChanged);
        i
    }

    /// Changes a tally, tells everyone about it and persists it if necessary.
    fn modify(&self, g: &mut Inner, idx: usize, change: impl FnOnce(&mut Tally)) {
        let before = g.config.tallies[idx].to_json_for_save();
        change(&mut g.config.tallies[idx]);
        let tally = &g.config.tallies[idx];
        self.push_tally(g, tally);
        self.emit(Event::TalliesChanged);
        if tally.to_json_for_save() != before {
            self.emit(Event::ConfigChanged);
        }
    }

    pub fn remove(&self, name: &str, ty: TallyType) {
        let mut g = self.lock();
        if let Some(i) = find(&g, name, ty) {
            g.config.tallies.remove(i);
            debug!("Removed tally \"{name}\"");
            self.emit(Event::ConfigChanged);
            self.emit(Event::TalliesChanged);
        }
    }

    pub fn patch(&self, name: &str, ty: TallyType, channel_id: Option<String>) {
        let mut g = self.lock();
        match find(&g, name, ty) {
            Some(i) => {
                let channel_id = channel_id.filter(|c| !c.is_empty());
                debug!("Tally \"{name}\" patched to {channel_id:?}");
                self.modify(&mut g, i, |t| t.channel_id = channel_id);
            }
            None => warn!("Can not patch unknown tally named \"{name}\""),
        }
    }

    pub fn update_settings(&self, name: &str, ty: TallyType, settings: TallyConfiguration) {
        let mut g = self.lock();
        match find(&g, name, ty) {
            Some(i) => {
                self.modify(&mut g, i, |t| t.configuration = settings);
                debug!("Tally \"{name}\" updated");
            }
            None => warn!("Can not update settings for unknown tally named \"{name}\""),
        }
    }

    /// Lets the tally flash for a moment so one can find it.
    pub fn highlight(self: &Arc<Self>, name: &str, ty: TallyType) {
        {
            let mut g = self.lock();
            let Some(i) = find(&g, name, ty) else {
                warn!("Can not highlight unknown tally named \"{name}\"");
                return;
            };
            debug!("Tally \"{name}\" highlighted");
            self.modify(&mut g, i, |t| t.highlight = true);
        }
        let hub = Arc::clone(self);
        let name = name.to_string();
        let time = self.options.highlight_time;
        tokio::spawn(async move {
            tokio::time::sleep(time).await;
            hub.de_highlight(&name, ty);
        });
    }

    fn de_highlight(&self, name: &str, ty: TallyType) {
        let mut g = self.lock();
        match find(&g, name, ty) {
            Some(i) => {
                debug!("Tally \"{name}\" de-highlighted");
                self.modify(&mut g, i, |t| t.highlight = false);
            }
            None => warn!("Can not unhighlight unknown tally named \"{name}\""),
        }
    }

    pub fn add_log(&self, name: &str, ty: TallyType, log: Log) {
        let mut g = self.lock();
        match find(&g, name, ty) {
            Some(i) => self.add_log_to(&mut g, i, log),
            None => warn!("Can not log for unknown tally named \"{name}\""),
        }
    }

    fn add_log_to(&self, g: &mut Inner, idx: usize, log: Log) {
        let tally = &mut g.config.tallies[idx];
        match log.severity {
            Severity::Error => error!("{}: {}", tally.name, log.message),
            Severity::Warning => warn!("{}: {}", tally.name, log.message),
            _ => info!("{}: {}", tally.name, log.message),
        }
        tally.logs.push(log.clone());
        let tally_id = tally.id();
        self.emit(Event::TallyLogged { tally_id, log });
    }

    // ------------------------------------------------------------------
    // UDP tallies
    // ------------------------------------------------------------------

    /// A tally has sent us a message: it is there.
    pub fn udp_tally_reported(&self, name: &str, address: &str, port: u16) {
        let mut g = self.lock();
        self.udp_reported_locked(&mut g, name, address, port);
    }

    fn udp_reported_locked(&self, g: &mut Inner, name: &str, address: &str, port: u16) -> usize {
        g.last_report.insert(name.to_string(), Instant::now());
        let idx = self.get_or_create(g, name, TallyType::Udp);
        let new = TallyKind::Udp {
            address: Some(address.to_string()),
            port: Some(port),
            state: ConnectionState::Connected,
        };
        if g.config.tallies[idx].kind != new {
            self.modify(g, idx, |t| t.kind = new);
        }
        idx
    }

    pub fn udp_tally_logged(&self, name: &str, address: &str, port: u16, log: Log) {
        let mut g = self.lock();
        let idx = self.udp_reported_locked(&mut g, name, address, port);
        self.add_log_to(&mut g, idx, log);
    }

    /// Notices tallies that stopped reporting. Called periodically.
    pub fn check_udp_timeouts(&self) {
        let now = Instant::now();
        let mut g = self.lock();
        for idx in 0..g.config.tallies.len() {
            let tally = &g.config.tallies[idx];
            let TallyKind::Udp { state, .. } = tally.kind else {
                continue;
            };
            let Some(last) = g.last_report.get(&tally.name).copied() else {
                // never reported since the hub started
                if state != ConnectionState::Disconnected {
                    set_udp_state(&mut g.config.tallies[idx], ConnectionState::Disconnected);
                }
                continue;
            };
            let diff = now.saturating_duration_since(last);
            let (new_state, message) = if diff > g.timeout_disconnected {
                (
                    ConnectionState::Disconnected,
                    format!(
                        "Tally got disconnected after not reporting for {}ms",
                        diff.as_millis()
                    ),
                )
            } else if diff > g.timeout_missing {
                (
                    ConnectionState::Missing,
                    format!(
                        "Tally got missing. It has not reported for {}ms",
                        diff.as_millis()
                    ),
                )
            } else {
                continue;
            };
            if state != new_state {
                self.modify(&mut g, idx, |t| set_udp_state(t, new_state));
                self.add_log_to(&mut g, idx, Log::new(Severity::Status, message));
            }
        }
    }

    // ------------------------------------------------------------------
    // web tallies
    // ------------------------------------------------------------------

    /// Creates a web tally (or re-patches an existing one).
    pub fn web_create(&self, name: &str, channel_id: Option<String>) {
        let mut g = self.lock();
        let idx = self.get_or_create(&mut g, name, TallyType::Web);
        let channel_id = channel_id.filter(|c| !c.is_empty());
        self.modify(&mut g, idx, |t| t.channel_id = channel_id);
        debug!("Tally \"{name}\" updated");
    }

    /// Returns `false` if there is no such web tally.
    pub fn web_subscribe(&self, name: &str, sid: &str, address: &str) -> bool {
        let mut g = self.lock();
        if g.web_clients
            .get(name)
            .is_some_and(|c| c.iter().any(|c| c.sid == sid))
        {
            warn!("socket {sid} is already subscribed to {name}");
            return true;
        }
        let Some(idx) = find(&g, name, TallyType::Web) else {
            error!("can not subscribe to unknown web tally \"{name}\"");
            return false;
        };
        debug!("Connected {name} from {sid}");
        g.web_clients
            .entry(name.to_string())
            .or_default()
            .push(WebClient {
                sid: sid.to_string(),
                address: address.to_string(),
            });
        self.update_web_connections(&mut g, name, idx);
        self.add_log_to(
            &mut g,
            idx,
            Log::new(
                Severity::Status,
                format!("A new browser connected from {address}."),
            ),
        );
        true
    }

    pub fn web_unsubscribe(&self, name: &str, sid: &str) {
        let mut g = self.lock();
        self.web_unsubscribe_locked(&mut g, name, sid);
    }

    /// Removes a socket from all tallies it was watching, e.g. because it disconnected.
    pub fn web_unsubscribe_all(&self, sid: &str) {
        let mut g = self.lock();
        let names: Vec<String> = g
            .web_clients
            .iter()
            .filter(|(_, c)| c.iter().any(|c| c.sid == sid))
            .map(|(n, _)| n.clone())
            .collect();
        for name in names {
            self.web_unsubscribe_locked(&mut g, &name, sid);
        }
    }

    fn web_unsubscribe_locked(&self, g: &mut Inner, name: &str, sid: &str) {
        let Some(clients) = g.web_clients.get_mut(name) else {
            return;
        };
        let Some(pos) = clients.iter().position(|c| c.sid == sid) else {
            return;
        };
        let gone = clients.remove(pos);
        debug!("Disconnected {name} from {sid}");
        let Some(idx) = find(g, name, TallyType::Web) else {
            return;
        };
        self.update_web_connections(g, name, idx);

        let remaining: Vec<String> = g.web_clients[name]
            .iter()
            .map(|c| c.address.clone())
            .collect();
        let mut line = format!("A browser disconnected from {}. ", gone.address);
        match remaining.len() {
            0 => line.push_str("The Tally is disconnected."),
            1 => line.push_str(&format!(
                "There is still one client connected from {}.",
                remaining[0]
            )),
            n => line.push_str(&format!(
                "There are still {n} clients connected from {}.",
                remaining.join(",")
            )),
        }
        self.add_log_to(g, idx, Log::new(Severity::Status, line));
    }

    fn update_web_connections(&self, g: &mut Inner, name: &str, idx: usize) {
        let addresses: Vec<String> = g
            .web_clients
            .get(name)
            .map(|c| c.iter().map(|c| c.address.clone()).collect())
            .unwrap_or_default();
        self.modify(g, idx, |t| {
            if let TallyKind::Web { connected_clients } = &mut t.kind {
                *connected_clients = addresses;
            }
        });
    }
}

fn set_udp_state(tally: &mut Tally, new_state: ConnectionState) {
    if let TallyKind::Udp { state, .. } = &mut tally.kind {
        *state = new_state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::StateCommand;
    use tokio::sync::broadcast::error::TryRecvError;

    fn hub() -> Arc<Hub> {
        Hub::new(HubOptions::default(), AppConfig::default())
    }

    fn drain(rx: &mut broadcast::Receiver<Event>) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(e) => events.push(e),
                Err(TryRecvError::Empty) => return events,
                Err(e) => panic!("{e}"),
            }
        }
    }

    fn count(events: &[Event], pred: impl Fn(&Event) -> bool) -> usize {
        events.iter().filter(|e| pred(e)).count()
    }

    fn strs(v: &[&str]) -> Option<Vec<String>> {
        Some(v.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn container_writes_changes_to_the_configuration() {
        let hub = hub();
        assert!(hub.config_json()["tallies"].as_array().unwrap().is_empty());

        hub.web_create("Foobar", None);
        let tallies = hub.config_json()["tallies"].clone();
        assert_eq!(tallies, json!([{"name": "Foobar", "type": "web"}]));

        hub.patch("Foobar", TallyType::Web, Some("42".into()));
        assert_eq!(
            hub.config_json()["tallies"],
            json!([{"name": "Foobar", "type": "web", "channelId": "42"}])
        );

        hub.remove("Foobar", TallyType::Web);
        assert!(hub.config_json()["tallies"].as_array().unwrap().is_empty());
    }

    #[test]
    fn only_persisted_changes_request_a_save() {
        let hub = hub();
        hub.web_create("w", None);
        let mut rx = hub.subscribe();

        hub.patch("w", TallyType::Web, Some("1".into()));
        let events = drain(&mut rx);
        assert_eq!(count(&events, |e| matches!(e, Event::ConfigChanged)), 1);
        assert_eq!(count(&events, |e| matches!(e, Event::TalliesChanged)), 1);

        // patching to the same channel changes nothing that is saved
        hub.patch("w", TallyType::Web, Some("1".into()));
        let events = drain(&mut rx);
        assert_eq!(count(&events, |e| matches!(e, Event::ConfigChanged)), 0);
        assert_eq!(count(&events, |e| matches!(e, Event::TalliesChanged)), 1);
    }

    #[test]
    fn unknown_tallies_are_ignored() {
        let hub = hub();
        let mut rx = hub.subscribe();
        hub.patch("nope", TallyType::Udp, Some("1".into()));
        hub.remove("nope", TallyType::Udp);
        hub.add_log("nope", TallyType::Udp, Log::new(Severity::Info, "x"));
        hub.update_settings("nope", TallyType::Udp, TallyConfiguration::default());
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn same_name_can_exist_as_udp_and_web_tally() {
        let hub = hub();
        hub.udp_tally_reported("Cam", "10.0.0.2", 1111);
        hub.web_create("Cam", Some("1".into()));
        assert_eq!(hub.tallies_json().as_array().unwrap().len(), 2);
        hub.remove("Cam", TallyType::Udp);
        assert!(hub.tally("Cam", TallyType::Web).is_some());
    }

    #[test]
    fn program_changes_are_debounced() {
        let hub = hub();
        let mut rx = hub.subscribe();
        let changed = |rx: &mut broadcast::Receiver<Event>| {
            count(&drain(rx), |e| matches!(e, Event::ProgramChanged))
        };

        hub.notify_program_preview_changed(strs(&["1"]), strs(&["2"]));
        assert_eq!(changed(&mut rx), 1);
        assert_eq!(hub.programs(), strs(&["1"]));
        assert_eq!(hub.previews(), strs(&["2"]));

        hub.notify_program_preview_changed(strs(&["1"]), strs(&["2"]));
        assert_eq!(changed(&mut rx), 0);
        hub.notify_program_changed(strs(&["1"]));
        hub.notify_preview_changed(strs(&["2"]));
        assert_eq!(changed(&mut rx), 0);

        hub.notify_preview_changed(None);
        assert_eq!(changed(&mut rx), 1);
        hub.notify_program_changed(strs(&["3"]));
        assert_eq!(changed(&mut rx), 1);
    }

    #[test]
    fn channel_names_are_debounced() {
        let hub = hub();
        let mut rx = hub.subscribe();
        hub.notify_channel_names(Some(3));
        let events = drain(&mut rx);
        assert_eq!(count(&events, |e| matches!(e, Event::ChannelsChanged)), 1);
        assert_eq!(hub.channels().len(), 3);

        hub.notify_channel_names(Some(3));
        assert_eq!(
            count(&drain(&mut rx), |e| matches!(e, Event::ChannelsChanged)),
            0
        );

        hub.notify_channel_names(None);
        assert!(hub.channels().is_empty());
    }

    #[test]
    fn mixer_connection_is_debounced() {
        let hub = hub();
        let mut rx = hub.subscribe();
        assert!(!hub.is_mixer_connected());
        hub.notify_mixer_is_connected();
        hub.notify_mixer_is_connected();
        assert!(hub.is_mixer_connected());
        hub.notify_mixer_is_disconnected();
        hub.notify_mixer_is_disconnected();
        let events = drain(&mut rx);
        assert_eq!(count(&events, |e| matches!(e, Event::MixerConnected)), 1);
        assert_eq!(count(&events, |e| matches!(e, Event::MixerDisconnected)), 1);
    }

    #[test]
    fn udp_tallies_receive_their_state() {
        let hub = hub();
        let (outbox, mut sent) = mpsc::unbounded_channel();
        hub.set_udp_outbox(outbox);
        let mut next = || {
            let (to, command) = sent.try_recv().expect("nothing was sent");
            assert_eq!(to, "10.0.0.2:7000".parse().unwrap());
            command
        };

        hub.notify_channel_names(Some(2));
        hub.udp_tally_reported("Cam", "10.0.0.2", 7000);
        assert_eq!(next(), "O000/001/000 S000/000/000"); // not patched: idle

        hub.patch("Cam", TallyType::Udp, Some("1".into()));
        assert_eq!(next(), "O000/000/255 S000/000/000 0x80 250"); // mixer unknown

        hub.notify_program_preview_changed(strs(&["1"]), strs(&["2"]));
        assert_eq!(next(), "O255/000/000 S255/000/000");
        hub.notify_program_preview_changed(strs(&["2"]), strs(&["1"]));
        assert_eq!(next(), "O000/255/000 S000/255/000");

        hub.send_keepalives();
        assert_eq!(next(), "O000/255/000 S000/255/000");

        hub.set_tally_defaults(&json!({"stBrightness": 50}));
        assert_eq!(next(), "O000/255/000 S000/128/000");

        assert!(sent.try_recv().is_err(), "exactly one message per event");
    }

    #[tokio::test(start_paused = true)]
    async fn udp_tallies_go_missing_and_disconnect() {
        let hub = hub();
        let mut rx = hub.subscribe();
        hub.udp_tally_reported("Cam", "10.0.0.2", 7000);
        let state = |hub: &Hub| match hub.tally("Cam", TallyType::Udp).unwrap().kind {
            TallyKind::Udp { state, .. } => state,
            _ => unreachable!(),
        };
        assert_eq!(state(&hub), ConnectionState::Connected);
        drain(&mut rx);

        tokio::time::advance(Duration::from_millis(2900)).await;
        hub.check_udp_timeouts();
        assert_eq!(state(&hub), ConnectionState::Connected);

        tokio::time::advance(Duration::from_millis(200)).await;
        hub.check_udp_timeouts();
        assert_eq!(state(&hub), ConnectionState::Missing);
        hub.check_udp_timeouts(); // nothing new
        let logs: Vec<_> = drain(&mut rx)
            .into_iter()
            .filter_map(|e| match e {
                Event::TallyLogged { tally_id, log } => Some((tally_id, log.message)),
                _ => None,
            })
            .collect();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].0, "udp-Cam");
        assert!(logs[0]
            .1
            .starts_with("Tally got missing. It has not reported for 3100ms"));

        tokio::time::advance(Duration::from_secs(28)).await;
        hub.check_udp_timeouts();
        assert_eq!(state(&hub), ConnectionState::Disconnected);

        // and it comes back
        hub.udp_tally_reported("Cam", "10.0.0.2", 7000);
        assert_eq!(state(&hub), ConnectionState::Connected);
        assert_eq!(hub.tally("Cam", TallyType::Udp).unwrap().logs.len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn highlight_wears_off() {
        let hub = hub();
        hub.web_create("w", None);
        hub.highlight("w", TallyType::Web);
        assert!(hub.tally("w", TallyType::Web).unwrap().highlight);
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!hub.tally("w", TallyType::Web).unwrap().highlight);
    }

    #[test]
    fn udp_tally_logs() {
        let hub = hub();
        hub.udp_tally_logged("Cam", "10.0.0.2", 7000, Log::new(Severity::Warning, "hot"));
        let t = hub.tally("Cam", TallyType::Udp).unwrap();
        assert!(t.is_connected());
        assert_eq!(t.logs.len(), 1);
        assert_eq!(hub.tally_logs_json()[0]["logs"][0]["message"], json!("hot"));
    }

    #[test]
    fn web_tallies_track_their_browsers() {
        let hub = hub();
        let mut rx = hub.subscribe();
        assert!(!hub.web_subscribe("w", "s1", "1.1.1.1"), "unknown tally");

        hub.web_create("w", Some("1".into()));
        hub.notify_program_preview_changed(strs(&["1"]), None);
        drain(&mut rx);

        assert!(hub.web_subscribe("w", "s1", "1.1.1.1"));
        // the new browser gets the current state right away
        let events = drain(&mut rx);
        assert!(events.iter().any(|e| matches!(e,
            Event::WebTallyState { tally_name, command: StateCommand::OnAir, .. } if tally_name == "w")));
        assert!(hub.tally("w", TallyType::Web).unwrap().is_connected());

        assert!(hub.web_subscribe("w", "s2", "2.2.2.2"));
        hub.web_unsubscribe("w", "s1");
        hub.web_unsubscribe("w", "s1"); // twice is fine
        hub.web_unsubscribe_all("s2");
        let t = hub.tally("w", TallyType::Web).unwrap();
        assert!(!t.is_connected());
        let messages: Vec<_> = t.logs.iter().map(|l| l.message.clone()).collect();
        assert_eq!(
            messages,
            vec![
                "A new browser connected from 1.1.1.1.",
                "A new browser connected from 2.2.2.2.",
                "A browser disconnected from 1.1.1.1. There is still one client connected from 2.2.2.2.",
                "A browser disconnected from 2.2.2.2. The Tally is disconnected.",
            ]
        );
    }

    #[test]
    fn web_tally_state_is_only_sent_while_a_browser_watches() {
        let hub = hub();
        hub.web_create("w", Some("1".into()));
        let mut rx = hub.subscribe();
        hub.notify_program_preview_changed(strs(&["1"]), None);
        assert_eq!(
            count(&drain(&mut rx), |e| matches!(
                e,
                Event::WebTallyState { .. }
            )),
            0
        );
    }

    #[test]
    fn mixer_settings_changes_are_announced() {
        let hub = hub();
        let mut rx = hub.subscribe();
        hub.set_mixer_settings("atem", &json!({"ip": "10.0.0.9"}));
        hub.set_mixer_selection("atem");
        let events = drain(&mut rx);
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::MixerSettingsChanged(id) if id == "atem")));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::MixerSelectionChanged)));
        assert_eq!(hub.mixer_selection().as_deref(), Some("atem"));
        assert_eq!(
            hub.mixer_settings_json("atem").unwrap()["ip"],
            json!("10.0.0.9")
        );
    }
}
