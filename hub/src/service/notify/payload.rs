//! What each kind of channel sends for a [`Notification`] (D-66), as data
//! ([`Outgoing`]) that a transport then delivers: tests read it without a
//! network.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;

use super::message::Notification;
use crate::model::{AlertKind, ChannelKind, MailSecurity, addresses};

/// The hub's mail server, its password opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailServer {
    pub server: String,
    pub port: u16,
    pub security: MailSecurity,
    pub username: String,
    pub password: String,
    pub from: String,
}

/// A channel, its secret opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenChannel {
    pub id: i64,
    pub name: String,
    pub kind: ChannelKind,
    pub target: String,
    pub secret: Option<String>,
}

/// One delivery, ready to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    Email {
        server: MailServer,
        to: Vec<String>,
        subject: String,
        body: String,
    },
    /// A JSON POST; `body` is sent byte for byte (a webhook's signature covers it).
    Http {
        url: String,
        headers: Vec<(String, String)>,
        body: String,
    },
}

/// The header with a webhook's signature, and the one with its timestamp.
pub const SIGNATURE_HEADER: &str = "X-Blentinel-Signature";
pub const TIMESTAMP_HEADER: &str = "X-Blentinel-Timestamp";

/// `sha256=<hex>` of HMAC-SHA256 over `<timestamp>.<body>` with `key`: what a
/// webhook receiver recomputes (and it refuses old timestamps, so a captured
/// request can't be replayed).
pub fn signature(key: &str, timestamp: i64, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC takes any key");
    mac.update(format!("{timestamp}.{body}").as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn secret(channel: &OpenChannel) -> anyhow::Result<&str> {
    channel
        .secret
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the channel has no {}",
                channel.kind.secret_label().unwrap_or("secret")
            )
        })
}

fn http(url: &str, body: serde_json::Value) -> Outgoing {
    Outgoing::Http {
        url: url.to_string(),
        headers: Vec::new(),
        body: body.to_string(),
    }
}

/// Discord's embed colour for an alert.
fn colour(n: &Notification) -> u32 {
    match (n.test, n.kind) {
        (true, _) => 0x3b82f6,
        (_, AlertKind::Down) => 0xef4444,
        (_, AlertKind::Warning) => 0xf59e0b,
        (_, AlertKind::Recovered) => 0x22c55e,
        (_, AlertKind::ProbeExpired) => 0x94a3b8,
    }
}

/// Slack treats `&`, `<` and `>` as markup.
fn slack_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A webhook's JSON: the notification's fields, for any receiver to read.
fn webhook_body(n: &Notification, link: Option<&str>) -> serde_json::Value {
    json!({
        "event": if n.test { "test" } else { match n.kind {
            AlertKind::Down => "down",
            AlertKind::Warning => "warning",
            AlertKind::Recovered => "recovered",
            AlertKind::ProbeExpired => "probe_expired",
        }},
        "subject": n.subject(),
        "at": n.at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "company": { "id": n.company_id, "name": n.company_name },
        "site": { "id": n.site_id, "name": n.site_name },
        "probe": { "id": n.probe_id, "name": n.probe_name, "hostname": n.hostname },
        "check": n.check.as_ref().map(|c| json!({
            "id": c.id, "name": c.name, "kind": c.kind, "target": c.target,
        })),
        "detail": n.detail,
        "incident": n.incident.map(|i| json!({
            "id": i.id,
            "opened_at": i.opened_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        })),
        "link": link,
    })
}

