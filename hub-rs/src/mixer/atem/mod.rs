//! Blackmagic ATEM switchers.
//!
//! The Node hub uses the `atem-connection` library; there is no equivalent for
//! Rust, so the little that is needed is implemented here: the UDP protocol
//! ([`packet`]), the state of the mixer and the tally logic ([`state`]) and the
//! connection handling in this file.
//!
//! The hub only listens. It never sends commands to the ATEM, so there is no
//! need to retransmit anything.

pub mod packet;
pub mod state;

use super::config::Endpoint;
use super::connectors::{Connector, TaskGuard};
use crate::hub::Hub;
use packet::*;
use state::{Mode, State};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{lookup_host, UdpSocket};
use tokio::time::{interval, sleep, sleep_until, Instant, MissedTickBehavior};
use tracing::{debug, error, info, warn};

/// The ATEM talks to us regularly. If it does not, it is gone.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Acknowledgements are combined for a moment, because the ATEM sends hundreds of packets at the start.
const ACK_DELAY: Duration = Duration::from_millis(5);
const MAX_PACKETS_PER_ACK: u32 = 16;

/// The state of the conversation with the ATEM, independent of the network.
#[derive(Default)]
pub struct Session {
    session_id: u16,
    /// the ATEM answered our hello
    established: bool,
    last_received_id: u16,
    unacked: u32,
    pub state: State,
}

/// What to do after a datagram was handled.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// An acknowledgement must be sent right now.
    pub ack_now: bool,
    /// An acknowledgement should be sent soon.
    pub ack_later: bool,
    /// The state could have changed.
    pub changed: bool,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_established(&self) -> bool {
        self.established
    }

    pub fn ack_packet(&self) -> [u8; HEADER_LENGTH] {
        ack(self.session_id, self.last_received_id)
    }

    pub fn handle(&mut self, data: &[u8]) -> Outcome {
        let mut outcome = Outcome::default();
        let Some(header) = parse_header(data) else {
            return outcome;
        };
        self.session_id = header.session_id;

        if header.flags & FLAG_NEW_SESSION != 0 {
            // the answer to our hello
            self.established = true;
            self.last_received_id = header.packet_id;
            self.unacked = 0;
            outcome.ack_now = true;
            return outcome;
        }
        if !self.established || header.flags & FLAG_ACK_REQUEST == 0 {
            return outcome;
        }

        if header.packet_id == (self.last_received_id + 1) % PACKET_ID_RANGE {
            self.last_received_id = header.packet_id;
            self.unacked += 1;
            if self.unacked >= MAX_PACKETS_PER_ACK {
                self.unacked = 0;
                outcome.ack_now = true;
            } else {
                outcome.ack_later = true;
            }
            for (name, body) in commands(&data[HEADER_LENGTH..]) {
                if self.state.apply(&name, body).is_none() {
                    debug!("Could not read the \"{name}\" command of the ATEM");
                }
                outcome.changed = true;
            }
        } else if is_covered_by_ack(self.last_received_id, header.packet_id) {
            // something we already have: the ATEM missed our acknowledgement
            outcome.ack_later = true;
        }
        // Otherwise a packet was lost. The ATEM sends it again because we do not acknowledge.
        outcome
    }
}

pub struct AtemConnector {
    hub: Arc<Hub>,
    endpoint: Endpoint,
    connected: Arc<AtomicBool>,
    task: Option<TaskGuard>,
}

impl AtemConnector {
    pub fn new(hub: Arc<Hub>, endpoint: Endpoint) -> Self {
        AtemConnector {
            hub,
            endpoint,
            connected: Arc::new(AtomicBool::new(false)),
            task: None,
        }
    }
}

impl Connector for AtemConnector {
    fn connect(&mut self) {
        let hub = Arc::clone(&self.hub);
        let endpoint = self.endpoint.clone();
        let connected = Arc::clone(&self.connected);
        self.task = Some(TaskGuard::spawn(async move {
            run(hub, endpoint, connected).await
        }));
    }

