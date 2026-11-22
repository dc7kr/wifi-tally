//! vMix, connected through its TCP API.
//!
//! See <https://www.vmix.com/help20/index.htm?TCPAPI.html>.
//!
//! After connecting, vMix greets with `VERSION OK <version>`. We then send
//! `SUBSCRIBE TALLY` and receive `TALLY OK 0120…` whenever a tally changes (one
//! digit per input: 0 = off, 1 = program, 2 = preview). Input names are only
//! available from the `XML` command, which is polled every few seconds.

use super::config::Endpoint;
use super::connectors::{Connector, TaskGuard};
use crate::domain::channel::Channel;
use crate::hub::Hub;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{interval, sleep, timeout, Instant, MissedTickBehavior};
use tracing::{debug, error, info};

const DEFAULT_PORT: u16 = 8099;
const XML_QUERY_INTERVAL: Duration = Duration::from_secs(5);
const WAIT_FOR_HELLO: Duration = Duration::from_secs(5);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// What vMix said.
#[derive(Debug, PartialEq)]
pub enum Message {
    /// `VERSION OK ...`: the greeting
    Version,
    /// `SUBSCRIBE OK TALLY ...`
    SubscribedToTally,
    /// `TALLY OK 012`: the digits
    Tally(String),
    /// A `TALLY OK` that could not be understood.
    BadTally,
    /// The payload of the `XML` command
    Xml(String),
    /// anything else (including answers we did not ask for)
    Other(String),
}

/// Splits the byte stream into messages.
///
/// vMix terminates lines with CRLF. The answer to `XML` is a line `XML <length>`
/// followed by exactly `length` bytes of XML, which can contain line breaks and
/// may arrive in several TCP segments.
#[derive(Default)]
pub struct Parser {
    buffer: Vec<u8>,
    /// Number of XML bytes that are still expected after an `XML <n>` line.
    xml_length: Option<usize>,
}

impl Parser {
    pub fn feed(&mut self, data: &[u8]) -> Vec<Message> {
        self.buffer.extend_from_slice(data);
        let mut messages = Vec::new();
        loop {
            if let Some(length) = self.xml_length {
                if self.buffer.len() < length {
                    break;
                }
                let xml: Vec<u8> = self.buffer.drain(..length).collect();
                self.xml_length = None;
                messages.push(Message::Xml(String::from_utf8_lossy(&xml).into_owned()));
                continue;
            }
            let Some(end) = self.buffer.windows(2).position(|w| w == b"\r\n") else {
                break;
            };
            let line: Vec<u8> = self.buffer.drain(..end + 2).take(end).collect();
            let line = String::from_utf8_lossy(&line).into_owned();
            if line.is_empty() {
                continue; // e.g. the CRLF that terminates the XML payload
            }
            if let Some(length) = line.strip_prefix("XML ").and_then(|n| n.trim().parse().ok()) {
                self.xml_length = Some(length);
            } else {
                messages.push(Self::classify(line));
            }
        }
        messages
    }

    fn classify(line: String) -> Message {
        if line.starts_with("VERSION OK") {
            Message::Version
        } else if line.starts_with("SUBSCRIBE OK TALLY") {
            Message::SubscribedToTally
        } else if line.starts_with("TALLY OK") {
            match line.strip_prefix("TALLY OK ") {
                Some(digits) if digits.bytes().all(|b| b.is_ascii_digit()) => {
                    Message::Tally(digits.to_string())
                }
                _ => Message::BadTally,
            }
        } else if line.starts_with("<vmix>") {
            // an XML document without the length header
            Message::Xml(line)
        } else {
            Message::Other(line)
        }
    }
}

/// Programs and previews from the digits of a `TALLY OK` message. Input
/// numbers start at 1.
pub fn parse_tally(digits: &str) -> (Vec<String>, Vec<String>) {
    let mut programs = Vec::new();
    let mut previews = Vec::new();
    for (i, digit) in digits.chars().enumerate() {
        match digit {
            '1' => programs.push((i + 1).to_string()),
            '2' => previews.push((i + 1).to_string()),
            _ => {}
        }
    }
    (programs, previews)
}

