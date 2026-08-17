//! Runtime-neutral HTTP bridge endpoint and SSE framing primitives.

use std::{fmt, mem, num::NonZeroU32, num::NonZeroUsize};

use thiserror::Error;
use url::Url;

use crate::{BridgeMessage, ClientId, TraceId};

/// Validated base URL of a TON Connect HTTP bridge.
///
/// Both HTTPS and HTTP are accepted because local bridge deployments commonly
/// use HTTP. Production hosts remain responsible for requiring HTTPS.
#[derive(Clone, Eq, PartialEq)]
pub struct HttpBridgeUrl(Url);

impl HttpBridgeUrl {
    /// Builds the bridge SSE endpoint without discarding a base-path segment.
    #[must_use]
    pub fn events_endpoint(
        &self,
        client_id: ClientId,
        last_event_id: Option<&str>,
        trace_id: Option<&TraceId>,
    ) -> Url {
        let mut endpoint = self.endpoint("events");
        let client_id = client_id.to_string();
        {
            let mut query = endpoint.query_pairs_mut();
            let _ = query.append_pair("client_id", &client_id);
            if let Some(last_event_id) = last_event_id {
                let _ = query.append_pair("last_event_id", last_event_id);
            }
            if let Some(trace_id) = trace_id {
                let _ = query.append_pair("trace_id", trace_id.as_str());
            }
            let _ = query.append_pair("heartbeat", "message");
        }
        endpoint
    }

    /// Builds the bridge send endpoint for one encrypted message.
    #[must_use]
    pub fn message_endpoint(
        &self,
        sender: ClientId,
        recipient: ClientId,
        ttl: NonZeroU32,
        topic: Option<&str>,
        trace_id: Option<&TraceId>,
    ) -> Url {
        let mut endpoint = self.endpoint("message");
        let sender = sender.to_string();
        let recipient = recipient.to_string();
        let ttl = ttl.to_string();
        {
            let mut query = endpoint.query_pairs_mut();
            let _ = query
                .append_pair("client_id", &sender)
                .append_pair("to", &recipient)
                .append_pair("ttl", &ttl);
            if let Some(topic) = topic {
                let _ = query.append_pair("topic", topic);
            }
            if let Some(trace_id) = trace_id {
                let _ = query.append_pair("trace_id", trace_id.as_str());
            }
        }
        endpoint
    }

    fn endpoint(&self, suffix: &str) -> Url {
        let mut endpoint = self.0.clone();
        let mut path = endpoint.path().trim_end_matches('/').to_owned();
        path.push('/');
        path.push_str(suffix);
        endpoint.set_path(&path);
        endpoint
    }
}

impl fmt::Debug for HttpBridgeUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("HttpBridgeUrl")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<&str> for HttpBridgeUrl {
    type Error = HttpBridgeError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let parsed = Url::parse(value).map_err(|_| HttpBridgeError::InvalidUrl)?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.has_host()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(HttpBridgeError::InvalidUrl);
        }
        Ok(Self(parsed))
    }
}

/// One decoded bridge message together with its SSE resume cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeSseMessage {
    event_id: Option<String>,
    message: BridgeMessage,
}

impl BridgeSseMessage {
    /// Returns the cursor to pass as `last_event_id` after reconnecting.
    #[must_use]
    pub fn event_id(&self) -> Option<&str> {
        self.event_id.as_deref()
    }

    /// Returns the validated encrypted bridge envelope.
    #[must_use]
    pub const fn message(&self) -> &BridgeMessage {
        &self.message
    }

    /// Consumes the SSE wrapper and returns the bridge envelope.
    #[must_use]
    pub fn into_message(self) -> BridgeMessage {
        self.message
    }
}

/// Incremental decoder for the `text/event-stream` returned by `/events`.
///
/// The decoder accepts arbitrarily split network chunks, tolerates comments,
/// legacy heartbeat events, and the `heartbeat=message` payload. A strict size
/// bound prevents an untrusted bridge from growing the pending event forever.
#[derive(Debug)]
pub struct BridgeSseDecoder {
    pending: Vec<u8>,
    event_type: Option<String>,
    data_lines: Vec<String>,
    last_event_id: Option<String>,
    current_event_bytes: usize,
    max_event_bytes: NonZeroUsize,
}