    fn disconnect(&mut self) {
        info!("Cutting connection to Atem mixer.");
        self.task = None; // aborts the task
        self.connected.store(false, Ordering::SeqCst);
        self.hub.notify_mixer_is_disconnected();
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

async fn run(hub: Arc<Hub>, endpoint: Endpoint, connected: Arc<AtomicBool>) {
    loop {
        if let Err(e) = connection(&hub, &endpoint, &connected).await {
            error!("ATEM at {}:{}: {e}", endpoint.ip, endpoint.port);
            sleep(Duration::from_secs(5)).await;
        }
    }
}

fn set_disconnected(hub: &Hub, connected: &AtomicBool) {
    connected.store(false, Ordering::SeqCst);
    hub.notify_mixer_is_disconnected();
}

/// Talks to the ATEM until the socket breaks. Reconnects by itself if the ATEM goes silent.
async fn connection(hub: &Arc<Hub>, endpoint: &Endpoint, connected: &AtomicBool) -> Result<(), String> {
    let target = lookup_host((endpoint.ip.as_str(), endpoint.port))
        .await
        .map_err(|e| format!("Could not resolve the address: {e}"))?
        .min_by_key(|a| !a.is_ipv4()) // IPv4 first
        .ok_or("Could not resolve the address")?;
    let bind = if target.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let socket = UdpSocket::bind(bind).await.map_err(|e| e.to_string())?;
    socket.connect(target).await.map_err(|e| e.to_string())?;

    info!("Connecting to ATEM at {}:{}", endpoint.ip, endpoint.port);

    let mut session = Session::new();
    let mut initialized = false;
    let mut warned = false;
    let mut last_received = Instant::now();
    let mut ack_due: Option<Instant> = None;
    let mut check = interval(CHECK_INTERVAL);
    check.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut buf = [0u8; 2048];

    let _ = socket.send(&HELLO).await;

    loop {
        tokio::select! {
            received = socket.recv(&mut buf) => {
                let n = match received {
                    Ok(n) => n,
                    Err(e) => {
                        // e.g. "connection refused" when an ICMP error arrives because nobody listens
                        debug!("Receiving from the ATEM failed: {e}");
                        sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                last_received = Instant::now();
                warned = false;
                let outcome = session.handle(&buf[..n]);
                if outcome.ack_now {
                    ack_due = None;
                    let _ = socket.send(&session.ack_packet()).await;
                } else if outcome.ack_later && ack_due.is_none() {
                    ack_due = Some(Instant::now() + ACK_DELAY);
                }
                if outcome.changed {
                    if session.state.initialized && !initialized {
                        initialized = true;
                        info!("Connected to ATEM");
                        connected.store(true, Ordering::SeqCst);
                        hub.notify_mixer_is_connected();
                    } else if !session.state.initialized {
                        initialized = false; // a new dump has started
                    }
                    if initialized {
                        publish(hub, &session.state);
                    }
                }
            }
            _ = async { sleep_until(ack_due.unwrap()).await }, if ack_due.is_some() => {
                ack_due = None;
                let _ = socket.send(&session.ack_packet()).await;
            }
            _ = check.tick() => {
                let silent = last_received.elapsed() >= CONNECTION_TIMEOUT;
                if silent {
                    if initialized {
                        warn!("Lost connection to ATEM");
                        initialized = false;
                        set_disconnected(hub, connected);
                    } else if !warned {
                        warn!("Could not connect to ATEM. This could mean that the maximum number of devices are already connected to ATEM.");
                        warned = true;
                    }
                    session = Session::new();
                }
                if silent || !session.is_established() {
                    // (again) ask for a session
                    let _ = socket.send(&HELLO).await;
                }
            }
        }
    }
}

/// Tells the hub what the ATEM shows.
fn publish(hub: &Hub, state: &State) {
    let list = |mode| {
        // the Node hub sorts the ids as strings
        let mut ids: Vec<String> = state
            .visible_inputs(mode, 0)
            .iter()
            .map(|i| i.to_string())
            .collect();
        ids.sort();
        ids
    };
    hub.notify_program_preview_changed(Some(list(Mode::Program)), Some(list(Mode::Preview)));
    hub.notify_channels(Some(state.channels()));
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;
    use crate::appconfig::AppConfig;
    use crate::hub::HubOptions;

    fn input_payload(id: u16, name: &str, port_type: u8) -> Vec<u8> {
        let mut p = vec![0u8; 36];
        p[0..2].copy_from_slice(&id.to_be_bytes());
        p[2..2 + name.len()].copy_from_slice(name.as_bytes());
        p[32] = port_type;
        p
    }

    /// What a small ATEM sends when a client connects.
    fn dump() -> Vec<Vec<u8>> {
        vec![
            [
                command("_ver", &[0, 2, 0, 28]),
                command("InPr", &input_payload(1, "Camera 1", 0)),
                command("InPr", &input_payload(2, "Camera 2", 0)),
                command("InPr", &input_payload(3, "Camera 3", 0)),
                command("InPr", &input_payload(1000, "Bars", 2)),
            ]
            .concat(),
            [
                command("PrgI", &[0, 0, 0, 2]),
                command("PrvI", &[0, 0, 0, 3, 0, 0, 0, 0]),
                command("InCm", &[1, 0, 0, 0]),
            ]
            .concat(),
        ]
    }

    #[test]
    fn session_acknowledges_in_order_packets_and_applies_commands() {
        let mut s = Session::new();
        // the hello answer
        let o = s.handle(&packet(FLAG_NEW_SESSION, 0x8001, 0, &[2, 0, 0, 0, 0, 0, 0, 0]));
        assert!(o.ack_now);
        assert!(s.is_established());
        assert_eq!(s.ack_packet()[2..6], [0x80, 0x01, 0, 0]);

        let d = dump();
        let o = s.handle(&packet(FLAG_ACK_REQUEST, 0x8001, 1, &d[0]));
        assert_eq!(o, Outcome { ack_later: true, changed: true, ..Default::default() });
        assert_eq!(s.ack_packet()[4..6], [0, 1]);
        assert!(!s.state.initialized);

        s.handle(&packet(FLAG_ACK_REQUEST, 0x8001, 2, &d[1]));
        assert!(s.state.initialized);
        assert_eq!(s.state.visible_inputs(Mode::Program, 0), vec![2]);
        assert_eq!(s.state.visible_inputs(Mode::Preview, 0), vec![3]);
    }

    #[test]
    fn session_ignores_packets_that_skip_one_and_reacks_duplicates() {
        let mut s = Session::new();
        s.handle(&packet(FLAG_NEW_SESSION, 7, 0, &[2, 0, 0, 0, 0, 0, 0, 0]));
        let p = command("PrgI", &[0, 0, 0, 5]);
        // packet 2 arrives before packet 1
        let o = s.handle(&packet(FLAG_ACK_REQUEST, 7, 2, &p));
        assert_eq!(o, Outcome::default());
        assert_eq!(s.state.visible_inputs(Mode::Program, 0), vec![0]);
        s.handle(&packet(FLAG_ACK_REQUEST, 7, 1, &p));
        assert_eq!(s.state.visible_inputs(Mode::Program, 0), vec![5]);
        // packet 1 again
        let o = s.handle(&packet(FLAG_ACK_REQUEST, 7, 1, &p));
        assert_eq!(o, Outcome { ack_later: true, ..Default::default() });
    }

    #[test]
    fn session_acknowledges_every_16_packets_at_once() {
        let mut s = Session::new();
        s.handle(&packet(FLAG_NEW_SESSION, 7, 0, &[2, 0, 0, 0, 0, 0, 0, 0]));
        let mut immediate = 0;
        for id in 1..=32 {
            if s.handle(&packet(FLAG_ACK_REQUEST, 7, id, &[])).ack_now {
                immediate += 1;
            }
        }
        assert_eq!(immediate, 2);
    }

    #[test]
    fn session_ignores_data_before_hello_and_garbage() {
        let mut s = Session::new();
        assert_eq!(
            s.handle(&packet(FLAG_ACK_REQUEST, 7, 1, &command("PrgI", &[0, 0, 0, 5]))),
            Outcome::default()
        );
        assert_eq!(s.handle(&[1, 2, 3]), Outcome::default());
    }

    #[test]
    fn session_wraps_packet_ids() {
        let mut s = Session::new();
        s.handle(&packet(FLAG_NEW_SESSION, 7, PACKET_ID_RANGE - 1, &[2, 0, 0, 0, 0, 0, 0, 0]));
        let o = s.handle(&packet(FLAG_ACK_REQUEST, 7, 0, &command("PrgI", &[0, 0, 0, 5])));
        assert!(o.changed);
    }

    // ------------------------------------------------------------------
    // the whole connector against a fake ATEM
    // ------------------------------------------------------------------

    fn hub() -> Arc<Hub> {
        Hub::new(HubOptions::default(), AppConfig::default())
    }

    async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// Plays the ATEM: answers the hello, sends the dump and expects it to be acknowledged.
    /// Returns the ids of the acknowledged packets.
    async fn serve_client(atem: &UdpSocket, session_id: u16) -> (std::net::SocketAddr, Vec<u16>) {
        let mut buf = [0u8; 64];
        let (n, client) = atem.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], &HELLO);
        atem.send_to(&packet(FLAG_NEW_SESSION, session_id, 0, &[2, 0, 0, 0, 0, 0, 0, 0]), client)
            .await
            .unwrap();
        // the client acknowledges the hello
        // (a client that was waiting for an answer sent more than one hello)
        let h = loop {
            let (n, _) = atem.recv_from(&mut buf).await.unwrap();
            if buf[..n] != HELLO {
                break parse_header(&buf[..n]).unwrap();
            }
        };
        assert_eq!(h.flags, FLAG_ACK_REPLY);
        assert_eq!(h.session_id, session_id);

        let mut acks = Vec::new();
        for (i, payload) in dump().iter().enumerate() {
            atem.send_to(&packet(FLAG_ACK_REQUEST, session_id, i as u16 + 1, payload), client)
                .await
                .unwrap();
        }
        // acknowledgements arrive delayed and combined, until everything is covered
        while acks.last() != Some(&2) {
            let (n, _) = tokio::time::timeout(Duration::from_secs(2), atem.recv_from(&mut buf))
                .await
                .expect("the dump was not acknowledged")
                .unwrap();
            assert_eq!(n, HEADER_LENGTH);
            acks.push(u16::from_be_bytes([buf[4], buf[5]]));
        }
        (client, acks)
    }

    #[tokio::test]
    async fn connects_to_an_atem_and_reports_tallies() {
        let atem = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = atem.local_addr().unwrap().port();
        let hub = hub();
        let mut connector = AtemConnector::new(
            Arc::clone(&hub),
            Endpoint { ip: "127.0.0.1".into(), port },
        );
        connector.connect();

        let (client, _) = serve_client(&atem, 0x8123).await;
        wait_for("the connection", || hub.is_mixer_connected()).await;
        assert!(connector.is_connected());
        assert_eq!(hub.programs(), Some(vec!["2".to_string()]));
        assert_eq!(hub.previews(), Some(vec!["3".to_string()]));
        assert_eq!(hub.channels().len(), 3);
        assert_eq!(hub.channels()[1].name.as_deref(), Some("Camera 2"));

        // a cut to camera 1
        atem.send_to(
            &packet(FLAG_ACK_REQUEST, 0x8123, 3, &command("PrgI", &[0, 0, 0, 1])),
            client,
        )
        .await
        .unwrap();
        wait_for("the new program", || hub.programs() == Some(vec!["1".to_string()])).await;

        connector.disconnect();
        assert!(!connector.is_connected());
        assert!(!hub.is_mixer_connected());
    }

    #[tokio::test(start_paused = true)]
    async fn reports_a_lost_connection_and_reconnects() {
        let atem = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = atem.local_addr().unwrap().port();
        let hub = hub();
        let mut connector = AtemConnector::new(
            Arc::clone(&hub),
            Endpoint { ip: "127.0.0.1".into(), port },
        );
        connector.connect();
        serve_client(&atem, 1).await;
        wait_for("the connection", || hub.is_mixer_connected()).await;

        // silence for longer than the timeout
        sleep(CONNECTION_TIMEOUT + Duration::from_secs(2)).await;
        assert!(!hub.is_mixer_connected(), "the loss was not noticed");

        // the client starts over with a new hello
        let (_, acks) = serve_client(&atem, 2).await;
        assert_eq!(acks.last(), Some(&2));
        wait_for("the reconnect", || hub.is_mixer_connected()).await;
        connector.disconnect();
    }
}
