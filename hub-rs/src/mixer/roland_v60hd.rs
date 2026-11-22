//! Roland V-60HD, connected through its "Smart Tally" HTTP interface.
//!
//! See <https://static.roland.com/assets/media/pdf/V-60HD_smart_tally_eng02_W.pdf>.
//!
//! `GET /tally/<input>/status` answers `onair` (program), `selected` (preview) or `unselected`.
//! The eight inputs are polled in regular intervals.

use super::config::{Endpoint, RolandV60HDConfig};
use super::connectors::{Connector, TaskGuard};
use crate::hub::Hub;
use futures_util::future::join_all;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{interval, timeout, MissedTickBehavior};
use tracing::{error, info};

const INPUTS: usize = 8;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const MIN_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tally {
    #[default]
    Off,
    Program,
    Preview,
}

/// The answer of the V-60HD for one input. Everything unknown is "off".
pub fn parse_status(body: &str) -> Tally {
    match body.trim() {
        "onair" => Tally::Program,
        "selected" => Tally::Preview,
        _ => Tally::Off,
    }
}

/// Input numbers (starting with 1) that are on program and in preview.
pub fn program_and_preview(inputs: &[Tally]) -> (Vec<String>, Vec<String>) {
    let numbers = |wanted: Tally| {
        inputs
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == wanted)
            .map(|(i, _)| (i + 1).to_string())
            .collect::<Vec<_>>()
    };
    (numbers(Tally::Program), numbers(Tally::Preview))
}

/// The body of an HTTP response.
fn body_of(response: &str) -> Option<&str> {
    response.split_once("\r\n\r\n").map(|(_, body)| body)
}

async fn query_input(endpoint: &Endpoint, input: usize) -> Result<Tally, String> {
    let address = (endpoint.ip.as_str(), endpoint.port);
    let request = async {
        let mut stream = TcpStream::connect(address).await?;
        let request = format!(
            "GET /tally/{input}/status HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            endpoint.ip, endpoint.port
        );
        stream.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };
    let response = timeout(REQUEST_TIMEOUT, request)
        .await
        .map_err(|_| "timeout".to_string())?
        .map_err(|e| e.to_string())?;
    let response = String::from_utf8_lossy(&response);
    body_of(&response)
        .map(parse_status)
        .ok_or_else(|| "invalid response".to_string())
}

pub struct RolandV60HDConnector {
    hub: Arc<Hub>,
    config: RolandV60HDConfig,
    connected: Arc<AtomicBool>,
    task: Option<TaskGuard>,
}

impl RolandV60HDConnector {
    pub fn new(hub: Arc<Hub>, config: RolandV60HDConfig) -> Self {
        RolandV60HDConnector {
            hub,
            config,
            connected: Arc::new(AtomicBool::new(false)),
            task: None,
        }
    }
}

impl Connector for RolandV60HDConnector {
    fn connect(&mut self) {
        let Endpoint { ip, port } = &self.config.endpoint;
        info!("Connecting to Roland V-60HD at {ip}:{port}");
        let hub = Arc::clone(&self.hub);
        let config = self.config.clone();
        let connected = Arc::clone(&self.connected);
        self.task = Some(TaskGuard::spawn(async move { run(hub, config, connected).await }));
    }

