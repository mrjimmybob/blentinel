//! MySQL's and MariaDB's greeting: the server speaks first, with its
//! version, or with why it takes no connection.
//!
//! The probe then answers as a client would, with a login that names a user
//! and no password, and reads the refusal. It has no account and wants none:
//! a client that hangs up after the greeting counts as a failed connection,
//! and a server blocks a host that does that too often
//! (`max_connect_errors`). A login that is refused does not count.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::{Answer, Greeted, broken, c_string, read_bytes, said};

/// The greeting's protocol version, and the first byte of an error, of an
/// "all is well" and of a request to authenticate another way.
const HANDSHAKE_V10: u8 = 10;
const ERROR: u8 = 0xFF;
const OK: u8 = 0x00;
const AUTH_SWITCH: u8 = 0xFE;
/// The user the login names. It needn't exist.
const USER: &str = "blentinel";
/// What the login says the client can do: the 4.1 protocol, long
/// passwords, secure authentication, authentication plugins.
const CAPABILITIES: u32 = 0x0000_0001 | 0x0000_0200 | 0x0000_8000 | 0x0008_0000;
/// Errors that are about this probe's address, not about the server:
/// "host is not allowed to connect" and "host is blocked".
const HOST_NOT_ALLOWED: u16 = 1130;
const HOST_BLOCKED: u16 = 1129;
/// How many messages of the login are read before the probe leaves, and
/// how long it waits for them: a server slow to refuse has still greeted.
const LOGIN_ROUNDS: usize = 4;
const LOGIN_TIMEOUT: Duration = Duration::from_millis(if cfg!(test) { 100 } else { 2000 });

/// One packet: its sequence number and its content.
async fn read_packet(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), String> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.map_err(broken)?;
    let len = usize::from(header[0]) | usize::from(header[1]) << 8 | usize::from(header[2]) << 16;
    Ok((header[3], read_bytes(stream, len).await?))
}

fn packet(sequence: u8, content: &[u8]) -> Vec<u8> {
    let len = content.len();
    let mut packet = vec![len as u8, (len >> 8) as u8, (len >> 16) as u8, sequence];
    packet.extend(content);
    packet
}

/// An error packet's number and text (after the first byte).
fn error(content: &[u8]) -> (u16, String) {
    let code = u16::from_le_bytes([
        content.first().copied().unwrap_or_default(),
        content.get(1).copied().unwrap_or_default(),
    ]);
    let text = content.get(2..).unwrap_or_default();
    // Once a login is under way, an SQL state comes before the text: `#28000`.
    let text = match text.first() {
        Some(b'#') => text.get(6..).unwrap_or_default(),
        _ => text,
    };
    (code, String::from_utf8_lossy(text).into_owned())
}

/// Whose server it is, with its version: MariaDB says `5.5.5-10.11.6-MariaDB`
/// or `11.4.2-MariaDB-ubu2404`, MySQL `8.4.2`.
fn product(version: &str) -> String {
    match version.split_once("-MariaDB") {
        Some((number, _)) => format!(
            "MariaDB {}",
            number.strip_prefix("5.5.5-").unwrap_or(number)
        ),
        None => format!("MySQL {version}"),
    }
}

/// The authentication plugin a greeting names (what follows its version),
/// if it names one.
fn plugin(after_version: &[u8]) -> Option<String> {
    // Thread ID (4), salt (8), filler (1), capabilities (2), character set
    // (1), status (2), capabilities (2), salt length (1), reserved (10).
    let salt_len = usize::from(*after_version.get(20)?);
    let rest = after_version.get(31 + salt_len.saturating_sub(8).max(13)..)?;
    let (name, _) = c_string(rest);
    (!name.is_empty() && name.is_ascii()).then_some(name)
}

/// The login: who the client is and what it can do, with no password.
fn login(plugin: &str) -> Vec<u8> {
    let mut content = CAPABILITIES.to_le_bytes().to_vec();
    // The largest packet (16 MB), the character set (utf8mb4), 23 unused bytes.
    content.extend(0x0100_0000u32.to_le_bytes());
    content.push(45);
    content.extend([0; 23]);
    content.extend(USER.as_bytes());
    content.push(0);
    // The answer to the server's challenge: none, as without a password.
    content.push(0);
    content.extend(plugin.as_bytes());
    content.push(0);
    content
}

