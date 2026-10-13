//! The `mail` check (M5-16c, D-95): does a mail server answer, in SMTP, IMAP
//! or POP3, and encrypted as it should be?
//!
//! An open port doesn't say so, and neither does a TLS handshake: a mail
//! server that is overloaded, misconfigured or replaced by something else
//! still accepts connections. So the probe speaks the start of the server's
//! own protocol: it waits for the greeting, starts TLS the way the check says
//! (`STARTTLS` on the usual port, TLS from the first byte, or not at all),
//! and says goodbye. It never logs in (it has no account) and sends no mail.
//!
//! Where the connection is encrypted the certificate is judged as the `tls`
//! check judges one, which can't reach a certificate behind `STARTTLS`: an
//! expired or (unless the check says not to verify) untrusted one is Down,
//! and its days left are reported for the hub's limits.

use std::time::{Duration, Instant};

use common::checks::{Health, MailProtocol, MailSecurity};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::certificate::{Handshake, Trust};
use super::database::{broken, said};
use super::{Outcome, TCP_TIMEOUT};

/// How long the server has for the whole exchange once the port is open.
const TALK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long it has to answer the goodbye: a server that doesn't is still up.
const BYE_TIMEOUT: Duration = Duration::from_secs(1);
/// The longest line the probe reads (an IMAP greeting lists what the server
/// can do), and the most lines of one answer.
const MAX_LINE_BYTES: usize = 8 * 1024;
const MAX_LINES: usize = 64;
/// The name the probe gives in SMTP's `EHLO`: one that is nobody's.
const HELLO: &str = "blentinel.invalid";
/// How much of a greeting a message shows: servers add session numbers
/// and more to theirs.
const GREETING_CHARS: usize = 80;
/// The tag of the probe's IMAP commands.
const TAG: &str = "a1";

/// Why a server isn't one that serves mail now.
#[derive(Debug, PartialEq)]
enum Trouble {
    /// No answer of that protocol (something else listens there, or it hung
    /// up).
    Silent(String),
    /// It is that kind of server, and says it can't serve now: its words.
    Refusing(String),
    /// It serves, but not encrypted as the check wants.
    NoTls(String),
}

impl From<String> for Trouble {
    fn from(why: String) -> Self {
        Trouble::Silent(why)
    }
}

/// What a server that serves said, and what its certificate is worth.
#[derive(Debug)]
struct Heard {
    greeting: String,
    certificate: Option<Outcome>,
}

/// A connection read line by line, as all three protocols are spoken.
struct Wire<S> {
    stream: BufReader<S>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Wire<S> {
    fn new(stream: S) -> Self {
        Self {
            stream: BufReader::new(stream),
        }
    }

    /// The next line, without its end.
    async fn line(&mut self) -> Result<String, String> {
        let mut bytes = Vec::new();
        (&mut self.stream)
            .take(MAX_LINE_BYTES as u64)
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(broken)?;
        match bytes.last() {
            None => Err("it closed the connection".to_string()),
            Some(b'\n') => Ok(String::from_utf8_lossy(&bytes).trim_end().to_string()),
            Some(_) if bytes.len() < MAX_LINE_BYTES => Err("it closed the connection".to_string()),
            Some(_) => Err("its answer isn't a mail server's".to_string()),
        }
    }

    async fn send(&mut self, command: &str) -> Result<(), String> {
        self.stream
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .map_err(broken)?;
        self.stream.flush().await.map_err(broken)
    }

    /// The connection itself, to start TLS on. Anything the server sent
    /// after agreeing to TLS and before TLS began would be read as if it
    /// were encrypted: that is refused.
    fn into_plain(self) -> Result<S, Trouble> {
        if !self.stream.buffer().is_empty() {
            return Err(Trouble::NoTls(
                "it sent more before TLS began, which a mail client must not accept".to_string(),
            ));
        }
        Ok(self.stream.into_inner())
    }
}

/// An SMTP answer: its code and the text of each of its lines
/// (`250-mail.example.com`, `250 STARTTLS`).
async fn smtp_reply<S: AsyncRead + AsyncWrite + Unpin>(
    wire: &mut Wire<S>,
) -> Result<(u16, Vec<String>), String> {
    let not_smtp = || "its answer isn't SMTP's".to_string();
    let mut lines = Vec::new();
    loop {
        let line = wire.line().await?;
        let (code, rest) = line.split_at_checked(3).ok_or_else(not_smtp)?;
        if !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(not_smtp());
        }
        let code: u16 = code.parse().map_err(|_| not_smtp())?;
        lines.push(rest.get(1..).unwrap_or_default().trim().to_string());
        match rest.as_bytes().first() {
            Some(b'-') if lines.len() < MAX_LINES => continue,
            None | Some(b' ') => return Ok((code, lines)),
            _ => return Err(not_smtp()),
        }
    }
}