/// The inputs from the XML that vMix returns for the `XML` command, named by their short title.
pub fn parse_inputs(xml: &str) -> Result<Vec<Channel>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let vmix = doc.root_element();
    if vmix.tag_name().name() != "vmix" {
        return Err("the root element is not <vmix>".into());
    }
    let inputs = vmix
        .children()
        .find(|n| n.has_tag_name("inputs"))
        .ok_or("could not find inputs")?;
    Ok(inputs
        .children()
        .filter(|n| n.has_tag_name("input"))
        .enumerate()
        .map(|(i, input)| {
            Channel::new(
                (i + 1).to_string(),
                input.attribute("shortTitle").map(str::to_string),
            )
        })
        .collect())
}

pub struct VmixConnector {
    hub: Arc<Hub>,
    endpoint: Endpoint,
    connected: Arc<AtomicBool>,
    task: Option<TaskGuard>,
}

impl VmixConnector {
    pub fn new(hub: Arc<Hub>, endpoint: Endpoint) -> Self {
        VmixConnector {
            hub,
            endpoint,
            connected: Arc::new(AtomicBool::new(false)),
            task: None,
        }
    }
}

impl Connector for VmixConnector {
    fn connect(&mut self) {
        let hub = Arc::clone(&self.hub);
        let endpoint = self.endpoint.clone();
        let connected = Arc::clone(&self.connected);
        self.task = Some(TaskGuard::spawn(async move {
            run(hub, endpoint, connected).await
        }));
    }

