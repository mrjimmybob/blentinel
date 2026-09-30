//! Probe enrollment (proposal 0002 §4.2, D-47): the code an admin gets from
//! the hub's "Add probe", and the request the probe sends with it.
//!
//! A code carries everything a stock probe binary needs to trust its hub
//! from the first packet, without building anything per hub:
//!
//! - the hub URL;
//! - the hub's **X25519 public key** (reports are sealed for it; no
//!   trust-on-first-use handshake, D-02);
//! - the hub's **Ed25519 signing key**, which signs its answers to reports
//!   (`common::ack`, D-48);
//! - the **SHA-256 of the hub's TLS certificate**, if the hub serves HTTPS
//!   (the probe accepts only that certificate, D-01);
//! - a one-time token (ID + secret) that authorizes this enrollment.
//!
//! Wire form: `BLT1-` + Crockford base32 of
//! `[version][flags][url len u16][url][hub key 32][hub signing key 32][cert sha256 32?][token id 8][secret 16][crc32 4]`.
//! Pure code, no dependencies: shared by the hub (which issues codes) and the
//! probe (which redeems them).

use serde::{Deserialize, Serialize};

/// Text before the base32 payload.
pub const PREFIX: &str = "BLT1-";
const VERSION: u8 = 2;
const FLAG_TLS_PIN: u8 = 0b0000_0001;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The content of an enrollment code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentCode {
    /// Where the probe reaches the hub, e.g. `https://hub.example.com:3443`.
    pub hub_url: String,
    /// The hub's X25519 public key.
    pub hub_key: [u8; 32],
    /// The hub's Ed25519 public key, which signs its answers to reports.
    pub hub_signing_key: [u8; 32],
    /// SHA-256 of the hub's TLS certificate (DER); `None` if the hub has no
    /// TLS of its own (plain HTTP, or behind a proxy with a public certificate).
    pub tls_cert_sha256: Option<[u8; 32]>,
    pub token_id: [u8; 8],
    pub secret: [u8; 16],
}

/// Why a code can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeError {
    /// Doesn't start with `BLT1-`.
    NotACode,
    /// A character that isn't base32.
    BadCharacter(char),
    /// Mistyped or truncated (checksum mismatch or wrong length).
    Damaged,
    /// Made by a newer hub.
    UnsupportedVersion(u8),
}

impl std::fmt::Display for CodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodeError::NotACode => write!(
                f,
                "not an enrollment code: those start with {PREFIX} and come from the hub's \
                 Add probe (Companies > company > Add probe). The 8-character setup code \
                 is only for creating the hub's first Owner"
            ),
            CodeError::BadCharacter(c) => write!(f, "the code contains an invalid character {c:?}"),
            CodeError::Damaged => write!(f, "the code is damaged or incomplete; copy it again"),
            CodeError::UnsupportedVersion(v) => {
                write!(f, "the code is version {v}; this probe is too old for it")
            }
        }
    }
}

impl std::error::Error for CodeError {}

impl EnrollmentCode {
    pub fn encode(&self) -> String {
        let url = self.hub_url.as_bytes();
        let mut bytes = Vec::with_capacity(128 + url.len());
        bytes.push(VERSION);
        bytes.push(if self.tls_cert_sha256.is_some() {
            FLAG_TLS_PIN
        } else {
            0
        });
        bytes.extend_from_slice(&(url.len() as u16).to_be_bytes());
        bytes.extend_from_slice(url);
        bytes.extend_from_slice(&self.hub_key);
        bytes.extend_from_slice(&self.hub_signing_key);
        if let Some(pin) = &self.tls_cert_sha256 {
            bytes.extend_from_slice(pin);
        }
        bytes.extend_from_slice(&self.token_id);
        bytes.extend_from_slice(&self.secret);
        let crc = crc32(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        format!("{PREFIX}{}", base32_encode(&bytes))
    }

    /// Read a code. Case, surrounding spaces, dashes and line breaks are
    /// ignored; `O` reads as `0`, `I`/`L` as `1` (Crockford).
    pub fn decode(text: &str) -> Result<Self, CodeError> {
        let text = text.trim();
        let body = text
            .get(..PREFIX.len())
            .filter(|p| p.eq_ignore_ascii_case(PREFIX))
            .map(|_| &text[PREFIX.len()..])
            .ok_or(CodeError::NotACode)?;
        let bytes = base32_decode(body)?;
        let (payload, crc) = bytes
            .split_at_checked(bytes.len().saturating_sub(4))
            .ok_or(CodeError::Damaged)?;
        if crc.len() != 4 || crc32(payload).to_be_bytes() != crc {
            return Err(CodeError::Damaged);
        }

        let mut r = Reader(payload);
        let version = r.take(1)?[0];
        if version != VERSION {
            return Err(CodeError::UnsupportedVersion(version));
        }
        let flags = r.take(1)?[0];
        let url_len = u16::from_be_bytes(r.array()?) as usize;
        let hub_url =
            String::from_utf8(r.take(url_len)?.to_vec()).map_err(|_| CodeError::Damaged)?;
        let hub_key = r.array()?;
        let hub_signing_key = r.array()?;
        let tls_cert_sha256 = if flags & FLAG_TLS_PIN != 0 {
            Some(r.array()?)
        } else {
            None
        };
        let token_id = r.array()?;
        let secret = r.array()?;
        if !r.0.is_empty() {
            return Err(CodeError::Damaged);
        }
        Ok(Self {
            hub_url,
            hub_key,
            hub_signing_key,
            tls_cert_sha256,
            token_id,
            secret,
        })
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CodeError> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(CodeError::Damaged)?;
        self.0 = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], CodeError> {
        self.take(N)?.try_into().map_err(|_| CodeError::Damaged)
    }
}

fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 8 / 5 + 1);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn base32_decode(text: &str) -> Result<Vec<u8>, CodeError> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.chars() {
        if c == '-' || c.is_whitespace() {
            continue;
        }
        let value = match c.to_ascii_uppercase() {
            'O' => 0,
            'I' | 'L' => 1,
            upper => ALPHABET
                .iter()
                .position(|&a| a as char == upper)
                .ok_or(CodeError::BadCharacter(c))? as u32,
        };
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

/// CRC-32 (IEEE), to catch copy mistakes (not a security measure: the
/// secret and the hub's checks are).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// The enrollment request (probe → hub, sealed and signed like a report)
// ---------------------------------------------------------------------------

/// What a probe sends to `POST /api/enroll`, before signing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollRequest {
    /// Hex of the code's token ID.
    pub token_id: String,
    /// Hex of the code's secret.
    pub secret: String,
    /// The probe's ID: hex of its Ed25519 public key.
    pub probe_id: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub probe_version: String,
    /// RFC 3339; the hub refuses requests too far from its clock.
    pub timestamp: String,
}