/// `prefix` taken off `line` whatever its case, if what follows is its own
/// word: `+OK ready` without `+OK` is `ready`.
fn after<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let (start, rest) = line.split_at_checked(prefix.len())?;
    (start.eq_ignore_ascii_case(prefix) && (rest.is_empty() || rest.starts_with(' ')))
        .then(|| rest.trim())
}

/// A greeting as a message shows it: without the list of what an IMAP
/// server can do (`[CAPABILITY …]`), and not too long.
fn short(greeting: &str) -> String {
    // The list first: it can be longer than what `said` keeps.
    let greeting = greeting.trim();
    let words = match greeting
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
    {
        Some((_, words)) if !words.trim().is_empty() => words,
        _ => greeting,
    };
    let text = said(words);
    let words = text.as_str();
    match words.char_indices().nth(GREETING_CHARS) {
        Some((at, _)) => format!("{}…", words[..at].trim_end()),
        None => words.to_string(),
    }
}

/// The server's first words: `Ok` with them if it serves.
async fn greeting<S: AsyncRead + AsyncWrite + Unpin>(
    protocol: MailProtocol,
    wire: &mut Wire<S>,
) -> Result<String, Trouble> {
    match protocol {
        MailProtocol::Smtp => {
            let (code, lines) = smtp_reply(wire).await?;
            let text = said(lines.first().map(String::as_str).unwrap_or_default());
            match code {
                220 => Ok(short(lines.first().map(String::as_str).unwrap_or_default())),
                _ => Err(Trouble::Refusing(format!("{code} {text}"))),
            }
        }
        MailProtocol::Imap => {
            let line = wire.line().await?;
            if let Some(text) = after(&line, "* OK").or_else(|| after(&line, "* PREAUTH")) {
                Ok(short(text))
            } else if let Some(text) = after(&line, "* BYE") {
                Err(Trouble::Refusing(said(text)))
            } else {
                Err(Trouble::Silent("its answer isn't IMAP's".to_string()))
            }
        }
        MailProtocol::Pop3 => {
            let line = wire.line().await?;
            if let Some(text) = after(&line, "+OK") {
                Ok(short(text))
            } else if let Some(text) = after(&line, "-ERR") {
                Err(Trouble::Refusing(said(text)))
            } else {
                Err(Trouble::Silent("its answer isn't POP3's".to_string()))
            }
        }
    }
}

/// Ask the server to start TLS on this connection (`STARTTLS`, POP3's
/// `STLS`): `Ok` when it agreed, and TLS is next.
async fn ask_for_tls<S: AsyncRead + AsyncWrite + Unpin>(
    protocol: MailProtocol,
    wire: &mut Wire<S>,
) -> Result<(), Trouble> {
    let refused = |text: &str| match said(text) {
        text if text.is_empty() => Trouble::NoTls("it doesn't offer STARTTLS".to_string()),
        text => Trouble::NoTls(format!("it doesn't offer STARTTLS ({text})")),
    };
    match protocol {
        MailProtocol::Smtp => {
            // What it can do comes in the answer to EHLO.
            wire.send(&format!("EHLO {HELLO}")).await?;
            let (code, lines) = smtp_reply(wire).await?;
            let offered = lines.iter().skip(1).any(|line| {
                line.split_whitespace()
                    .next()
                    .is_some_and(|word| word.eq_ignore_ascii_case("STARTTLS"))
            });
            if code != 250 || !offered {
                return Err(refused(""));
            }
            wire.send("STARTTLS").await?;
            let (code, lines) = smtp_reply(wire).await?;
            match code {
                220 => Ok(()),
                _ => Err(refused(&format!("{code} {}", lines.join(" ")))),
            }
        }
        MailProtocol::Imap => {
            wire.send(&format!("{TAG} STARTTLS")).await?;
            // Lines that aren't the answer to this command may come first.
            for _ in 0..MAX_LINES {
                let line = wire.line().await?;
                let Some(answer) = after(&line, TAG) else {
                    continue;
                };
                return match after(answer, "OK") {
                    Some(_) => Ok(()),
                    None => Err(refused(answer)),
                };
            }
            Err(Trouble::Silent("its answer isn't IMAP's".to_string()))
        }
        MailProtocol::Pop3 => {
            wire.send("STLS").await?;
            let line = wire.line().await?;
            match after(&line, "+OK") {
                Some(_) => Ok(()),
                None => Err(refused(after(&line, "-ERR").unwrap_or(&line))),
            }
        }
    }
}

