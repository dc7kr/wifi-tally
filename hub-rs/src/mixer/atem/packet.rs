//! The UDP protocol of the ATEM ("BURP").
//!
//! Every datagram starts with a 12 byte header:
//!
//! | bytes | meaning                                              |
//! |-------|------------------------------------------------------|
//! | 0–1   | flags (upper 5 bits) and the length of the datagram  |
//! | 2–3   | session id                                           |
//! | 4–5   | id of the packet that is acknowledged                |
//! | 6–7   | id from which packets are requested again            |
//! | 8–9   | unknown                                              |
//! | 10–11 | id of this packet                                    |
//!
//! The payload is a list of commands: length (2), padding (2), name (4), data.

pub const FLAG_ACK_REQUEST: u8 = 0x01;
pub const FLAG_NEW_SESSION: u8 = 0x02;
pub const FLAG_ACK_REPLY: u8 = 0x10;

pub const HEADER_LENGTH: usize = 12;
/// The ATEM counts packets with 15 bits.
pub const PACKET_ID_RANGE: u16 = 1 << 15;

/// The first packet of a connection.
pub const HELLO: [u8; 20] = [
    0x10, 0x14, 0x53, 0xab, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3a, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
];

#[derive(Debug, PartialEq)]
pub struct Header {
    pub flags: u8,
    pub session_id: u16,
    pub packet_id: u16,
}

/// Reads the header of a datagram. `None` if it is too short or its length does not match.
pub fn parse_header(data: &[u8]) -> Option<Header> {
    if data.len() < HEADER_LENGTH {
        return None;
    }
    let first = u16::from_be_bytes([data[0], data[1]]);
    if (first & 0x07ff) as usize != data.len() {
        return None;
    }
    Some(Header {
        flags: (first >> 11) as u8,
        session_id: u16::from_be_bytes([data[2], data[3]]),
        packet_id: u16::from_be_bytes([data[10], data[11]]),
    })
}

/// Confirms all packets up to and including `packet_id`.
pub fn ack(session_id: u16, packet_id: u16) -> [u8; HEADER_LENGTH] {
    let mut p = [0u8; HEADER_LENGTH];
    let first = ((FLAG_ACK_REPLY as u16) << 11) | HEADER_LENGTH as u16;
    p[0..2].copy_from_slice(&first.to_be_bytes());
    p[2..4].copy_from_slice(&session_id.to_be_bytes());
    p[4..6].copy_from_slice(&packet_id.to_be_bytes());
    p
}

/// Whether an acknowledgement of `ack_id` covers `packet_id`, considering that ids wrap around.
pub fn is_covered_by_ack(ack_id: u16, packet_id: u16) -> bool {
    let tolerance = PACKET_ID_RANGE / 2;
    let shortly_before = packet_id < ack_id && packet_id + tolerance > ack_id;
    let shortly_after = packet_id > ack_id && packet_id < ack_id + tolerance;
    let before_wrap = packet_id > ack_id + tolerance;
    packet_id == ack_id || ((shortly_before || before_wrap) && !shortly_after)
}

/// The commands in the payload of a datagram: name and data.
pub fn commands(mut payload: &[u8]) -> Vec<(String, &[u8])> {
    let mut result = Vec::new();
    while payload.len() >= 8 {
        let length = u16::from_be_bytes([payload[0], payload[1]]) as usize;
        // a command is never shorter than its header
        if length < 8 || length > payload.len() {
            break;
        }
        let name = String::from_utf8_lossy(&payload[4..8]).into_owned();
        result.push((name, &payload[8..length]));
        payload = &payload[length..];
    }
    result
}

#[cfg(test)]
pub mod build {
    //! Builds what an ATEM would send.
    use super::*;

    pub fn packet(flags: u8, session_id: u16, packet_id: u16, payload: &[u8]) -> Vec<u8> {
        let length = HEADER_LENGTH + payload.len();
        let mut p = vec![0u8; length];
        p[0..2].copy_from_slice(&(((flags as u16) << 11) | length as u16).to_be_bytes());
        p[2..4].copy_from_slice(&session_id.to_be_bytes());
        p[10..12].copy_from_slice(&packet_id.to_be_bytes());
        p[HEADER_LENGTH..].copy_from_slice(payload);
        p
    }

    pub fn command(name: &str, data: &[u8]) -> Vec<u8> {
        let mut c = vec![0u8; 8 + data.len()];
        let length = c.len() as u16;
        c[0..2].copy_from_slice(&length.to_be_bytes());
        c[4..8].copy_from_slice(name.as_bytes());
        c[8..].copy_from_slice(data);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn hello_is_a_valid_packet() {
        let h = parse_header(&HELLO).unwrap();
        assert_eq!(h.flags, FLAG_NEW_SESSION);
        assert_eq!(HELLO.len(), 20);
    }

    #[test]
    fn parses_headers() {
        let p = packet(FLAG_ACK_REQUEST, 0x8123, 77, &[1, 2, 3]);
        assert_eq!(
            parse_header(&p),
            Some(Header {
                flags: FLAG_ACK_REQUEST,
                session_id: 0x8123,
                packet_id: 77
            })
        );
    }

    #[test]
    fn rejects_broken_datagrams() {
        assert_eq!(parse_header(&[0; 5]), None);
        let mut p = packet(FLAG_ACK_REQUEST, 1, 1, &[1, 2, 3]);
        p.push(0); // the length field does not match any more
        assert_eq!(parse_header(&p), None);
    }

    #[test]
    fn builds_acks() {
        let a = ack(0x8123, 300);
        assert_eq!(a, [0x80, 0x0c, 0x81, 0x23, 0x01, 0x2c, 0, 0, 0, 0, 0, 0]);
        let h = parse_header(&a).unwrap();
        assert_eq!(h.flags, FLAG_ACK_REPLY);
    }

    #[test]
    fn ack_coverage_handles_wrap_around() {
        assert!(is_covered_by_ack(10, 10));
        assert!(is_covered_by_ack(10, 9));
        assert!(!is_covered_by_ack(10, 11));
        // 5 was sent after 32766, 32767, 0, 1, …
        assert!(is_covered_by_ack(5, 32766));
        assert!(!is_covered_by_ack(32766, 5));
    }

    #[test]
    fn splits_commands() {
        let mut payload = command("PrgI", &[0, 0, 0, 3]);
        payload.extend(command("InCm", &[]));
        payload.extend(command("PrvI", &[0, 0, 0, 4]));
        let list = commands(&payload);
        assert_eq!(
            list,
            vec![
                ("PrgI".to_string(), &[0u8, 0, 0, 3][..]),
                ("InCm".to_string(), &[][..]),
                ("PrvI".to_string(), &[0u8, 0, 0, 4][..]),
            ]
        );
    }

    #[test]
    fn stops_at_broken_commands() {
        let mut payload = command("PrgI", &[0, 0, 0, 3]);
        payload.extend([0, 99, 0, 0, b'X', b'X', b'X', b'X', 1]); // claims to be longer than it is
        assert_eq!(commands(&payload).len(), 1);
        assert_eq!(commands(&[0, 4, 0, 0, b'X', b'X', b'X', b'X', 1]).len(), 0);
    }
}