/// What `channel` sends for `n` (`mail`: the hub's mail server, for e-mail;
/// `now`: a webhook's signature time).
pub fn outgoing(
    channel: &OpenChannel,
    n: &Notification,
    link: Option<&str>,
    mail: Option<&MailServer>,
    now: DateTime<Utc>,
) -> anyhow::Result<Outgoing> {
    let title = format!("{} {}", n.about(), n.word());
    let mut summary = n.place();
    if let Some(lasted) = n.lasted() {
        summary.push_str(&format!("\nLasted {lasted}"));
    }
    if !n.detail.is_empty() {
        summary.push_str(&format!("\n{}", n.detail));
    }
    Ok(match channel.kind {
        ChannelKind::Email => {
            let server = mail
                .filter(|m| !m.server.is_empty())
                .ok_or_else(|| anyhow::anyhow!("the hub's mail server isn't set (alerts page)"))?;
            Outgoing::Email {
                server: server.clone(),
                to: addresses(&channel.target),
                subject: n.subject(),
                body: n.text(link),
            }
        }
        ChannelKind::Webhook => {
            let body = webhook_body(n, link).to_string();
            let timestamp = now.timestamp();
            Outgoing::Http {
                url: channel.target.trim().to_string(),
                headers: vec![
                    (TIMESTAMP_HEADER.into(), timestamp.to_string()),
                    (
                        SIGNATURE_HEADER.into(),
                        signature(secret(channel)?, timestamp, &body),
                    ),
                ],
                body,
            }
        }
        ChannelKind::Slack => {
            let mut text = format!("*{}*\n{}", slack_escape(&title), slack_escape(&summary));
            if let Some(link) = link {
                text.push_str(&format!("\n<{link}|Open in Blentinel>"));
            }
            http(secret(channel)?, json!({ "text": text }))
        }
        ChannelKind::Discord => {
            let mut embed = json!({
                "title": title.chars().take(256).collect::<String>(),
                "description": summary.chars().take(4000).collect::<String>(),
                "color": colour(n),
                "timestamp": n.at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            });
            if let Some(link) = link {
                embed["url"] = json!(link);
            }
            http(secret(channel)?, json!({ "embeds": [embed] }))
        }
        ChannelKind::Teams => {
            let mut card = json!({
                "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                "type": "AdaptiveCard",
                "version": "1.4",
                "body": [
                    { "type": "TextBlock", "text": title, "weight": "Bolder", "size": "Medium", "wrap": true },
                    { "type": "TextBlock", "text": summary, "wrap": true },
                ],
            });
            if let Some(link) = link {
                card["actions"] = json!([{ "type": "Action.OpenUrl", "title": "Open in Blentinel", "url": link }]);
            }
            http(
                secret(channel)?,
                json!({
                    "type": "message",
                    "attachments": [{
                        "contentType": "application/vnd.microsoft.card.adaptive",
                        "content": card,
                    }],
                }),
            )
        }
        ChannelKind::Telegram => http(
            &format!(
                "https://api.telegram.org/bot{}/sendMessage",
                secret(channel)?
            ),
            json!({
                "chat_id": channel.target.trim(),
                "text": n.text(link),
                "disable_web_page_preview": true,
            }),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_791_000_000, 0).unwrap()
    }

    fn alert() -> Notification {
        Notification {
            test: false,
            probe_id: "p1".into(),
            ..Notification::sample(at())
        }
    }

    fn channel(kind: ChannelKind, target: &str, secret: Option<&str>) -> OpenChannel {
        OpenChannel {
            id: 1,
            name: "ops".into(),
            kind,
            target: target.into(),
            secret: secret.map(str::to_string),
        }
    }

    fn json_of(out: &Outgoing) -> (String, serde_json::Value) {
        match out {
            Outgoing::Http { url, body, .. } => (url.clone(), serde_json::from_str(body).unwrap()),
            other => panic!("not HTTP: {other:?}"),
        }
    }

    const LINK: Option<&str> = Some("https://hub.example.com/probes/p1");

    #[test]
    fn a_webhook_is_signed_over_its_timestamp_and_exact_body() {
        let out = outgoing(
            &channel(
                ChannelKind::Webhook,
                "https://example.com/hook",
                Some("k3y"),
            ),
            &alert(),
            LINK,
            None,
            at(),
        )
        .unwrap();
        let Outgoing::Http { url, headers, body } = &out else {
            panic!()
        };
        assert_eq!(url, "https://example.com/hook");
        let header = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(header(TIMESTAMP_HEADER), "1791000000");
        assert_eq!(
            header(SIGNATURE_HEADER),
            signature("k3y", 1_791_000_000, body)
        );
        assert_ne!(
            header(SIGNATURE_HEADER),
            signature("other", 1_791_000_000, body)
        );
        let (_, json) = json_of(&out);
        assert_eq!(json["event"], "down");
        assert_eq!(json["check"]["name"], "Test check");
        assert_eq!(json["company"]["name"], "Example Ltd");
        assert_eq!(json["link"], "https://hub.example.com/probes/p1");
        assert_eq!(json["incident"], serde_json::Value::Null, "none given");
    }

    #[test]
    fn the_signature_is_standard_hmac_sha256() {
        // The primitive, on RFC 4231's test case 2...
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac.finalize().into_bytes()),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // ...and the signature is that, over "<timestamp>.<body>".
        let mut mac = Hmac::<Sha256>::new_from_slice(b"k").unwrap();
        mac.update(b"1.{}");
        assert_eq!(
            signature("k", 1, "{}"),
            format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
        );
    }

    #[test]
    fn chat_channels_get_their_own_shape() {
        let slack = outgoing(
            &channel(ChannelKind::Slack, "", Some("https://hooks.slack.com/x")),
            &alert(),
            LINK,
            None,
            at(),
        )
        .unwrap();
        let (url, json) = json_of(&slack);
        assert_eq!(url, "https://hooks.slack.com/x");
        let text = json["text"].as_str().unwrap();
        assert!(text.starts_with("*Test check DOWN*\n"), "{text}");
        assert!(text.contains("Example Ltd › Head office"));
        assert!(text.ends_with("<https://hub.example.com/probes/p1|Open in Blentinel>"));

        let (_, discord) = json_of(
            &outgoing(
                &channel(
                    ChannelKind::Discord,
                    "",
                    Some("https://discord.com/api/webhooks/1/x"),
                ),
                &alert(),
                LINK,
                None,
                at(),
            )
            .unwrap(),
        );
        assert_eq!(discord["embeds"][0]["title"], "Test check DOWN");
        assert_eq!(discord["embeds"][0]["color"], 0xef4444);
        assert_eq!(
            discord["embeds"][0]["url"],
            "https://hub.example.com/probes/p1"
        );

        let (_, teams) = json_of(
            &outgoing(
                &channel(ChannelKind::Teams, "", Some("https://prod.workflows/x")),
                &alert(),
                None,
                None,
                at(),
            )
            .unwrap(),
        );
        assert_eq!(
            teams["attachments"][0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(
            teams["attachments"][0]["content"]["body"][0]["text"],
            "Test check DOWN"
        );
        assert!(
            teams["attachments"][0]["content"].get("actions").is_none(),
            "no link: no button"
        );

        let (url, telegram) = json_of(
            &outgoing(
                &channel(ChannelKind::Telegram, " -100123 ", Some("123:ABC")),
                &alert(),
                LINK,
                None,
                at(),
            )
            .unwrap(),
        );
        assert_eq!(url, "https://api.telegram.org/bot123:ABC/sendMessage");
        assert_eq!(telegram["chat_id"], "-100123");
        assert!(
            telegram["text"]
                .as_str()
                .unwrap()
                .starts_with("Test check DOWN")
        );
    }

    #[test]
    fn slack_markup_in_names_is_escaped() {
        let mut odd = alert();
        odd.company_name = "A&B <Ltd>".into();
        let (_, json) = json_of(
            &outgoing(
                &channel(ChannelKind::Slack, "", Some("https://hooks.slack.com/x")),
                &odd,
                None,
                None,
                at(),
            )
            .unwrap(),
        );
        assert!(
            json["text"]
                .as_str()
                .unwrap()
                .contains("A&amp;B &lt;Ltd&gt;")
        );
    }

    #[test]
    fn e_mail_needs_the_hubs_mail_server() {
        let email = channel(ChannelKind::Email, "a@example.com, b@example.com", None);
        assert!(
            outgoing(&email, &alert(), None, None, at())
                .unwrap_err()
                .to_string()
                .contains("mail server")
        );
        let server = MailServer {
            server: "smtp.example.com".into(),
            port: 587,
            security: MailSecurity::Starttls,
            username: String::new(),
            password: String::new(),
            from: "hub@example.com".into(),
        };
        let Outgoing::Email {
            to, subject, body, ..
        } = outgoing(&email, &alert(), LINK, Some(&server), at()).unwrap()
        else {
            panic!()
        };
        assert_eq!(to, ["a@example.com", "b@example.com"]);
        assert_eq!(subject, "[BLENTINEL][example][hq] Test check DOWN");
        assert!(body.ends_with("https://hub.example.com/probes/p1"));
    }

    #[test]
    fn a_channel_without_its_secret_says_so() {
        let error = outgoing(
            &channel(ChannelKind::Telegram, "1", None),
            &alert(),
            None,
            None,
            at(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Bot token"));
    }
}