impl BridgeSseDecoder {
    /// Creates a decoder with an explicit maximum size for one SSE event.
    #[must_use]
    pub const fn new(max_event_bytes: NonZeroUsize) -> Self {
        Self {
            pending: Vec::new(),
            event_type: None,
            data_lines: Vec::new(),
            last_event_id: None,
            current_event_bytes: 0,
            max_event_bytes,
        }
    }

    /// Appends one transport chunk and returns every complete bridge message.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<BridgeSseMessage>, HttpBridgeError> {
        self.pending.extend_from_slice(chunk);
        let mut messages = Vec::new();

        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=newline).collect::<Vec<_>>();
            if line.last() == Some(&b'\n') {
                let _ = line.pop();
            }
            if line.last() == Some(&b'\r') {
                let _ = line.pop();
            }
            self.current_event_bytes = self
                .current_event_bytes
                .saturating_add(line.len())
                .saturating_add(1);
            // Bytes after this newline can belong to later events already
            // buffered in the same network chunk, so only charge this line to
            // the current event. The remaining partial line is checked below.
            self.ensure_size_bound(0)?;

            let line = String::from_utf8(line).map_err(|_| HttpBridgeError::InvalidSseUtf8)?;
            if line.is_empty() {
                if let Some(message) = self.dispatch()? {
                    messages.push(message);
                }
                self.current_event_bytes = 0;
            } else {
                self.process_line(&line);
            }
        }

        self.ensure_size_bound(self.pending.len())?;
        Ok(messages)
    }

    fn ensure_size_bound(&self, pending_bytes: usize) -> Result<(), HttpBridgeError> {
        if self.current_event_bytes.saturating_add(pending_bytes) > self.max_event_bytes.get() {
            Err(HttpBridgeError::EventTooLarge)
        } else {
            Ok(())
        }
    }

    fn process_line(&mut self, line: &str) {
        if line.starts_with(':') {
            return;
        }
        let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| {
            (field, value.strip_prefix(' ').unwrap_or(value))
        });
        match field {
            "event" => self.event_type = Some(value.to_owned()),
            "data" => self.data_lines.push(value.to_owned()),
            "id" if !value.contains('\0') => self.last_event_id = Some(value.to_owned()),
            _ => {}
        }
    }

    fn dispatch(&mut self) -> Result<Option<BridgeSseMessage>, HttpBridgeError> {
        let event_type = self
            .event_type
            .take()
            .unwrap_or_else(|| "message".to_owned());
        let data = mem::take(&mut self.data_lines).join("\n");
        if event_type == "heartbeat" || data.is_empty() || data == "heartbeat" {
            return Ok(None);
        }
        if event_type != "message" {
            return Ok(None);
        }
        let message = serde_json::from_str(&data).map_err(HttpBridgeError::InvalidMessage)?;
        Ok(Some(BridgeSseMessage {
            event_id: self.last_event_id.clone(),
            message,
        }))
    }
}

