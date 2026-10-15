//! SQL Server's greeting: the TDS pre-login, which every client sends
//! before it logs in. The answer carries the server's version.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::{Answer, Greeted, broken, read_bytes};

/// TDS packet types: a pre-login, and a server's answer.
const PRELOGIN: u8 = 0x12;
const RESPONSE: u8 = 0x04;
/// The pre-login option that holds the version, and the end of the options.
const VERSION: u8 = 0x00;
const TERMINATOR: u8 = 0xFF;
/// "I don't encrypt" (the server's answer says what it wants; nothing
/// further is sent anyway).
const ENCRYPT_NOT_SUPPORTED: u8 = 0x02;

/// The pre-login packet: a header, the options (what, where, how long),
/// then their values.
fn prelogin() -> Vec<u8> {
    // Version, encryption, instance, thread ID, MARS.
    let values: [&[u8]; 5] = [&[0; 6], &[ENCRYPT_NOT_SUPPORTED], &[0], &[0; 4], &[0]];
    let mut options = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    let mut offset = values.len() * 5 + 1;
    for (token, value) in values.iter().enumerate() {
        options.push(token as u8);
        options.extend((offset as u16).to_be_bytes());
        options.extend((value.len() as u16).to_be_bytes());
        data.extend(*value);
        offset += value.len();
    }
    options.push(TERMINATOR);
    let len = 8 + options.len() + data.len();
    // Type, "last packet", length, then channel, packet number and window.
    let mut packet = vec![PRELOGIN, 0x01];
    packet.extend((len as u16).to_be_bytes());
    packet.extend([0, 0, 1, 0]);
    packet.extend(options);
    packet.extend(data);
    packet
}

/// The version in a pre-login answer: major, minor, build.
fn version(payload: &[u8]) -> Option<(u8, u8, u16)> {
    let mut options = payload;
    while let Some((&token, rest)) = options.split_first() {
        if token == TERMINATOR {
            return None;
        }
        let offset = usize::from(u16::from_be_bytes([*rest.first()?, *rest.get(1)?]));
        let len = usize::from(u16::from_be_bytes([*rest.get(2)?, *rest.get(3)?]));
        if token == VERSION {
            let value = payload.get(offset..offset + len)?;
            return Some((
                *value.first()?,
                *value.get(1)?,
                u16::from_be_bytes([*value.get(2)?, *value.get(3)?]),
            ));
        }
        options = rest.get(4..)?;
    }
    None
}

/// The release a version number is of, as people call it.
fn release(major: u8) -> Option<&'static str> {
    Some(match major {
        17 => "2025",
        16 => "2022",
        15 => "2019",
        14 => "2017",
        13 => "2016",
        12 => "2014",
        11 => "2012",
        10 => "2008",
        9 => "2005",
        _ => return None,
    })
}

pub(super) async fn greet(mut stream: TcpStream) -> Greeted {
    stream.write_all(&prelogin()).await.map_err(broken)?;
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).await.map_err(broken)?;
    let len = usize::from(u16::from_be_bytes([header[2], header[3]]));
    if header[0] != RESPONSE || len < 8 {
        return Err("its answer isn't SQL Server's".to_string());
    }
    let payload = read_bytes(&mut stream, len - 8).await?;
    let Some((major, minor, build)) = version(&payload) else {
        return Err("its answer isn't SQL Server's".to_string());
    };
    Ok(Answer::Ready(match release(major) {
        Some(release) => {
            format!("SQL Server {release} ({major}.{minor}.{build}) accepts connections")
        }
        None => format!("SQL Server {major}.{minor}.{build} accepts connections"),
    }))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{greeting, server};
    use super::*;
    use common::checks::{DbEngine, Health};

    /// A pre-login answer of a server of this version.
    fn answer(major: u8, minor: u8, build: u16) -> Vec<u8> {
        // Two options: the version (6 bytes) and encryption (1 byte).
        let mut payload = vec![VERSION, 0, 11, 0, 6, 0x01, 0, 17, 0, 1, TERMINATOR];
        payload.extend([major, minor]);
        payload.extend(build.to_be_bytes());
        payload.extend([0, 0, ENCRYPT_NOT_SUPPORTED]);
        let mut packet = vec![RESPONSE, 0x01];
        packet.extend(((payload.len() + 8) as u16).to_be_bytes());
        packet.extend([0, 0, 1, 0]);
        packet.extend(payload);
        packet
    }

    #[tokio::test]
    async fn a_server_answers_the_pre_login_with_its_version() {
        let port = server(true, answer(16, 0, 4135)).await;
        let outcome = greeting(DbEngine::Mssql, port).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(
            outcome.message,
            "SQL Server 2022 (16.0.4135) accepts connections"
        );
        let unknown = greeting(DbEngine::Mssql, server(true, answer(42, 1, 7)).await).await;
        assert_eq!(unknown.message, "SQL Server 42.1.7 accepts connections");
    }

    #[test]
    fn the_pre_login_says_where_each_of_its_options_is() {
        let packet = prelogin();
        assert_eq!(packet[0], PRELOGIN);
        assert_eq!(
            usize::from(u16::from_be_bytes([packet[2], packet[3]])),
            packet.len()
        );
        // Five options and the terminator, then 13 bytes of values.
        assert_eq!(packet.len(), 8 + 26 + 13);
        assert_eq!(packet[8 + 25], TERMINATOR);
        // Our own pre-login reads as one: version 0.0.0.
        assert_eq!(version(&packet[8..]), Some((0, 0, 0)));
        assert_eq!(version(&[TERMINATOR]), None);
        assert_eq!(
            version(&[VERSION, 0, 99, 0, 6, TERMINATOR]),
            None,
            "past the end"
        );
        assert_eq!(version(&[]), None);
    }
}
