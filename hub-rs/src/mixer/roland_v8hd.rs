//! Roland V-8HD, connected through MIDI (USB).
//!
//! See <https://static.roland.com/assets/media/pdf/V-8HD_reference_eng03_W.pdf>.
//!
//! The mixer shows up as a MIDI port with "V-8HD" in its name. For each of the eight inputs the hub
//! regularly sends a SysEx request for the tally state; the answers say whether the input is off (0),
//! on program (1) or in preview (2). The tallies are published once the answer for the last input arrived.

use super::config::RolandV8HDConfig;
use super::connectors::{Connector, TaskGuard};
use crate::hub::Hub;
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, MissedTickBehavior};
use tracing::{debug, error, info};

const INPUTS: usize = 8;
const DEVICE_NAME: &str = "V-8HD";
const RETRY_DELAY: Duration = Duration::from_secs(2);
const MIN_INTERVAL: Duration = Duration::from_millis(10);

/// The SysEx message that asks for the tally state of an input (0..8).
pub fn query_message(input: usize) -> [u8; 16] {
    let mut message = [
        0xF0, 0x41, 0x10, 0x00, 0x00, 0x00, 0x68, 0x11, // header: Roland, device id, model id, RQ1
        0x0C, 0x00, input as u8, 0x00, // address of the input's tally state
        0x00, 0x03, // size
        0x00, // checksum
        0xF7,
    ];
    // Roland checksum: the sum of address, size and checksum is a multiple of 128
    let sum: u32 = message[8..14].iter().map(|b| *b as u32).sum();
    message[14] = ((128 - sum % 128) % 128) as u8;
    message
}

/// What the V-8HD said about its inputs.
#[derive(Debug, Default)]
pub struct InputStatus {
    status: [u8; INPUTS],
}

impl InputStatus {
    /// Handles a MIDI message. Returns the programs and previews (input numbers starting with 1)
    /// when the answer for the last input arrived.
    pub fn feed(&mut self, message: &[u8]) -> Option<(Vec<String>, Vec<String>)> {
        // answers for the tally parameter area have 12 as the first address byte
        if message.len() < 12 || message[8] != 12 {
            return None;
        }
        let input = message[10] as usize;
        if input >= INPUTS {
            return None;
        }
        self.status[input] = message[11];
        // only publish after a whole round
        if input != INPUTS - 1 {
            return None;
        }
        let numbers = |wanted: u8| {
            (0..INPUTS)
                .filter(|i| self.status[*i] == wanted)
                .map(|i| (i + 1).to_string())
                .collect::<Vec<_>>()
        };
        Some((numbers(1), numbers(2)))
    }
}

pub struct RolandV8HDConnector {
    hub: Arc<Hub>,
    config: RolandV8HDConfig,
    connected: Arc<AtomicBool>,
    task: Option<TaskGuard>,
}

impl RolandV8HDConnector {
    pub fn new(hub: Arc<Hub>, config: RolandV8HDConfig) -> Self {
        RolandV8HDConnector {
            hub,
            config,
            connected: Arc::new(AtomicBool::new(false)),
            task: None,
        }
    }
}

impl Connector for RolandV8HDConnector {
    fn connect(&mut self) {
        info!("Connecting to Roland V-8HD via MIDI");
        let hub = Arc::clone(&self.hub);
        let config = self.config.clone();
        let connected = Arc::clone(&self.connected);
        self.task = Some(TaskGuard::spawn(async move { run(hub, config, connected).await }));
    }

    fn disconnect(&mut self) {
        info!("Roland V-8HD connection closed");
        self.task = None; // aborts the task, which closes the MIDI ports
        self.connected.store(false, Ordering::SeqCst);
        self.hub.notify_mixer_is_disconnected();
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// The open MIDI ports. They are closed when this is dropped.
struct Ports {
    _input: MidiInputConnection<()>,
    output: MidiOutputConnection,
    messages: mpsc::UnboundedReceiver<Vec<u8>>,
}

fn open_ports() -> Result<Ports, String> {
    let mut input = MidiInput::new("vtally-in").map_err(|e| format!("MIDI is not available: {e}"))?;
    // SysEx messages are what we are interested in, timing and active sensing are not
    input.ignore(Ignore::TimeAndActiveSense);
    let input_port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).is_ok_and(|n| n.contains(DEVICE_NAME)))
        .ok_or_else(|| format!("No MIDI input port of the {DEVICE_NAME} found"))?;
    let output = MidiOutput::new("vtally-out").map_err(|e| format!("MIDI is not available: {e}"))?;
    let output_port = output
        .ports()
        .into_iter()
        .find(|p| output.port_name(p).is_ok_and(|n| n.contains(DEVICE_NAME)))
        .ok_or_else(|| format!("No MIDI output port of the {DEVICE_NAME} found"))?;

