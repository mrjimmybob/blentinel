//! The `database` check (M5-10, D-88): does a database server accept
//! connections?
//!
//! An open port doesn't say so: a server that is starting, in recovery or
//! out of connections still accepts TCP. So the probe speaks the start of
//! the server's own protocol and wants a proper answer to it. It never logs
//! in (it has no account), reads no data and changes nothing.
//!
//! **One module per engine**, each with one function, `greet`: given the
//! open connection, say what answered ([`Answer`]). To add an engine: a
//! variant of [`DbEngine`] in `common`, a module here, and its arm in
//! [`greet`] (the compiler asks for it). Timeouts, the connection, the
//! messages and the latency are shared, here.

mod mongodb;
mod mssql;
mod mysql;
mod postgres;

use std::time::{Duration, Instant};

use common::checks::DbEngine;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpStream;

use super::certificate::Trust;
use super::{Outcome, TCP_TIMEOUT};

/// How long the server has for the whole greeting once the port is open.
const GREETING_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest message of a greeting the probe reads (they are small; a
/// length beyond this isn't one).
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// What a server said to the greeting of its protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Answer {
    /// It accepts connections: what it is, for the message
    /// (`PostgreSQL accepts connections`).
    Ready(String),
    /// It is that kind of server and it answered, to say it can't serve
    /// now (starting up, too many connections): its own words.
    Refusing(String),
}

/// `Err`: no answer of that protocol (something else listens there, or it
/// hung up), in words for the message.
type Greeted = Result<Answer, String>;

/// Greet the server at `host:port` in `engine`'s protocol.
pub async fn check(engine: DbEngine, host: &str, port: Option<u16>, trust: &Trust) -> Outcome {
    let port = port.unwrap_or(engine.default_port());
    ask(engine, host, port, trust, GREETING_TIMEOUT).await
}

async fn ask(engine: DbEngine, host: &str, port: u16, trust: &Trust, timeout: Duration) -> Outcome {
    let name = engine.label();
    let start = Instant::now();
    let stream = match tokio::time::timeout(TCP_TIMEOUT, TcpStream::connect((host, port))).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Outcome::down(format!("Connection failed: {e}")),
        Err(_) => return Outcome::down("Connection timeout"),
    };
    match tokio::time::timeout(timeout, greet(engine, stream, host, trust)).await {
        Ok(Ok(Answer::Ready(what))) => Outcome::up(what).with_latency(start.elapsed()),
        Ok(Ok(Answer::Refusing(why))) => {
            Outcome::down(format!("{name} answers, but refuses connections: {why}"))
        }
        Ok(Err(why)) => Outcome::down(format!("The port is open, but {name} didn't answer: {why}")),
        Err(_) => Outcome::down(format!(
            "The port is open, but {name} didn't answer within {} s",
            timeout.as_secs_f32()
        )),
    }
}

/// The greeting of each engine.
async fn greet(engine: DbEngine, stream: TcpStream, host: &str, trust: &Trust) -> Greeted {
    match engine {
        DbEngine::Postgres => postgres::greet(stream, host, trust).await,
        DbEngine::Mysql => mysql::greet(stream).await,
        DbEngine::Mssql => mssql::greet(stream).await,
        DbEngine::Mongodb => mongodb::greet(stream).await,
    }
}

// --- What the engines share ---------------------------------------------------

/// Why a read or a write failed, in words: mostly, the server hung up.
fn broken(error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::BrokenPipe => "it closed the connection".to_string(),
        _ => format!("the connection failed ({error})"),
    }
}

/// The next `len` bytes; a length no greeting has is refused unread.
async fn read_bytes<S: AsyncRead + Unpin>(stream: &mut S, len: usize) -> Result<Vec<u8>, String> {
    if len > MAX_MESSAGE_BYTES {
        return Err("its answer isn't that protocol's".to_string());
    }
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes).await.map_err(broken)?;
    Ok(bytes)
}

/// The text up to the first zero byte of `bytes` (all of it without one),
/// and what follows that byte.
fn c_string(bytes: &[u8]) -> (String, &[u8]) {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let rest = bytes.get(end + 1..).unwrap_or_default();
    (String::from_utf8_lossy(&bytes[..end]).into_owned(), rest)
}

