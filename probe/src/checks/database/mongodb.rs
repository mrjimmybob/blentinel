//! MongoDB's greeting: the `hello` command, which every driver sends first
//! and which needs no login. The answer says what the server is: its
//! version's wire protocol, and its place in a replica set.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::{Answer, Greeted, broken, read_bytes, said};

/// The message kind every command since MongoDB 3.6 travels in.
const OP_MSG: u32 = 2013;

/// A value of a BSON document, as far as a greeting needs them.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f64),
    Text(String),
    Bool(bool),
    Other,
}

/// `{ hello: 1, $db: "admin" }` as BSON.
fn hello_document() -> Vec<u8> {
    let mut fields = Vec::new();
    // An int32 named "hello".
    fields.push(0x10);
    fields.extend(b"hello\0");
    fields.extend(1i32.to_le_bytes());
    // A string named "$db" (its length counts its zero).
    fields.push(0x02);
    fields.extend(b"$db\0");
    fields.extend(6i32.to_le_bytes());
    fields.extend(b"admin\0");
    fields.push(0);
    let mut document = ((fields.len() + 4) as i32).to_le_bytes().to_vec();
    document.extend(fields);
    document
}

/// The `hello` command as a message: a header (length, request ID, what it
/// answers, kind), flags, and one section holding the document.
fn hello() -> Vec<u8> {
    let document = hello_document();
    let len = 16 + 4 + 1 + document.len();
    let mut message = (len as i32).to_le_bytes().to_vec();
    message.extend(1i32.to_le_bytes());
    message.extend(0i32.to_le_bytes());
    message.extend(OP_MSG.to_le_bytes());
    message.extend(0u32.to_le_bytes());
    message.push(0);
    message.extend(document);
    message
}