    let (tx, messages) = mpsc::unbounded_channel();
    let input_name = input.port_name(&input_port).unwrap_or_default();
    let input = input
        .connect(
            &input_port,
            "vtally-in",
            move |_, message, _| {
                let _ = tx.send(message.to_vec());
            },
            (),
        )
        .map_err(|e| format!("Could not open the MIDI input port {input_name}: {e}"))?;
    let output_name = output.port_name(&output_port).unwrap_or_default();
    let output = output
        .connect(&output_port, "vtally-out")
        .map_err(|e| format!("Could not open the MIDI output port {output_name}: {e}"))?;
    info!("Opened MIDI ports {input_name} / {output_name}");
    Ok(Ports { _input: input, output, messages })
}

/// Connects again and again until the task is aborted.
async fn run(hub: Arc<Hub>, config: RolandV8HDConfig, connected: Arc<AtomicBool>) {
    let mut last_error = String::new();
    loop {
        let result = match open_ports() {
            Ok(ports) => session(&hub, &config, &connected, ports).await,
            Err(e) => Err(e),
        };
        connected.store(false, Ordering::SeqCst);
        hub.notify_mixer_is_disconnected();
        if let Err(e) = result {
            // do not repeat the same message while the device is away
            if e != last_error {
                error!("{e}. Please check the connection.");
                last_error = e;
            }
        }
        sleep(RETRY_DELAY).await;
    }
}

/// Talks to the V-8HD until a message can not be sent anymore.
async fn session(
    hub: &Arc<Hub>,
    config: &RolandV8HDConfig,
    connected: &AtomicBool,
    mut ports: Ports,
) -> Result<(), String> {
    connected.store(true, Ordering::SeqCst);
    hub.notify_mixer_is_connected();

    let period = Duration::from_secs_f64(config.request_interval.max(0.0) / 1000.0).max(MIN_INTERVAL);
    let mut timer = interval(period);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut status = InputStatus::default();
    loop {
        tokio::select! {
            _ = timer.tick() => {
                for input in 0..INPUTS {
                    ports
                        .output
                        .send(&query_message(input))
                        .map_err(|e| format!("Could not send to the {DEVICE_NAME}: {e}"))?;
                }
            }
            message = ports.messages.recv() => {
                let Some(message) = message else {
                    return Err(format!("The MIDI input of the {DEVICE_NAME} was closed"));
                };
                if let Some((programs, previews)) = status.feed(&message) {
                    debug!("V-8HD tallies: {programs:?} {previews:?}");
                    hub.notify_program_preview_changed(Some(programs), Some(previews));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the TypeScript hub sent: it incremented the input byte and decremented the checksum.
    fn node_message(input: u8) -> [u8; 16] {
        let mut m = [
            0xF0, 0x41, 0x10, 0x00, 0x00, 0x00, 0x68, 0x11, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x03, 0x71, 0xF7,
        ];
        m[10] += input;
        m[14] -= input;
        m
    }

    #[test]
    fn queries_are_the_same_as_before() {
        for input in 0..INPUTS {
            assert_eq!(query_message(input), node_message(input as u8), "input {input}");
        }
    }

    fn answer(input: u8, status: u8) -> Vec<u8> {
        vec![0xF0, 0x41, 0x10, 0x00, 0x00, 0x00, 0x68, 0x12, 0x0C, 0x00, input, status, 0x00, 0xF7]
    }

    #[test]
    fn publishes_after_the_last_input() {
        let mut s = InputStatus::default();
        let states = [0, 2, 1, 1, 2, 0, 0, 2];
        for (i, state) in states.iter().enumerate().take(7) {
            assert_eq!(s.feed(&answer(i as u8, *state)), None, "input {i}");
        }
        assert_eq!(
            s.feed(&answer(7, states[7])),
            Some((
                vec!["3".to_string(), "4".to_string()],
                vec!["2".to_string(), "5".to_string(), "8".to_string()]
            ))
        );
    }

    #[test]
    fn remembers_inputs_between_rounds() {
        let mut s = InputStatus::default();
        for i in 0..8 {
            s.feed(&answer(i, if i == 0 { 1 } else { 0 }));
        }
        // only the first input changes
        s.feed(&answer(0, 2));
        assert_eq!(s.feed(&answer(7, 0)), Some((vec![], vec!["1".to_string()])));
    }

    #[test]
    fn ignores_other_messages() {
        let mut s = InputStatus::default();
        assert_eq!(s.feed(&[]), None);
        assert_eq!(s.feed(&[0xF0, 0x41, 0xF7]), None);
        // another address area
        let mut other = answer(7, 1);
        other[8] = 0x0D;
        assert_eq!(s.feed(&other), None);
        // an input that does not exist
        assert_eq!(s.feed(&answer(9, 1)), None);
    }
}