/// The sealed payload: the request and the probe's signature over its JSON
/// (`serde_json::to_vec(&request)`), which proves the probe holds the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedEnrollRequest {
    pub request: EnrollRequest,
    /// Hex of the Ed25519 signature.
    pub signature: String,
}

/// The hub's answer: where the probe now belongs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollResponse {
    pub probe_id: String,
    pub name: String,
    pub company_id: String,
    pub company_name: String,
    pub site_id: String,
    pub site_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(pin: bool) -> EnrollmentCode {
        EnrollmentCode {
            hub_url: "https://hub.example.com:3443".into(),
            hub_key: [7; 32],
            hub_signing_key: [5; 32],
            tls_cert_sha256: pin.then_some([9; 32]),
            token_id: [1, 2, 3, 4, 5, 6, 7, 8],
            secret: [0xAB; 16],
        }
    }

    #[test]
    fn codes_round_trip_with_and_without_a_tls_pin() {
        for pin in [true, false] {
            let text = code(pin).encode();
            assert!(text.starts_with("BLT1-"));
            assert_eq!(EnrollmentCode::decode(&text).unwrap(), code(pin));
        }
    }

    #[test]
    fn decoding_forgives_case_spaces_dashes_and_lookalikes() {
        let text = code(true).encode();
        let messy = format!("  {}\n", text.to_lowercase());
        assert_eq!(EnrollmentCode::decode(&messy).unwrap(), code(true));
        let body = &text[PREFIX.len()..];
        let dashed: String = body
            .chars()
            .collect::<Vec<_>>()
            .chunks(5)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("-");
        assert_eq!(
            EnrollmentCode::decode(&format!("{PREFIX}{dashed}")).unwrap(),
            code(true)
        );
        let lookalike = format!(
            "{PREFIX}{}",
            body.replacen('0', "O", 1).replacen('1', "l", 1)
        );
        assert_eq!(EnrollmentCode::decode(&lookalike).unwrap(), code(true));
    }

    #[test]
    fn a_changed_or_missing_character_is_caught() {
        let text = code(true).encode();
        let last = text.len() - 3;
        let replacement = if &text[last..=last] == "A" { "B" } else { "A" };
        let typo = format!("{}{}{}", &text[..last], replacement, &text[last + 1..]);
        assert_eq!(EnrollmentCode::decode(&typo), Err(CodeError::Damaged));
        assert_eq!(
            EnrollmentCode::decode(&text[..text.len() - 1]),
            Err(CodeError::Damaged)
        );
        assert_eq!(
            EnrollmentCode::decode("BLT1-!!"),
            Err(CodeError::BadCharacter('!'))
        );
        assert_eq!(EnrollmentCode::decode("hello"), Err(CodeError::NotACode));
    }

    #[test]
    fn a_newer_version_is_refused_by_name() {
        let mut bytes = vec![VERSION + 1, 0, 0, 0];
        let crc = crc32(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        let text = format!("{PREFIX}{}", base32_encode(&bytes));
        assert_eq!(
            EnrollmentCode::decode(&text),
            Err(CodeError::UnsupportedVersion(VERSION + 1))
        );
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