fn int32(bytes: &[u8]) -> Option<i32> {
    Some(i32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

/// The top-level fields of a BSON document; reading stops at what it
/// doesn't know or can't be right.
fn fields(document: &[u8]) -> Vec<(String, Value)> {
    let mut found = Vec::new();
    let mut rest = document.get(4..).unwrap_or_default();
    while let Some((&kind, after)) = rest.split_first() {
        if kind == 0 {
            break;
        }
        let Some(end) = after.iter().position(|b| *b == 0) else {
            break;
        };
        let name = String::from_utf8_lossy(&after[..end]).into_owned();
        let value = &after[end + 1..];
        // How long the value is, and what it is worth.
        let read = match kind {
            0x01 => value
                .get(..8)
                .and_then(|b| b.try_into().ok())
                .map(|b| (8, Value::Number(f64::from_le_bytes(b)))),
            0x02 => int32(value).and_then(|len| {
                let len = usize::try_from(len).ok()?;
                let text = value.get(4..4 + len.checked_sub(1)?)?;
                Some((
                    4 + len,
                    Value::Text(String::from_utf8_lossy(text).into_owned()),
                ))
            }),
            // A document, an array: their length counts itself.
            0x03 | 0x04 => int32(value).map(|len| (len.max(5) as usize, Value::Other)),
            0x05 => int32(value).map(|len| (len.max(0) as usize + 5, Value::Other)),
            0x07 => Some((12, Value::Other)),
            0x08 => value.first().map(|b| (1, Value::Bool(*b != 0))),
            0x09 | 0x11 => Some((8, Value::Other)),
            0x0A => Some((0, Value::Other)),
            0x10 => int32(value).map(|n| (4, Value::Number(f64::from(n)))),
            0x12 => value
                .get(..8)
                .and_then(|b| b.try_into().ok())
                .map(|b| (8, Value::Number(i64::from_le_bytes(b) as f64))),
            0x13 => Some((16, Value::Other)),
            _ => None,
        };
        let Some((len, value)) = read else {
            break;
        };
        let Some(next) = after.get(end + 1 + len..) else {
            break;
        };
        found.push((name, value));
        rest = next;
    }
    found
}

/// The release that speaks a wire version, where it is a known one.
fn release(wire: i64) -> Option<&'static str> {
    Some(match wire {
        6 => "3.6",
        7 => "4.0",
        8 => "4.2",
        9 => "4.4",
        13 => "5.0",
        17 => "6.0",
        21 => "7.0",
        25 => "8.0",
        _ => return None,
    })
}

/// What a `hello` answer says about the server.
fn judge(document: &[u8]) -> Greeted {
    let fields = fields(document);
    let get = |name: &str| fields.iter().find(|(n, _)| n == name).map(|(_, v)| v);
    let text = |name: &str| match get(name) {
        Some(Value::Text(text)) => Some(text.as_str()),
        _ => None,
    };
    let yes = |name: &str| get(name) == Some(&Value::Bool(true));
    match get("ok") {
        Some(Value::Number(ok)) if *ok == 1.0 => {}
        Some(_) => {
            return Ok(Answer::Refusing(said(
                text("errmsg").unwrap_or("it gave no reason"),
            )));
        }
        None => return Err("its answer isn't MongoDB's".to_string()),
    }
    let version = match get("maxWireVersion") {
        Some(Value::Number(wire)) => match release(*wire as i64) {
            Some(release) => format!("MongoDB {release}"),
            None => format!("MongoDB (wire version {wire})"),
        },
        _ => "MongoDB".to_string(),
    };
    // Its place: a router, a member of a replica set, or on its own.
    let place = if text("msg") == Some("isdbgrid") {
        " (a mongos router)".to_string()
    } else if let Some(set) = text("setName") {
        let role = if yes("isWritablePrimary") || yes("ismaster") {
            "primary"
        } else if yes("secondary") {
            "secondary"
        } else if yes("arbiterOnly") {
            "arbiter"
        } else {
            "member"
        };
        format!(" ({role} of {})", said(set))
    } else {
        String::new()
    };
    Ok(Answer::Ready(format!(
        "{version} accepts connections{place}"
    )))
}

pub(super) async fn greet(mut stream: TcpStream) -> Greeted {
    stream.write_all(&hello()).await.map_err(broken)?;
    let mut header = [0u8; 16];
    stream.read_exact(&mut header).await.map_err(broken)?;
    let len = int32(&header).and_then(|len| usize::try_from(len).ok());
    let kind = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    let Some(body_len) = len.and_then(|len| len.checked_sub(16)) else {
        return Err("its answer isn't MongoDB's".to_string());
    };
    if kind != OP_MSG || body_len < 5 {
        return Err("its answer isn't MongoDB's".to_string());
    }
    let body = read_bytes(&mut stream, body_len).await?;
    // Flags, then a section of kind 0: the document.
    if body[4] != 0 {
        return Err("its answer isn't MongoDB's".to_string());
    }
    judge(&body[5..])
}

#[cfg(test)]
mod tests {
    use super::super::tests::{greeting, server};
    use super::*;
    use common::checks::{DbEngine, Health};

    enum Field<'a> {
        Double(f64),
        Int(i32),
        Text(&'a str),
        Bool(bool),
        /// An embedded document, as given.
        Document(Vec<u8>),
    }

    fn document(fields: &[(&str, Field<'_>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, field) in fields {
            let (kind, value) = match field {
                Field::Double(n) => (0x01, n.to_le_bytes().to_vec()),
                Field::Int(n) => (0x10, n.to_le_bytes().to_vec()),
                Field::Text(text) => {
                    let mut value = ((text.len() + 1) as i32).to_le_bytes().to_vec();
                    value.extend(text.as_bytes());
                    value.push(0);
                    (0x02, value)
                }
                Field::Bool(b) => (0x08, vec![u8::from(*b)]),
                Field::Document(bytes) => (0x03, bytes.clone()),
            };
            body.push(kind);
            body.extend(name.as_bytes());
            body.push(0);
            body.extend(value);
        }
        body.push(0);
        let mut document = ((body.len() + 4) as i32).to_le_bytes().to_vec();
        document.extend(body);
        document
    }

    /// A server's answer holding `document`.
    fn reply(document: &[u8]) -> Vec<u8> {
        let len = 16 + 4 + 1 + document.len();
        let mut message = (len as i32).to_le_bytes().to_vec();
        message.extend(7i32.to_le_bytes());
        message.extend(1i32.to_le_bytes());
        message.extend(OP_MSG.to_le_bytes());
        message.extend(0u32.to_le_bytes());
        message.push(0);
        message.extend(document);
        message
    }

    #[tokio::test]
    async fn a_server_answers_hello_with_what_it_is() {
        let alone = document(&[
            ("isWritablePrimary", Field::Bool(true)),
            // What a greeting doesn't need is stepped over.
            (
                "topologyVersion",
                Field::Document(document(&[("counter", Field::Int(6))])),
            ),
            ("maxWireVersion", Field::Int(21)),
            ("ok", Field::Double(1.0)),
        ]);
        let outcome = greeting(DbEngine::Mongodb, server(true, reply(&alone)).await).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(outcome.message, "MongoDB 7.0 accepts connections");

        let secondary = document(&[
            ("setName", Field::Text("rs0")),
            ("isWritablePrimary", Field::Bool(false)),
            ("secondary", Field::Bool(true)),
            ("maxWireVersion", Field::Int(99)),
            ("ok", Field::Double(1.0)),
        ]);
        assert_eq!(
            judge(&secondary),
            Ok(Answer::Ready(
                "MongoDB (wire version 99) accepts connections (secondary of rs0)".into()
            ))
        );
        let primary = document(&[
            ("setName", Field::Text("rs0")),
            ("isWritablePrimary", Field::Bool(true)),
            ("ok", Field::Int(1)),
        ]);
        assert_eq!(
            judge(&primary),
            Ok(Answer::Ready(
                "MongoDB accepts connections (primary of rs0)".into()
            ))
        );
        let router = document(&[("msg", Field::Text("isdbgrid")), ("ok", Field::Double(1.0))]);
        assert_eq!(
            judge(&router),
            Ok(Answer::Ready(
                "MongoDB accepts connections (a mongos router)".into()
            ))
        );
    }

    #[tokio::test]
    async fn a_server_that_says_no_is_down_with_its_reason() {
        let refused = document(&[
            ("ok", Field::Double(0.0)),
            ("errmsg", Field::Text("node is shutting down")),
        ]);
        let outcome = greeting(DbEngine::Mongodb, server(true, reply(&refused)).await).await;
        assert_eq!(outcome.status, Health::Down);
        assert_eq!(
            outcome.message,
            "MongoDB answers, but refuses connections: node is shutting down"
        );
    }

    #[test]
    fn hello_is_a_message_of_its_own_length_and_broken_documents_end_quietly() {
        let message = hello();
        assert_eq!(int32(&message), Some(message.len() as i32));
        assert_eq!(&message[12..16], &OP_MSG.to_le_bytes());
        let document = &message[21..];
        assert_eq!(int32(document), Some(document.len() as i32));
        assert_eq!(
            fields(document),
            [
                ("hello".to_string(), Value::Number(1.0)),
                ("$db".to_string(), Value::Text("admin".into())),
            ]
        );
        // Cut anywhere, a document gives what it had and no panic.
        for len in 0..document.len() {
            assert!(fields(&document[..len]).len() <= 2);
        }
        assert_eq!(judge(&[]), Err("its answer isn't MongoDB's".to_string()));
        // A string whose length lies.
        assert!(
            fields(&[
                20, 0, 0, 0, 0x02, b'a', 0, 0xff, 0xff, 0xff, 0x7f, b'x', 0, 0
            ])
            .is_empty()
        );
    }
}
