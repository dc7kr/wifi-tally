//! Talks to the tally lights over UDP.

use crate::command::{self, TallyCommand};
use crate::hub::Hub;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

pub async fn bind(port: u16) -> std::io::Result<Arc<UdpSocket>> {
    let socket = UdpSocket::bind(("0.0.0.0", port)).await?;
    info!("Listening for Tallies on {}", socket.local_addr()?);
    Ok(Arc::new(socket))
}

fn handle_datagram(hub: &Hub, data: &[u8], from: SocketAddr) {
    let text = String::from_utf8_lossy(data);
    let address = from.ip().to_canonical().to_string();
    match command::parse(text.trim()) {
        Ok(TallyCommand::TallyHo { tally_name }) => {
            hub.udp_tally_reported(&tally_name, &address, from.port());
        }
        Ok(TallyCommand::Log { tally_name, log }) => {
            hub.udp_tally_logged(&tally_name, &address, from.port(), log);
        }
        Err(e) => warn!("{e}"),
    }
}

/// Receives messages from tallies, notices missing ones and sends keep alives.
pub async fn run(hub: Arc<Hub>, socket: Arc<UdpSocket>) {
    let (outbox, mut to_send) = tokio::sync::mpsc::unbounded_channel();
    hub.set_udp_outbox(outbox);

    let mut timeouts = tokio::time::interval(Duration::from_millis(500));
    let keepalive =
        Duration::from_secs_f64(1.0 / f64::from(hub.options.keepalives_per_second.max(1)));
    let mut keepalives = tokio::time::interval(keepalive);
    let mut buf = vec![0u8; 2048];

    loop {
        tokio::select! {
            received = socket.recv_from(&mut buf) => match received {
                Ok((len, from)) => handle_datagram(&hub, &buf[..len], from),
                // e.g. ICMP "port unreachable" of an earlier send on some platforms
                Err(e) => warn!("error receiving from tally: {e}"),
            },
            Some((to, command)) = to_send.recv() => {
                if let Err(e) = socket.send_to(command.as_bytes(), to).await {
                    debug!("could not send to tally at {to}: {e}");
                }
            },
            _ = timeouts.tick() => hub.check_udp_timeouts(),
            _ = keepalives.tick() => hub.send_keepalives(),
        }
    }
}