/// A server's own words in a message: one line, not too long.
fn said(text: &str) -> String {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = line.trim();
    match line.char_indices().nth(200) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::public_trust;
    use common::checks::Health;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    /// A server on a local port that reads what a client sends first (if
    /// `reads`), then answers `answer` and hangs up.
    pub(super) async fn server(reads: bool, answer: Vec<u8>) -> u16 {
        server_then(reads, answer, Vec::new()).await
    }

    /// The same, answering `then` to what the client says next.
    pub(super) async fn server_then(reads: bool, answer: Vec<u8>, then: Vec<u8>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (answer, then) = (answer.clone(), then.clone());
                tokio::spawn(async move {
                    if reads {
                        let mut buffer = [0u8; 1024];
                        let _ = socket.read(&mut buffer).await;
                    }
                    let _ = socket.write_all(&answer).await;
                    // Whatever the client says next is read, so that closing
                    // doesn't reset the connection under what was written.
                    let mut buffer = [0u8; 1024];
                    while matches!(socket.read(&mut buffer).await, Ok(n) if n > 0) {
                        if !then.is_empty() {
                            let _ = socket.write_all(&then).await;
                        }
                    }
                });
            }
        });
        port
    }

    pub(super) async fn greeting(engine: DbEngine, port: u16) -> Outcome {
        ask(
            engine,
            "127.0.0.1",
            port,
            &public_trust(),
            Duration::from_millis(500),
        )
        .await
    }

    #[tokio::test]
    async fn nobody_listening_and_a_silent_server_are_down_with_the_reason() {
        let free = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        for engine in DbEngine::ALL {
            let closed = greeting(*engine, free).await;
            assert_eq!(closed.status, Health::Down);
            assert!(
                closed.message.starts_with("Connection"),
                "{}",
                closed.message
            );
        }

        // Something listens, and says nothing.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        for engine in DbEngine::ALL {
            let silent = greeting(*engine, port).await;
            assert_eq!(silent.status, Health::Down);
            assert_eq!(
                silent.message,
                format!(
                    "The port is open, but {} didn't answer within 0.5 s",
                    engine.label()
                )
            );
            assert_eq!(silent.latency_ms, None);
        }
    }

    #[tokio::test]
    async fn another_kind_of_server_is_not_taken_for_a_database() {
        // A web server's answer, whatever it is asked.
        let web = server(true, b"HTTP/1.1 400 Bad Request\r\n\r\n".to_vec()).await;
        // A server that speaks first, as SSH and SMTP do.
        let ssh = server(false, b"SSH-2.0-OpenSSH_9.6\r\n".to_vec()).await;
        for engine in DbEngine::ALL {
            for port in [web, ssh] {
                let outcome = greeting(*engine, port).await;
                assert_eq!(
                    outcome.status,
                    Health::Down,
                    "{engine:?} {}",
                    outcome.message
                );
                assert!(
                    outcome.message.starts_with(&format!(
                        "The port is open, but {} didn't answer",
                        engine.label()
                    )),
                    "{}",
                    outcome.message
                );
            }
        }
    }

    #[test]
    fn a_servers_words_are_one_line_and_text_ends_at_a_zero() {
        assert_eq!(said(" too many\r\nconnections \0"), "too many  connections");
        assert_eq!(said(&"x".repeat(500)).chars().count(), 201);
        assert_eq!(
            c_string(b"8.4.2\0rest"),
            ("8.4.2".to_string(), &b"rest"[..])
        );
        assert_eq!(c_string(b"no end"), ("no end".to_string(), &b""[..]));
    }

    /// Real servers, run by hand (`--ignored`): for each engine to try,
    /// `DB_TEST_POSTGRES`, `DB_TEST_MYSQL`, `DB_TEST_MARIADB`,
    /// `DB_TEST_MSSQL`, `DB_TEST_MONGODB` hold its `host:port` (D-88).
    /// `DB_TEST_TIMES` greets each that often, and nothing else: to see
    /// what a server's log and counters make of the probe.
    #[tokio::test]
    #[ignore = "needs real database servers: see the comment"]
    async fn real_servers_accept_connections() {
        let mut tried = 0;
        let times: Option<usize> = std::env::var("DB_TEST_TIMES")
            .ok()
            .and_then(|times| times.parse().ok());
        for (variable, engine) in [
            ("DB_TEST_POSTGRES", DbEngine::Postgres),
            ("DB_TEST_MYSQL", DbEngine::Mysql),
            ("DB_TEST_MARIADB", DbEngine::Mysql),
            ("DB_TEST_MSSQL", DbEngine::Mssql),
            ("DB_TEST_MONGODB", DbEngine::Mongodb),
        ] {
            let Some(address) = std::env::var(variable).ok().filter(|v| !v.is_empty()) else {
                continue;
            };
            let (host, port) = address.rsplit_once(':').expect("host:port");
            let port: u16 = port.parse().expect("a port");
            tried += 1;
            let outcome = check(engine, host, Some(port), &public_trust()).await;
            println!("{variable}: {:?} {}", outcome.status, outcome.message);
            assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
            assert!(outcome.latency_ms.is_some());
            if let Some(times) = times {
                for _ in 1..times {
                    let again = check(engine, host, Some(port), &public_trust()).await;
                    assert_eq!(again.status, Health::Up, "{}", again.message);
                }
                continue;
            }
            // The other engines' greetings find no server of theirs there.
            for other in DbEngine::ALL.iter().filter(|other| **other != engine) {
                let wrong = check(*other, host, Some(port), &public_trust()).await;
                println!("  as {}: {}", other.label(), wrong.message);
                assert_eq!(wrong.status, Health::Down, "{}", wrong.message);
            }
        }
        assert!(tried > 0, "set at least one DB_TEST_… variable");
    }
}