/// Invalid bridge configuration or untrusted SSE input.
#[derive(Debug, Error)]
pub enum HttpBridgeError {
    /// The bridge base is not an absolute HTTP(S) URL without query or fragment.
    #[error("bridge URL must be an absolute HTTP(S) base without query or fragment")]
    InvalidUrl,
    /// A pending SSE event exceeded the configured memory bound.
    #[error("bridge SSE event exceeded the configured size limit")]
    EventTooLarge,
    /// An SSE field was not valid UTF-8.
    #[error("bridge SSE stream contains invalid UTF-8")]
    InvalidSseUtf8,
    /// An SSE message event did not contain a valid bridge envelope.
    #[error("invalid bridge message in SSE event: {0}")]
    InvalidMessage(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::{error::Error, num::NonZeroUsize};

    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::*;
    use crate::{AppRequest, SessionCrypto};

    const CLIENT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn endpoints_preserve_the_published_base_path() -> Result<(), Box<dyn Error>> {
        let bridge = HttpBridgeUrl::try_from("https://bridge.example/ton/bridge/")?;
        let client = CLIENT.parse()?;
        let event = bridge.events_endpoint(client, Some("41"), None);
        assert_eq!(event.path(), "/ton/bridge/events");
        assert_eq!(
            event.query(),
            Some(concat!(
                "client_id=0123456789abcdef0123456789abcdef",
                "0123456789abcdef0123456789abcdef&last_event_id=41&heartbeat=message"
            ))
        );

        let ttl = NonZeroU32::new(300).ok_or("TTL must be non-zero")?;
        let message = bridge.message_endpoint(client, client, ttl, Some("disconnect"), None);
        assert_eq!(message.path(), "/ton/bridge/message");
        assert!(message.as_str().contains("topic=disconnect"));
        Ok(())
    }

    #[test]
    fn decoder_handles_byte_splits_crlf_and_both_heartbeats() -> Result<(), Box<dyn Error>> {
        let input = concat!(
            "event: heartbeat\r\n\r\n",
            "event: message\n",
            "data: heartbeat\n\n",
            "id: 42\n",
            "event: message\n",
            "data: {\"from\":\"0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef\",\"message\":\"AA==\"}\n\n"
        );
        let limit = NonZeroUsize::new(4096).ok_or("limit must be non-zero")?;
        let mut decoder = BridgeSseDecoder::new(limit);
        let mut messages = Vec::new();
        for chunk in input.as_bytes().chunks(1) {
            messages.extend(decoder.push(chunk)?);
        }

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages.first().and_then(BridgeSseMessage::event_id),
            Some("42")
        );
        assert_eq!(
            messages
                .first()
                .map(BridgeSseMessage::message)
                .map(BridgeMessage::from),
            Some(CLIENT.parse()?)
        );
        Ok(())
    }

    #[test]
    fn decoder_rejects_an_unbounded_event_before_dispatch() -> Result<(), Box<dyn Error>> {
        let limit = NonZeroUsize::new(16).ok_or("limit must be non-zero")?;
        let mut decoder = BridgeSseDecoder::new(limit);
        assert!(matches!(
            decoder.push(b"data: 01234567890123456789"),
            Err(HttpBridgeError::EventTooLarge)
        ));
        Ok(())
    }

    #[test]
    fn decoder_applies_the_limit_per_event_not_per_network_chunk() -> Result<(), Box<dyn Error>> {
        let limit = NonZeroUsize::new(32).ok_or("limit must be non-zero")?;
        let mut decoder = BridgeSseDecoder::new(limit);
        let heartbeats = "event: heartbeat\n\n".repeat(100);
        assert!(decoder.push(heartbeats.as_bytes()).is_ok());
        Ok(())
    }

    #[test]
    fn encrypted_request_round_trips_through_the_sse_envelope() -> Result<(), Box<dyn Error>> {
        let dapp = SessionCrypto::generate()?;
        let wallet = SessionCrypto::generate()?;
        let request = AppRequest {
            method: "disconnect".to_owned(),
            params: Vec::new(),
            id: "7".to_owned(),
        };
        let plaintext = serde_json::to_vec(&request)?;
        let encrypted = dapp.encrypt(wallet.client_id(), &plaintext)?;
        let envelope = serde_json::json!({
            "from": dapp.client_id().to_string(),
            "message": STANDARD.encode(encrypted)
        });
        let stream = format!("id: 9\ndata: {envelope}\n\n");
        let limit = NonZeroUsize::new(4096).ok_or("limit must be non-zero")?;
        let mut decoder = BridgeSseDecoder::new(limit);
        let messages = decoder.push(stream.as_bytes())?;
        let message = messages.first().ok_or("message must be decoded")?;
        let ciphertext = message.message().message().decode()?;
        let decrypted = wallet.decrypt(message.message().from(), &ciphertext)?;

        assert_eq!(serde_json::from_slice::<AppRequest>(&decrypted)?, request);
        assert_eq!(message.event_id(), Some("9"));
        Ok(())
    }
}