/// Say goodbye, so the server's log has no connection that just ended. What
/// it answers, and whether it does, doesn't matter.
async fn bye<S: AsyncRead + AsyncWrite + Unpin>(protocol: MailProtocol, wire: &mut Wire<S>) {
    let command = match protocol {
        MailProtocol::Smtp | MailProtocol::Pop3 => "QUIT".to_string(),
        MailProtocol::Imap => format!("{TAG} LOGOUT"),
    };
    let _ = tokio::time::timeout(BYE_TIMEOUT, async {
        wire.send(&command).await?;
        wire.line().await
    })
    .await;
}

/// The whole exchange on an open connection.
async fn talk(
    protocol: MailProtocol,
    security: MailSecurity,
    verify: bool,
    host: &str,
    trust: &Trust,
    stream: TcpStream,
) -> Result<Heard, Trouble> {
    match security {
        MailSecurity::None => {
            let mut wire = Wire::new(stream);
            let greeting = greeting(protocol, &mut wire).await?;
            bye(protocol, &mut wire).await;
            Ok(Heard {
                greeting,
                certificate: None,
            })
        }
        MailSecurity::Tls => {
            let handshake = Handshake::new(trust)?;
            let tls = handshake.connect(host, stream).await?;
            let certificate = Some(handshake.certificate(&tls, verify, host));
            let mut wire = Wire::new(tls);
            let greeting = greeting(protocol, &mut wire).await?;
            bye(protocol, &mut wire).await;
            Ok(Heard {
                greeting,
                certificate,
            })
        }
        MailSecurity::Starttls => {
            let mut wire = Wire::new(stream);
            let greeting = greeting(protocol, &mut wire).await?;
            ask_for_tls(protocol, &mut wire).await?;
            let handshake = Handshake::new(trust)?;
            let tls = handshake
                .connect(host, wire.into_plain()?)
                .await
                .map_err(|why| Trouble::NoTls(format!("TLS didn't start after STARTTLS: {why}")))?;
            let certificate = Some(handshake.certificate(&tls, verify, host));
            bye(protocol, &mut Wire::new(tls)).await;
            Ok(Heard {
                greeting,
                certificate,
            })
        }
    }
}

/// Greet the mail server at `host:port` in `protocol`.
pub async fn check(
    protocol: MailProtocol,
    host: &str,
    port: Option<u16>,
    security: MailSecurity,
    verify: bool,
    trust: &Trust,
) -> Outcome {
    let port = port.unwrap_or(protocol.default_port(security));
    ask(
        (protocol, security, verify),
        host,
        port,
        trust,
        TALK_TIMEOUT,
    )
    .await
}

