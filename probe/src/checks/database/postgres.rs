//! PostgreSQL's greeting: ask for TLS as its clients do, then send a
//! startup message and read the first answer, as `pg_isready` does.
//!
//! The server asks for a password (it accepts connections), or says why it
//! doesn't take this one. "Starting up", "shutting down", "in recovery" and
//! "too many clients" mean it serves nobody; a refusal of this probe alone
//! (no `pg_hba.conf` entry, an unknown role) is still a server at work.

use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::{Answer, Greeted, broken, c_string, read_bytes, said};
use crate::checks::certificate::{Trust, unjudging_connector};

/// "Will you speak TLS?" (`SSLRequest`): a length of 8 and a magic number.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 0x04, 0xD2, 0x16, 0x2F];
/// Protocol 3.0.
const PROTOCOL: [u8; 4] = [0, 3, 0, 0];
/// The role the startup message names. It needn't exist: the server asks
/// for a password before it says whether it does.
const USER: &str = "blentinel";

/// The startup message: a length, the protocol, and settings as pairs of
/// zero-terminated texts.
fn startup() -> Vec<u8> {
    let mut body = PROTOCOL.to_vec();
    for text in [
        "user",
        USER,
        "database",
        "postgres",
        "application_name",
        "blentinel",
    ] {
        body.extend(text.as_bytes());
        body.push(0);
    }
    body.push(0);
    let mut message = ((body.len() + 4) as u32).to_be_bytes().to_vec();
    message.extend(body);
    message
}

/// The fields of an `ErrorResponse`: its SQLSTATE and its message.
fn error_fields(mut body: &[u8]) -> (String, String) {
    let (mut state, mut message) = (String::new(), String::new());
    while let Some((kind, rest)) = body.split_first() {
        if *kind == 0 {
            break;
        }
        let (text, rest) = c_string(rest);
        match kind {
            b'C' => state = text,
            b'M' => message = text,
            _ => {}
        }
        body = rest;
    }
    (state, message)
}

/// What an error says about the server: it serves nobody now, or it is
/// at work and turns this probe away.
fn judge_error(state: &str, message: &str, secured: &str) -> Answer {
    // 57P03: cannot connect now (starting up, shutting down, in recovery).
    // Class 53: out of resources (too many connections, disk, memory).
    if state == "57P03" || state.starts_with("53") {
        Answer::Refusing(said(message))
    } else {
        Answer::Ready(format!(
            "PostgreSQL accepts connections{secured} (it turns this probe away: {})",
            said(message)
        ))
    }
}

/// Send the startup message and read what the server makes of it.
async fn start<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, secured: &str) -> Greeted {
    stream.write_all(&startup()).await.map_err(broken)?;
    // A notice may come first; the answer is among the first messages.
    for _ in 0..4 {
        let mut header = [0u8; 5];
        stream.read_exact(&mut header).await.map_err(broken)?;
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if len < 4 {
            break;
        }
        let body = read_bytes(stream, len - 4).await?;
        let answer = match header[0] {
            // An authentication request: it accepts connections.
            b'R' => Answer::Ready(format!("PostgreSQL accepts connections{secured}")),
            b'E' => {
                let (state, message) = error_fields(&body);
                judge_error(&state, &message, secured)
            }
            b'N' => continue,
            _ => break,
        };
        // The probe then only hangs up. A server takes that quietly while
        // it waits for a password; a "Terminate" message there is logged
        // as a fatal error.
        return Ok(answer);
    }
    Err("its answer isn't PostgreSQL's".to_string())
}