/// Answer the greeting as a client that has no password, until the server
/// refuses it (or, with such an account, lets it in), then leave. What
/// goes wrong here changes nothing: the server has greeted.
async fn leave_properly(stream: &mut TcpStream, plugin: &str) {
    if stream.write_all(&packet(1, &login(plugin))).await.is_err() {
        return;
    }
    for _ in 0..LOGIN_ROUNDS {
        let Ok((sequence, content)) = read_packet(stream).await else {
            return;
        };
        match content.first() {
            // "Authenticate this way instead": again without a password.
            Some(&AUTH_SWITCH) => {
                if stream
                    .write_all(&packet(sequence.wrapping_add(1), &[]))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            // Let in (an account without a password by that name): quit.
            Some(&OK) => {
                let _ = stream.write_all(&packet(0, &[0x01])).await;
                return;
            }
            // Refused, as expected; or something only a real login goes on from.
            _ => return,
        }
    }
}

pub(super) async fn greet(mut stream: TcpStream) -> Greeted {
    let (_, content) = read_packet(&mut stream).await?;
    match content.split_first() {
        Some((&ERROR, rest)) => {
            let (code, text) = error(rest);
            let text = said(&text);
            Ok(match code {
                HOST_NOT_ALLOWED | HOST_BLOCKED => Answer::Ready(format!(
                    "MySQL or MariaDB accepts connections (it turns this probe away: {text})"
                )),
                _ => Answer::Refusing(format!("{text} (error {code})")),
            })
        }
        Some((&HANDSHAKE_V10, rest)) => {
            let (version, after) = c_string(rest);
            if version.is_empty() || !version.is_ascii() {
                return Err("its greeting isn't MySQL's".to_string());
            }
            let plugin = plugin(after).unwrap_or_else(|| "mysql_native_password".to_string());
            let _ = tokio::time::timeout(LOGIN_TIMEOUT, leave_properly(&mut stream, &plugin)).await;
            Ok(Answer::Ready(format!(
                "{} accepts connections",
                product(&said(&version))
            )))
        }
        _ => Err("its greeting isn't MySQL's".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{greeting, server, server_then};
    use super::*;
    use common::checks::{DbEngine, Health};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    /// A server's greeting: its version and what a real one sends after it.
    fn handshake(version: &str, plugin: &str) -> Vec<u8> {
        let mut content = vec![HANDSHAKE_V10];
        content.extend(version.as_bytes());
        content.push(0);
        content.extend([7, 0, 0, 0]); // thread ID
        content.extend(b"saltsalt\0"); // salt, filler
        content.extend([0xff, 0xff, 45, 2, 0, 0xff, 0xdf, 21]); // capabilities, charset, status, salt length
        content.extend([0; 10]);
        content.extend(b"saltsaltsalt\0");
        content.extend(plugin.as_bytes());
        content.push(0);
        packet(0, &content)
    }

    fn refusal(code: u16, text: &str) -> Vec<u8> {
        let mut content = vec![ERROR];
        content.extend(code.to_le_bytes());
        content.extend(text.as_bytes());
        packet(0, &content)
    }

    #[tokio::test]
    async fn a_greeting_names_the_server_and_its_version() {
        for (version, message) in [
            ("8.4.2", "MySQL 8.4.2 accepts connections"),
            (
                "5.5.5-10.11.6-MariaDB-1:10.11.6+maria~ubu2204",
                "MariaDB 10.11.6 accepts connections",
            ),
            (
                "11.4.2-MariaDB-ubu2404",
                "MariaDB 11.4.2 accepts connections",
            ),
        ] {
            let mut denied = vec![ERROR];
            denied.extend(1045u16.to_le_bytes());
            denied.extend(b"#28000Access denied");
            let greets = handshake(version, "caching_sha2_password");
            let port = server_then(false, greets.clone(), packet(2, &denied)).await;
            let outcome = greeting(DbEngine::Mysql, port).await;
            assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
            assert_eq!(outcome.message, message);
            // A server that never answers the login has greeted all the same.
            let slow = greeting(DbEngine::Mysql, server(false, greets).await).await;
            assert_eq!(slow.message, message);
        }
    }

    #[tokio::test]
    async fn a_server_that_takes_no_connection_says_why() {
        let full = server(false, refusal(1040, "Too many connections")).await;
        let outcome = greeting(DbEngine::Mysql, full).await;
        assert_eq!(outcome.status, Health::Down);
        assert_eq!(
            outcome.message,
            "MySQL or MariaDB answers, but refuses connections: Too many connections (error 1040)"
        );
        // Turned away for its address: the server is at work.
        let text = "Host '10.0.0.9' is not allowed to connect to this MySQL server";
        let outcome = greeting(DbEngine::Mysql, server(false, refusal(1130, text)).await).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(
            outcome.message,
            format!("MySQL or MariaDB accepts connections (it turns this probe away: {text})")
        );
    }

    /// The probe answers the greeting with a login and stays for the
    /// refusal, so that the server counts no failed connection.
    #[tokio::test]
    async fn the_probe_leaves_as_a_client_that_was_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let greeting = handshake("8.4.2", "caching_sha2_password");
            socket.write_all(&greeting).await.unwrap();
            // The login, then "authenticate with another plugin".
            let (sequence, login) = read_packet(&mut socket).await.unwrap();
            record.lock().unwrap().push((sequence, login));
            let mut switch = vec![AUTH_SWITCH];
            switch.extend(b"mysql_native_password\0saltsaltsaltsaltsalt\0");
            socket.write_all(&packet(2, &switch)).await.unwrap();
            let (sequence, answer) = read_packet(&mut socket).await.unwrap();
            record.lock().unwrap().push((sequence, answer));
            let mut denied = vec![ERROR];
            denied.extend(1045u16.to_le_bytes());
            denied.extend(b"#28000Access denied for user 'blentinel'@'10.0.0.9'");
            socket.write_all(&packet(4, &denied)).await.unwrap();
            // The client closes: nothing more comes.
            let mut rest = Vec::new();
            let _ = socket.read_to_end(&mut rest).await;
            record.lock().unwrap().push((0xEE, rest));
        });
        let outcome = greeting(DbEngine::Mysql, port).await;
        assert_eq!(outcome.message, "MySQL 8.4.2 accepts connections");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "a login, an answer, the end");
        let (sequence, sent) = &seen[0];
        assert_eq!(*sequence, 1);
        assert_eq!(sent[..4], CAPABILITIES.to_le_bytes());
        // The user, no password, and the plugin the server named.
        assert!(sent.ends_with(b"blentinel\0\0caching_sha2_password\0"));
        assert_eq!(seen[1], (3, vec![]), "no password this way either");
        assert_eq!(seen[2], (0xEE, vec![]), "then it only closes");
    }

    #[test]
    fn packets_errors_and_plugins_are_read_as_mysql_writes_them() {
        assert_eq!(packet(3, b"abc"), [3, 0, 0, 3, b'a', b'b', b'c']);
        assert_eq!(
            error(&[0x15, 0x04, b'#', b'2', b'8', b'0', b'0', b'0', b'n', b'o']),
            (1045, "no".to_string())
        );
        assert_eq!(
            error(&[0x10, 0x04, b'f', b'u', b'l', b'l']),
            (1040, "full".to_string())
        );
        assert_eq!(error(&[]), (0, String::new()));
        let greeting = handshake("8.4.2", "caching_sha2_password");
        let (_, after) = c_string(&greeting[5..]);
        assert_eq!(plugin(after).as_deref(), Some("caching_sha2_password"));
        assert_eq!(plugin(&after[..10]), None, "an old, short greeting");
        assert_eq!(
            product("8.0.36-0ubuntu0.22.04.1"),
            "MySQL 8.0.36-0ubuntu0.22.04.1"
        );
    }
}