    fn disconnect(&mut self) {
        info!("Disconnecting from vMix");
        self.task = None; // aborts the task
        self.connected.store(false, Ordering::SeqCst);
        self.hub.notify_mixer_is_disconnected();
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Connects again and again until the task is aborted.
async fn run(hub: Arc<Hub>, endpoint: Endpoint, connected: Arc<AtomicBool>) {
    let mut last_error = String::new();
    loop {
        info!("Connecting to vMix at {}:{}", endpoint.ip, endpoint.port);
        let result = session(&hub, &endpoint, &connected).await;
        connected.store(false, Ordering::SeqCst);
        hub.notify_mixer_is_disconnected();
        match result {
            Ok(()) => info!("Connection to vMix closed"),
            Err(e) => {
                // do not repeat the same message every second while vMix is away
                if e != last_error {
                    error!("{e}");
                    last_error = e;
                }
            }
        }
        sleep(RECONNECT_DELAY).await;
    }
}

/// One connection. Returns when it was closed or failed.
async fn session(
    hub: &Arc<Hub>,
    endpoint: &Endpoint,
    connected: &AtomicBool,
) -> Result<(), String> {
    let address = (endpoint.ip.as_str(), endpoint.port);
    let mut stream = timeout(Duration::from_secs(5), TcpStream::connect(address))
        .await
        .map_err(|_| format!("Connection to vMix at {}:{} timed out", endpoint.ip, endpoint.port))?
        .map_err(|e| format!("Could not connect to vMix at {}:{}: {e}", endpoint.ip, endpoint.port))?;
    debug!("TCP connection to {}:{} established", endpoint.ip, endpoint.port);

    let io_error = |e: std::io::Error| format!("Connection to vMix failed: {e}");
    stream
        .write_all(b"SUBSCRIBE TALLY\r\nXML\r\n")
        .await
        .map_err(io_error)?;

    let mut parser = Parser::default();
    let mut hello = false;
    let mut subscribed = false;
    let deadline = Instant::now() + WAIT_FOR_HELLO;
    let mut xml_timer = interval(XML_QUERY_INTERVAL);
    xml_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    xml_timer.tick().await; // the first tick is immediate; the query was sent above
    let mut buf = [0u8; 8192];

    loop {
        let complete = hello && subscribed;
        tokio::select! {
            read = stream.read(&mut buf) => {
                let n = read.map_err(io_error)?;
                if n == 0 {
                    return Ok(());
                }
                for message in parser.feed(&buf[..n]) {
                    match message {
                        Message::Version => {
                            debug!("Connection to vMix established");
                            hello = true;
                        }
                        Message::SubscribedToTally => {
                            debug!("Successfully subscribed to tally updates from vMix");
                            subscribed = true;
                        }
                        Message::Tally(digits) => {
                            let (programs, previews) = parse_tally(&digits);
                            hub.notify_program_preview_changed(Some(programs), Some(previews));
                        }
                        Message::BadTally => error!("Tally OK command was ill formed"),
                        Message::Xml(xml) => match parse_inputs(&xml) {
                            Ok(channels) => hub.notify_channels(Some(channels)),
                            Err(e) => error!("Could not read the XML from vMix: {e}"),
                        },
                        Message::Other(line) => debug!("Ignoring unknown command from vMix: {line}"),
                    }
                    if hello && subscribed && !connected.load(Ordering::SeqCst) {
                        info!("Connection to vMix complete");
                        connected.store(true, Ordering::SeqCst);
                        hub.notify_mixer_is_connected();
                    }
                }
            }
            _ = xml_timer.tick() => {
                // there is no way to subscribe to renamed or new inputs, so poll
                stream.write_all(b"XML\r\n").await.map_err(io_error)?;
            }
            _ = tokio::time::sleep_until(deadline), if !complete => {
                return Err(format!(
                    "The remote at {}:{} did not identify as vMix TCPAPI. Is this the correct port for the TCPAPI? (default {DEFAULT_PORT})",
                    endpoint.ip, endpoint.port
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_digits() {
        assert_eq!(parse_tally("012"), (vec!["2".into()], vec!["3".into()]));
        assert_eq!(
            parse_tally("012210"),
            (vec!["2".into(), "5".into()], vec!["3".into(), "4".into()])
        );
        assert_eq!(parse_tally(""), (vec![], vec![]));
    }

    #[test]
    fn classifies_lines() {
        let mut p = Parser::default();
        let m = p.feed(b"VERSION OK 0.1.2.3\r\nSUBSCRIBE OK TALLY Subscribed\r\nTALLY OK 012\r\nTALLY OK x\r\nFOO\r\n");
        assert_eq!(
            m,
            vec![
                Message::Version,
                Message::SubscribedToTally,
                Message::Tally("012".into()),
                Message::BadTally,
                Message::Other("FOO".into()),
            ]
        );
    }

    #[test]
    fn handles_split_lines() {
        let mut p = Parser::default();
        assert_eq!(p.feed(b"TALLY O"), vec![]);
        assert_eq!(p.feed(b"K 01\r"), vec![]);
        assert_eq!(p.feed(b"\n"), vec![Message::Tally("01".into())]);
    }

    #[test]
    fn xml_is_read_by_length_even_with_line_breaks_and_split_segments() {
        let xml = "<vmix>\r\n<inputs/>\r\n</vmix>";
        let mut p = Parser::default();
        let mut data = format!("XML {}\r\n{xml}\r\nTALLY OK 1\r\n", xml.len()).into_bytes();
        let tail = data.split_off(20);
        assert_eq!(p.feed(&data), vec![]);
        assert_eq!(
            p.feed(&tail),
            vec![Message::Xml(xml.into()), Message::Tally("1".into())]
        );
    }

    #[test]
    fn xml_without_length_header_is_accepted() {
        let mut p = Parser::default();
        assert_eq!(
            p.feed(b"<vmix><inputs/></vmix>\r\n"),
            vec![Message::Xml("<vmix><inputs/></vmix>".into())]
        );
    }

    const XML: &str = r#"<vmix><version>1</version><inputs><input key="a" number="1" type="Colour" title="Foobar" shortTitle="Foobar" state="Paused">Foobar</input><input key="b" number="2" type="Colour" title="Tolle rote Farbe" shortTitle="Tolle rote Farbe">x</input><input key="c" number="3" title="Colour Bars" shortTitle="Colour Bars"/></inputs><preview>2</preview></vmix>"#;

    #[test]
    fn reads_input_names() {
        let channels = parse_inputs(XML).unwrap();
        assert_eq!(
            channels,
            vec![
                Channel::new("1", Some("Foobar".into())),
                Channel::new("2", Some("Tolle rote Farbe".into())),
                Channel::new("3", Some("Colour Bars".into())),
            ]
        );
    }

    #[test]
    fn rejects_faulty_xml() {
        assert!(parse_inputs("<vmix></vmix>").is_err());
        assert!(parse_inputs("<foo/>").is_err());
        assert!(parse_inputs("<vmix>").is_err());
    }

    #[test]
    fn no_inputs_is_an_empty_list() {
        assert_eq!(parse_inputs("<vmix><inputs/></vmix>").unwrap(), vec![]);
    }

    // ------------------------------------------------------------------
    // the connector against a fake vMix (like the Jest spec of the Node hub)
    // ------------------------------------------------------------------

    use crate::appconfig::AppConfig;
    use crate::hub::HubOptions;
    use tokio::net::TcpListener;

    /// Plays vMix: greets, answers `SUBSCRIBE TALLY` and `XML`. Returns the number of accepted connections.
    async fn fake_vmix(listener: TcpListener, tallies: &'static str, xml: &'static str) {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                socket.write_all(b"VERSION OK 0.1.2.3\r\n").await.unwrap();
                let mut parser = Vec::new();
                let mut buf = [0u8; 256];
                loop {
                    let n = match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    parser.extend_from_slice(&buf[..n]);
                    while let Some(end) = parser.windows(2).position(|w| w == b"\r\n") {
                        let line: Vec<u8> = parser.drain(..end + 2).take(end).collect();
                        let reply = match &line[..] {
                            b"SUBSCRIBE TALLY" => format!(
                                "SUBSCRIBE OK TALLY Subscribed\r\nTALLY OK {tallies}\r\n"
                            ),
                            b"XML" => format!("XML {}\r\n{xml}\r\n", xml.len()),
                            _ => continue,
                        };
                        // in two pieces, as TCP may do
                        let (a, b) = reply.split_at(reply.len() / 2);
                        socket.write_all(a.as_bytes()).await.unwrap();
                        tokio::task::yield_now().await;
                        socket.write_all(b.as_bytes()).await.unwrap();
                    }
                }
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

    #[tokio::test]
    async fn connects_to_vmix_and_reports_tallies_and_names() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(fake_vmix(listener, "012210", XML));

        let hub = Hub::new(HubOptions::default(), AppConfig::default());
        let mut vmix = VmixConnector::new(
            Arc::clone(&hub),
            Endpoint { ip: "127.0.0.1".into(), port },
        );
        assert!(!vmix.is_connected());
        vmix.connect();

        wait_for("the connection", || hub.is_mixer_connected()).await;
        assert!(vmix.is_connected());
        wait_for("the tallies", || hub.programs().is_some()).await;
        assert_eq!(hub.programs(), Some(vec!["2".to_string(), "5".to_string()]));
        assert_eq!(hub.previews(), Some(vec!["3".to_string(), "4".to_string()]));
        wait_for("the names", || hub.channels().len() == 3).await;
        assert_eq!(hub.channels()[1].name.as_deref(), Some("Tolle rote Farbe"));

        vmix.disconnect();
        assert!(!vmix.is_connected());
        assert!(!hub.is_mixer_connected());
    }

    #[tokio::test]
    async fn reconnects_when_vmix_comes_back() {
        // nobody listens first
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let hub = Hub::new(HubOptions::default(), AppConfig::default());
        let mut vmix = VmixConnector::new(
            Arc::clone(&hub),
            Endpoint { ip: "127.0.0.1".into(), port },
        );
        vmix.connect();
        sleep(Duration::from_millis(300)).await;
        assert!(!hub.is_mixer_connected());

        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        tokio::spawn(fake_vmix(listener, "1", XML));
        wait_for("the connection", || hub.is_mixer_connected()).await;
        vmix.disconnect();
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_server_that_is_not_vmix() {
        // accepts, but says nothing
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&accepted);
        tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                held.push(listener.accept().await.unwrap());
                counter.fetch_add(1, Ordering::SeqCst);
            }
        });

        let hub = Hub::new(HubOptions::default(), AppConfig::default());
        let mut vmix = VmixConnector::new(
            Arc::clone(&hub),
            Endpoint { ip: "127.0.0.1".into(), port },
        );
        vmix.connect();
        sleep(Duration::from_secs(15)).await;
        assert!(!hub.is_mixer_connected());
        assert!(
            accepted.load(Ordering::SeqCst) >= 2,
            "it should try again after the hello did not come"
        );
        vmix.disconnect();
    }
}
