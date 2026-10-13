//! A connection spoken line by line: what the mail protocols (D-95) and a
//! UPS server (D-97) have in common.

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use super::database::broken;

/// The longest line the probe reads (an IMAP greeting lists what the server
/// can do). A longer one isn't a line of these protocols.
pub(super) const MAX_LINE_BYTES: usize = 8 * 1024;

/// A connection read and written line by line.
pub(super) struct Wire<S> {
    stream: BufReader<S>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Wire<S> {
    pub(super) fn new(stream: S) -> Self {
        Self {
            stream: BufReader::new(stream),
        }
    }

    /// The next line, without its end. `Err`: why there is none, in words.
    pub(super) async fn line(&mut self) -> Result<String, String> {
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
            Some(_) => Err("it sent a line without an end".to_string()),
        }
    }

    /// Send `command` as a line.
    pub(super) async fn send(&mut self, command: &str) -> Result<(), String> {
        self.stream
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .map_err(broken)?;
        self.stream.flush().await.map_err(broken)
    }

    /// The connection itself, to go on with in another way (TLS). `None`:
    /// the server has sent more than the lines read, which would be lost.
    pub(super) fn into_stream(self) -> Option<S> {
        self.stream
            .buffer()
            .is_empty()
            .then(|| self.stream.into_inner())
    }
}