    fn disconnect(&mut self) {
        info!("Roland V-60HD Smart Tally connection closed");
        self.task = None; // aborts the task
        self.connected.store(false, Ordering::SeqCst);
        self.hub.notify_mixer_is_disconnected();
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

async fn run(hub: Arc<Hub>, config: RolandV60HDConfig, connected: Arc<AtomicBool>) {
    let period = Duration::from_secs_f64(config.request_interval.max(0.0) / 1000.0).max(MIN_INTERVAL);
    let mut timer = interval(period);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_error = String::new();
    loop {
        timer.tick().await;
        let answers = join_all((1..=INPUTS).map(|i| query_input(&config.endpoint, i))).await;
        match answers.into_iter().collect::<Result<Vec<_>, _>>() {
            Ok(inputs) => {
                last_error.clear();
                if !connected.swap(true, Ordering::SeqCst) {
                    info!("Connected to Roland V-60HD");
                    hub.notify_mixer_is_connected();
                }
                let (programs, previews) = program_and_preview(&inputs);
                hub.notify_program_preview_changed(Some(programs), Some(previews));
            }
            Err(e) => {
                // do not repeat the same message with every request
                if e != last_error {
                    error!("Roland V-60HD Smart Tally error: {e}");
                    last_error = e;
                }
                if connected.swap(false, Ordering::SeqCst) {
                    hub.notify_mixer_is_disconnected();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appconfig::AppConfig;
    use crate::hub::HubOptions;
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tokio::time::sleep;

    #[test]
    fn parses_statuses() {
        assert_eq!(parse_status("onair"), Tally::Program);
        assert_eq!(parse_status("selected\n"), Tally::Preview);
        assert_eq!(parse_status("unselected"), Tally::Off);
        assert_eq!(parse_status("whatever"), Tally::Off);
        assert_eq!(parse_status(""), Tally::Off);
    }

    #[test]
    fn numbers_start_with_one() {
        use Tally::*;
        let inputs = [Off, Preview, Program, Program, Preview, Off, Off, Preview];
        assert_eq!(
            program_and_preview(&inputs),
            (
                vec!["3".to_string(), "4".to_string()],
                vec!["2".to_string(), "5".to_string(), "8".to_string()]
            )
        );
    }

    #[test]
    fn finds_the_body() {
        assert_eq!(body_of("HTTP/1.1 200 OK\r\nA: b\r\n\r\nonair"), Some("onair"));
        assert_eq!(body_of("garbage"), None);
    }

    /// Plays a V-60HD with the given tally state per input.
    async fn fake_v60hd(listener: TcpListener, tallies: Arc<Mutex<Vec<&'static str>>>) {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let tallies = Arc::clone(&tallies);
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let input: usize = request
                    .split('/')
                    .nth(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                let body = tallies.lock().unwrap()[input - 1];
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
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

    fn config(port: u16) -> RolandV60HDConfig {
        RolandV60HDConfig {
            endpoint: Endpoint { ip: "127.0.0.1".into(), port },
            request_interval: 50.0,
        }
    }

    #[tokio::test]
    async fn reports_tallies_and_follows_changes() {
        let tallies = Arc::new(Mutex::new(vec![
            "unselected", "selected", "onair", "onair", "selected", "unselected", "unselected", "selected",
        ]));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(fake_v60hd(listener, Arc::clone(&tallies)));

        let hub = Hub::new(HubOptions::default(), AppConfig::default());
        let mut mixer = RolandV60HDConnector::new(Arc::clone(&hub), config(port));
        assert!(!mixer.is_connected());
        mixer.connect();

        wait_for("the connection", || hub.is_mixer_connected()).await;
        assert!(mixer.is_connected());
        wait_for("the tallies", || hub.programs().is_some()).await;
        assert_eq!(hub.programs(), Some(vec!["3".to_string(), "4".to_string()]));
        assert_eq!(
            hub.previews(),
            Some(vec!["2".to_string(), "5".to_string(), "8".to_string()])
        );

        tallies.lock().unwrap()[0] = "onair";
        wait_for("the change", || hub.programs().map(|p| p.len()) == Some(3)).await;
        assert_eq!(hub.programs().unwrap()[0], "1");

        mixer.disconnect();
        assert!(!mixer.is_connected());
        assert!(!hub.is_mixer_connected());
    }

    #[tokio::test]
    async fn is_disconnected_while_the_mixer_is_unreachable_and_recovers() {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let hub = Hub::new(HubOptions::default(), AppConfig::default());
        let mut mixer = RolandV60HDConnector::new(Arc::clone(&hub), config(port));
        mixer.connect();
        sleep(Duration::from_millis(200)).await;
        assert!(!mixer.is_connected());
        assert!(!hub.is_mixer_connected());

        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        tokio::spawn(fake_v60hd(listener, Arc::new(Mutex::new(vec!["unselected"; 8]))));
        wait_for("the connection", || hub.is_mixer_connected()).await;
        mixer.disconnect();
    }
}