async fn ask(
    (protocol, security, verify): (MailProtocol, MailSecurity, bool),
    host: &str,
    port: u16,
    trust: &Trust,
    timeout: Duration,
) -> Outcome {
    let name = protocol.label();
    let start = Instant::now();
    let stream = match tokio::time::timeout(TCP_TIMEOUT, TcpStream::connect((host, port))).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Outcome::down(format!("Connection failed: {e}")),
        Err(_) => return Outcome::down("Connection timeout"),
    };
    let talked = tokio::time::timeout(
        timeout,
        talk(protocol, security, verify, host, trust, stream),
    )
    .await;
    let heard = match talked {
        Ok(Ok(heard)) => heard,
        Ok(Err(Trouble::Silent(why))) => {
            return Outcome::down(format!("The port is open, but {name} didn't answer: {why}"));
        }
        Ok(Err(Trouble::Refusing(why))) => {
            return Outcome::down(format!("{name} answers, but refuses: {why}"));
        }
        Ok(Err(Trouble::NoTls(why))) => return Outcome::down(format!("{name} answers, but {why}")),
        Err(_) => {
            return Outcome::down(format!(
                "The port is open, but {name} didn't answer within {} s",
                timeout.as_secs_f32()
            ));
        }
    };
    let elapsed = start.elapsed();
    let how = match security {
        MailSecurity::Starttls => " (STARTTLS)",
        MailSecurity::Tls => " (TLS)",
        MailSecurity::None => "",
    };
    match heard.certificate {
        None => Outcome::up(format!("{name} ready: {}", heard.greeting)),
        Some(certificate) if certificate.status == Health::Down => Outcome::down(format!(
            "{name} answers, but its certificate isn't good: {}",
            certificate.message
        ))
        .with_metrics(certificate.metrics),
        Some(certificate) => Outcome::up(format!(
            "{name} ready{how}: {}. {}",
            heard.greeting.trim_end_matches('.'),
            certificate.message
        ))
        .with_metrics(certificate.metrics),
    }
    .with_latency(elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::certificate::tests::{Authority, acceptor};
    use crate::checks::public_trust;
    use common::checks::metric;
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    const HOST: &str = "127.0.0.1";

    /// What a test server does, step by step.
    #[derive(Clone)]
    enum Step {
        Say(&'static str),
        /// Read one line of the client's.
        Hear,
        /// Start TLS on the connection.
        Tls(TlsAcceptor),
        /// Wait a moment, so that what is said next comes on its own.
        Pause,
    }
    use Step::{Hear, Say};

    trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
    impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

    /// A server that plays `script` to each client, records the lines it
    /// heard, then reads until the client hangs up. Returns its port and
    /// those lines.
    async fn server(script: Vec<Step>) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind((HOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = std::sync::Arc::clone(&heard);
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let (script, record) = (script.clone(), std::sync::Arc::clone(&record));
                tokio::spawn(async move {
                    let mut stream: Box<dyn Duplex> = Box::new(socket);
                    for step in script {
                        match step {
                            Step::Say(text) => {
                                if stream.write_all(text.as_bytes()).await.is_err() {
                                    return;
                                }
                            }
                            // Byte by byte: nothing of what follows a line
                            // (a TLS hello) may be taken with it.
                            Step::Hear => {
                                let mut line = Vec::new();
                                let mut byte = [0u8; 1];
                                while matches!(stream.read(&mut byte).await, Ok(1)) {
                                    line.push(byte[0]);
                                    if byte[0] == b'\n' {
                                        break;
                                    }
                                }
                                let line = String::from_utf8_lossy(&line).trim_end().to_string();
                                record.lock().unwrap().push(line);
                            }
                            Step::Pause => tokio::time::sleep(Duration::from_millis(300)).await,
                            Step::Tls(acceptor) => match acceptor.accept(stream).await {
                                Ok(tls) => stream = Box::new(tls),
                                Err(_) => return,
                            },
                        }
                    }
                    let mut rest = [0u8; 256];
                    while matches!(stream.read(&mut rest).await, Ok(n) if n > 0) {}
                });
            }
        });
        (port, heard)
    }

    async fn asked(
        protocol: MailProtocol,
        security: MailSecurity,
        verify: bool,
        port: u16,
        trust: &Trust,
    ) -> Outcome {
        ask(
            (protocol, security, verify),
            HOST,
            port,
            trust,
            Duration::from_millis(1500),
        )
        .await
    }

    /// A plain server and what a check without TLS says of it.
    async fn plain(protocol: MailProtocol, script: Vec<Step>) -> Outcome {
        let (port, _) = server(script).await;
        asked(protocol, MailSecurity::None, true, port, &public_trust()).await
    }

    /// An authority, and a TLS acceptor with a certificate of its for the
    /// test servers' address.
    fn secured() -> (Authority, TlsAcceptor) {
        let authority = Authority::new();
        let (certificate, key) = authority.issue(HOST, (2099, 1, 1));
        (authority, acceptor(certificate, key))
    }

    #[tokio::test]
    async fn each_protocols_greeting_is_up_with_the_servers_words() {
        let smtp = plain(
            MailProtocol::Smtp,
            vec![
                Say("220 mail.example.com ESMTP Postfix\r\n"),
                Hear,
                Say("221 Bye\r\n"),
            ],
        )
        .await;
        assert_eq!(smtp.status, Health::Up, "{}", smtp.message);
        assert_eq!(smtp.message, "SMTP ready: mail.example.com ESMTP Postfix");
        assert!(smtp.latency_ms.is_some());
        assert!(smtp.metrics.is_empty());

        // A greeting of several lines: the first one is shown.
        let long = plain(
            MailProtocol::Smtp,
            vec![Say(
                "220-mx.example.com ESMTP\r\n220-No spam\r\n220 Go on\r\n",
            )],
        )
        .await;
        assert_eq!(long.message, "SMTP ready: mx.example.com ESMTP");

        let imap = plain(
            MailProtocol::Imap,
            vec![Say(
                "* OK [CAPABILITY IMAP4rev1 STARTTLS] Dovecot ready.\r\n",
            )],
        )
        .await;
        assert_eq!(
            imap.message, "IMAP ready: Dovecot ready.",
            "without the list of what it can do"
        );
        // However long that list is (GMX's is some 230 characters).
        static LISTED: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
            format!(
                "* OK [CAPABILITY IMAP4rev1{}] IMAP server ready H migmx\r\n",
                " THREAD=ORDEREDSUBJECT".repeat(12)
            )
        });
        let listed = plain(MailProtocol::Imap, vec![Say(LISTED.as_str())]).await;
        assert_eq!(listed.message, "IMAP ready: IMAP server ready H migmx");
        // Nothing but that list: it is the greeting.
        let bare = plain(
            MailProtocol::Imap,
            vec![Say("* OK [CAPABILITY IMAP4rev1]\r\n")],
        )
        .await;
        assert_eq!(bare.message, "IMAP ready: [CAPABILITY IMAP4rev1]");
        // A long one is cut.
        let long = plain(
            MailProtocol::Smtp,
            vec![Say("220 MA3P292CA0064.outlook.office365.com Microsoft ESMTP MAIL Service ready at Tue, 13 Oct 2026 11:52:03 +0000 [08DF28BF5A96292F]\r\n")],
        )
        .await;
        assert_eq!(
            long.message,
            "SMTP ready: MA3P292CA0064.outlook.office365.com Microsoft ESMTP MAIL Service ready at Tue, 1…"
        );

        let pop3 = plain(MailProtocol::Pop3, vec![Say("+OK Dovecot ready.\r\n")]).await;
        assert_eq!(pop3.message, "POP3 ready: Dovecot ready.");
    }

    #[tokio::test]
    async fn the_probe_says_goodbye_and_never_logs_in() {
        for (protocol, hello, goodbye) in [
            (MailProtocol::Smtp, "220 mail ESMTP\r\n", "QUIT"),
            (MailProtocol::Imap, "* OK ready\r\n", "a1 LOGOUT"),
            (MailProtocol::Pop3, "+OK ready\r\n", "QUIT"),
        ] {
            let (port, heard) = server(vec![Say(hello), Hear, Say("bye\r\n")]).await;
            let outcome = asked(protocol, MailSecurity::None, true, port, &public_trust()).await;
            assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
            assert_eq!(*heard.lock().unwrap(), [goodbye]);
        }
    }

    #[tokio::test]
    async fn a_server_that_refuses_or_isnt_one_is_down_with_the_reason() {
        let busy = plain(
            MailProtocol::Smtp,
            vec![Say("421 4.3.2 Service not available, try later\r\n")],
        )
        .await;
        assert_eq!(busy.status, Health::Down);
        assert_eq!(
            busy.message,
            "SMTP answers, but refuses: 421 4.3.2 Service not available, try later"
        );
        let bye = plain(
            MailProtocol::Imap,
            vec![Say("* BYE Too many connections\r\n")],
        )
        .await;
        assert_eq!(
            bye.message,
            "IMAP answers, but refuses: Too many connections"
        );
        let locked = plain(MailProtocol::Pop3, vec![Say("-ERR maildrop locked\r\n")]).await;
        assert_eq!(locked.message, "POP3 answers, but refuses: maildrop locked");

        // Something else listens there.
        for (protocol, name) in [
            (MailProtocol::Smtp, "SMTP"),
            (MailProtocol::Imap, "IMAP"),
            (MailProtocol::Pop3, "POP3"),
        ] {
            let other = plain(protocol, vec![Say("SSH-2.0-OpenSSH_9.6\r\n")]).await;
            assert_eq!(other.status, Health::Down);
            assert_eq!(
                other.message,
                format!("The port is open, but {name} didn't answer: its answer isn't {name}'s")
            );
        }
        // `+OKAY` isn't `+OK`.
        let near = plain(MailProtocol::Pop3, vec![Say("+OKAY\r\n")]).await;
        assert_eq!(near.status, Health::Down);

        // It says nothing; it hangs up; nobody listens.
        let (port, _) = server(vec![Hear]).await;
        let silent = asked(
            MailProtocol::Smtp,
            MailSecurity::None,
            true,
            port,
            &public_trust(),
        )
        .await;
        assert_eq!(
            silent.message,
            "The port is open, but SMTP didn't answer within 1.5 s"
        );
        let listener = TcpListener::bind((HOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                drop(socket);
            }
        });
        let closed = asked(
            MailProtocol::Imap,
            MailSecurity::None,
            true,
            port,
            &public_trust(),
        )
        .await;
        assert!(
            closed
                .message
                .starts_with("The port is open, but IMAP didn't answer: "),
            "{}",
            closed.message
        );
        let free = {
            let listener = TcpListener::bind((HOST, 0)).await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let nobody = asked(
            MailProtocol::Pop3,
            MailSecurity::None,
            true,
            free,
            &public_trust(),
        )
        .await;
        assert!(
            nobody.message.starts_with("Connection"),
            "{}",
            nobody.message
        );
    }

    #[tokio::test]
    async fn starttls_is_asked_for_in_each_protocol_and_the_certificate_judged() {
        let (authority, tls) = secured();
        let scripts = [
            (
                MailProtocol::Smtp,
                vec![
                    Say("220 mail.example.com ESMTP Postfix\r\n"),
                    Hear,
                    Say(
                        "250-mail.example.com\r\n250-PIPELINING\r\n250-starttls\r\n250 8BITMIME\r\n",
                    ),
                    Hear,
                    Say("220 2.0.0 Ready to start TLS\r\n"),
                    Step::Tls(tls.clone()),
                    Hear,
                    Say("221 Bye\r\n"),
                ],
                vec!["EHLO blentinel.invalid", "STARTTLS", "QUIT"],
                "SMTP ready (STARTTLS): mail.example.com ESMTP Postfix. Certificate expires in ",
            ),
            (
                MailProtocol::Imap,
                vec![
                    Say("* OK Dovecot ready.\r\n"),
                    Hear,
                    Say("* CAPABILITY IMAP4rev1\r\na1 OK Begin TLS negotiation now.\r\n"),
                    Step::Tls(tls.clone()),
                    Hear,
                    Say("* BYE\r\n"),
                ],
                vec!["a1 STARTTLS", "a1 LOGOUT"],
                "IMAP ready (STARTTLS): Dovecot ready. Certificate expires in ",
            ),
            (
                MailProtocol::Pop3,
                vec![
                    Say("+OK POP3 ready\r\n"),
                    Hear,
                    Say("+OK Begin TLS\r\n"),
                    Step::Tls(tls.clone()),
                    Hear,
                    Say("+OK bye\r\n"),
                ],
                vec!["STLS", "QUIT"],
                "POP3 ready (STARTTLS): POP3 ready. Certificate expires in ",
            ),
        ];
        for (protocol, script, said, message) in scripts {
            let (port, heard) = server(script).await;
            let outcome = asked(
                protocol,
                MailSecurity::Starttls,
                true,
                port,
                &authority.trust(),
            )
            .await;
            assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
            assert!(outcome.message.starts_with(message), "{}", outcome.message);
            assert!(
                outcome
                    .message
                    .ends_with("(2099-01-01), issued by Test Authority"),
                "{}",
                outcome.message
            );
            assert_eq!(outcome.metrics[0].key, metric::DAYS_LEFT);
            assert!(outcome.metrics[0].value > 365.0);
            assert!(outcome.latency_ms.is_some());
            // The goodbye went through TLS: the server read it.
            assert_eq!(*heard.lock().unwrap(), said);
        }
    }

    #[tokio::test]
    async fn tls_from_the_start_comes_before_the_greeting() {
        let (authority, tls) = secured();
        let (port, heard) = server(vec![
            Step::Tls(tls),
            Say("* OK Gimap ready for requests\r\n"),
            Hear,
            Say("* BYE\r\n"),
        ])
        .await;
        let outcome = asked(
            MailProtocol::Imap,
            MailSecurity::Tls,
            true,
            port,
            &authority.trust(),
        )
        .await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert!(
            outcome
                .message
                .starts_with("IMAP ready (TLS): Gimap ready for requests. Certificate expires in "),
            "{}",
            outcome.message
        );
        assert_eq!(*heard.lock().unwrap(), ["a1 LOGOUT"]);

        // A server that isn't speaking TLS there.
        let (port, _) = server(vec![Say("220 mail ESMTP\r\n")]).await;
        let plain = asked(
            MailProtocol::Smtp,
            MailSecurity::Tls,
            true,
            port,
            &authority.trust(),
        )
        .await;
        assert_eq!(plain.status, Health::Down);
        assert!(
            plain
                .message
                .starts_with("The port is open, but SMTP didn't answer: TLS handshake"),
            "{}",
            plain.message
        );
    }

    #[tokio::test]
    async fn an_untrusted_certificate_is_down_unless_the_check_does_not_verify() {
        let (_, tls) = secured();
        let script = vec![
            Say("+OK POP3 ready\r\n"),
            Hear,
            Say("+OK Begin TLS\r\n"),
            Step::Tls(tls),
        ];
        let (port, _) = server(script).await;
        let public = public_trust();
        let refused = asked(
            MailProtocol::Pop3,
            MailSecurity::Starttls,
            true,
            port,
            &public,
        )
        .await;
        assert_eq!(refused.status, Health::Down);
        assert!(
            refused.message.starts_with(
                "POP3 answers, but its certificate isn't good: Certificate not trusted: its \
                 issuer isn't a public authority"
            ),
            "{}",
            refused.message
        );
        assert!(
            refused.metrics[0].value > 365.0,
            "the days are reported anyway"
        );

        let accepted = asked(
            MailProtocol::Pop3,
            MailSecurity::Starttls,
            false,
            port,
            &public,
        )
        .await;
        assert_eq!(accepted.status, Health::Up, "{}", accepted.message);

        // Expired: Down whether verified or not.
        let authority = Authority::new();
        let (certificate, key) = authority.issue(HOST, (2020, 1, 1));
        let (port, _) = server(vec![
            Step::Tls(acceptor(certificate, key)),
            Say("220 mail ESMTP\r\n"),
        ])
        .await;
        let expired = asked(
            MailProtocol::Smtp,
            MailSecurity::Tls,
            false,
            port,
            &authority.trust(),
        )
        .await;
        assert_eq!(expired.status, Health::Down);
        assert!(
            expired
                .message
                .starts_with("SMTP answers, but its certificate isn't good: Certificate expired "),
            "{}",
            expired.message
        );
    }

    #[tokio::test]
    async fn a_server_without_starttls_is_down() {
        let trust = public_trust();
        let cases = [
            (
                MailProtocol::Smtp,
                vec![
                    Say("220 mail ESMTP\r\n"),
                    Hear,
                    Say("250-mail\r\n250 8BITMIME\r\n"),
                ],
                "SMTP answers, but it doesn't offer STARTTLS",
            ),
            // An old server that knows no EHLO.
            (
                MailProtocol::Smtp,
                vec![
                    Say("220 mail SMTP\r\n"),
                    Hear,
                    Say("502 Command not implemented\r\n"),
                ],
                "SMTP answers, but it doesn't offer STARTTLS",
            ),
            // It offers, then refuses.
            (
                MailProtocol::Smtp,
                vec![
                    Say("220 mail ESMTP\r\n"),
                    Hear,
                    Say("250-mail\r\n250 STARTTLS\r\n"),
                    Hear,
                    Say("454 TLS not available due to temporary reason\r\n"),
                ],
                "SMTP answers, but it doesn't offer STARTTLS (454 TLS not available due to \
                 temporary reason)",
            ),
            (
                MailProtocol::Imap,
                vec![
                    Say("* OK ready\r\n"),
                    Hear,
                    Say("a1 BAD Unknown command\r\n"),
                ],
                "IMAP answers, but it doesn't offer STARTTLS (BAD Unknown command)",
            ),
            (
                MailProtocol::Pop3,
                vec![Say("+OK ready\r\n"), Hear, Say("-ERR Unknown command\r\n")],
                "POP3 answers, but it doesn't offer STARTTLS (Unknown command)",
            ),
            // It agrees, and something that isn't TLS follows.
            (
                MailProtocol::Pop3,
                vec![
                    Say("+OK ready\r\n"),
                    Hear,
                    Say("+OK Begin TLS\r\n"),
                    Step::Pause,
                    Say("this is not TLS, not at all, whatever a client may hope for\r\n"),
                ],
                "POP3 answers, but TLS didn't start after STARTTLS: TLS handshake",
            ),
            // More in the same breath as the agreement: an injected command.
            (
                MailProtocol::Smtp,
                vec![
                    Say("220 mail ESMTP\r\n"),
                    Hear,
                    Say("250-mail\r\n250 STARTTLS\r\n"),
                    Hear,
                    Say("220 Go ahead\r\n250 injected\r\n"),
                ],
                "SMTP answers, but it sent more before TLS began",
            ),
        ];
        for (protocol, script, message) in cases {
            let (port, _) = server(script).await;
            let outcome = asked(protocol, MailSecurity::Starttls, true, port, &trust).await;
            assert_eq!(outcome.status, Health::Down, "{}", outcome.message);
            assert!(
                outcome.message.starts_with(message),
                "{}: {message}",
                outcome.message
            );
        }
    }

    #[tokio::test]
    async fn a_line_without_end_is_not_read_for_ever() {
        static LONG: std::sync::LazyLock<String> =
            std::sync::LazyLock::new(|| format!("220 {}", "x".repeat(MAX_LINE_BYTES * 2)));
        let outcome = plain(MailProtocol::Smtp, vec![Say(LONG.as_str())]).await;
        assert_eq!(outcome.status, Health::Down);
        assert_eq!(
            outcome.message,
            "The port is open, but SMTP didn't answer: its answer isn't a mail server's"
        );
    }

    #[test]
    fn the_usual_port_follows_the_protocol_and_how_it_is_encrypted() {
        let port = |protocol: MailProtocol, security| protocol.default_port(security);
        assert_eq!(port(MailProtocol::Smtp, MailSecurity::Starttls), 25);
        assert_eq!(port(MailProtocol::Smtp, MailSecurity::Tls), 465);
        assert_eq!(port(MailProtocol::Imap, MailSecurity::None), 143);
        assert_eq!(port(MailProtocol::Imap, MailSecurity::Tls), 993);
        assert_eq!(port(MailProtocol::Pop3, MailSecurity::Starttls), 110);
        assert_eq!(port(MailProtocol::Pop3, MailSecurity::Tls), 995);
    }

    /// By hand (`--ignored --nocapture`): public mail servers, greeted.
    #[tokio::test]
    #[ignore = "talks to mail servers on the internet: run by hand"]
    async fn real_mail_servers() {
        let trust = public_trust();
        for (protocol, host, port, security) in [
            (
                MailProtocol::Smtp,
                "smtp.gmail.com",
                Some(587),
                MailSecurity::Starttls,
            ),
            (
                MailProtocol::Smtp,
                "smtp.gmail.com",
                None,
                MailSecurity::Tls,
            ),
            (
                MailProtocol::Imap,
                "imap.gmail.com",
                None,
                MailSecurity::Tls,
            ),
            (MailProtocol::Pop3, "pop.gmail.com", None, MailSecurity::Tls),
            (
                MailProtocol::Imap,
                "outlook.office365.com",
                None,
                MailSecurity::Tls,
            ),
            (
                MailProtocol::Smtp,
                "smtp.office365.com",
                Some(587),
                MailSecurity::Starttls,
            ),
            (
                MailProtocol::Imap,
                "imap.gmx.net",
                None,
                MailSecurity::Starttls,
            ),
            (
                MailProtocol::Pop3,
                "pop.gmx.net",
                None,
                MailSecurity::Starttls,
            ),
        ] {
            let outcome = check(protocol, host, port, security, true, &trust).await;
            println!(
                "{} {host} {:?}: {:?} in {:?} ms: {}",
                protocol.label(),
                security,
                outcome.status,
                outcome.latency_ms,
                outcome.message
            );
        }
    }
}