pub(super) async fn greet(mut stream: TcpStream, host: &str, trust: &Trust) -> Greeted {
    stream.write_all(&SSL_REQUEST).await.map_err(broken)?;
    let mut answer = [0u8; 1];
    stream.read_exact(&mut answer).await.map_err(broken)?;
    match answer[0] {
        // It speaks TLS: what follows goes through it, as its clients' does
        // (a server may take nothing else).
        b'S' => {
            let name = ServerName::try_from(host.to_string())
                .map_err(|_| "the host isn't a name or an IP address".to_string())?;
            let mut tls = unjudging_connector(trust)?
                .connect(name, stream)
                .await
                .map_err(|e| format!("it offered TLS, and the handshake failed ({e})"))?;
            let answer = start(&mut tls, " (over TLS)").await;
            // TLS has its own goodbye; a server logs an error when the
            // connection just ends under it.
            let _ = tls.shutdown().await;
            answer
        }
        b'N' => start(&mut stream, "").await,
        // A server from before TLS answers with an error at once.
        b'E' => Err("it answered the TLS question with an error".to_string()),
        _ => Err("its answer isn't PostgreSQL's".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::greeting;
    use super::*;
    use common::checks::{DbEngine, Health};
    use tokio::net::TcpListener;

    fn message(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut message = vec![kind];
        message.extend(((body.len() + 4) as u32).to_be_bytes());
        message.extend(body);
        message
    }

    fn error(state: &str, text: &str) -> Vec<u8> {
        let mut body = Vec::new();
        for (kind, value) in [(b'S', "FATAL"), (b'C', state), (b'M', text)] {
            body.push(kind);
            body.extend(value.as_bytes());
            body.push(0);
        }
        body.push(0);
        message(b'E', &body)
    }

    /// A PostgreSQL server without TLS: `N` to the TLS question, then
    /// `answer` to the startup message. Returns its port.
    async fn postgres(answer: Vec<u8>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let answer = answer.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 8];
                    if socket.read_exact(&mut request).await.is_err() || request != SSL_REQUEST {
                        return;
                    }
                    let _ = socket.write_all(b"N").await;
                    let mut buffer = [0u8; 256];
                    let Ok(read) = socket.read(&mut buffer).await else {
                        return;
                    };
                    // A startup message of protocol 3.0 that names its user.
                    let startup = &buffer[..read];
                    if startup.get(4..8) != Some(&PROTOCOL[..])
                        || !startup.windows(USER.len()).any(|w| w == USER.as_bytes())
                    {
                        return;
                    }
                    let _ = socket.write_all(&answer).await;
                    while matches!(socket.read(&mut buffer).await, Ok(n) if n > 0) {}
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn a_server_that_asks_for_a_password_accepts_connections() {
        // AuthenticationSASL (10), naming a mechanism.
        let port = postgres(message(b'R', b"\0\0\0\x0aSCRAM-SHA-256\0\0")).await;
        let outcome = greeting(DbEngine::Postgres, port).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(outcome.message, "PostgreSQL accepts connections");
        assert!(outcome.latency_ms.is_some());

        // A notice before it changes nothing.
        let mut answer = message(b'N', b"SNOTICE\0\0");
        answer.extend(message(b'R', &[0, 0, 0, 5, 1, 2, 3, 4]));
        let noticed = greeting(DbEngine::Postgres, postgres(answer).await).await;
        assert_eq!(noticed.status, Health::Up, "{}", noticed.message);
    }

    #[tokio::test]
    async fn a_server_that_serves_nobody_is_down_and_one_that_turns_the_probe_away_is_up() {
        for (state, text) in [
            ("57P03", "the database system is starting up"),
            ("53300", "sorry, too many clients already"),
        ] {
            let outcome = greeting(DbEngine::Postgres, postgres(error(state, text)).await).await;
            assert_eq!(outcome.status, Health::Down);
            assert_eq!(
                outcome.message,
                format!("PostgreSQL answers, but refuses connections: {text}")
            );
        }
        for (state, text) in [
            (
                "28000",
                "no pg_hba.conf entry for host \"10.0.0.9\", user \"blentinel\"",
            ),
            ("28000", "role \"blentinel\" does not exist"),
            ("3D000", "database \"postgres\" does not exist"),
        ] {
            let outcome = greeting(DbEngine::Postgres, postgres(error(state, text)).await).await;
            assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
            assert_eq!(
                outcome.message,
                format!("PostgreSQL accepts connections (it turns this probe away: {text})")
            );
        }
    }

    #[test]
    fn the_startup_message_is_its_length_the_protocol_and_pairs() {
        let message = startup();
        assert_eq!(
            u32::from_be_bytes(message[..4].try_into().unwrap()) as usize,
            message.len()
        );
        assert_eq!(message[4..8], PROTOCOL);
        assert!(message.ends_with(b"application_name\0blentinel\0\0"));
        assert_eq!(
            error_fields(b"SFATAL\0C57P03\0Mstarting up\0\0"),
            ("57P03".to_string(), "starting up".to_string())
        );
        assert_eq!(error_fields(b""), (String::new(), String::new()));
    }
}
