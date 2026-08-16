use std::{cmp::Ordering, fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

use crate::AppRequest;

/// A dApp RPC request ID ordered as an arbitrary-precision unsigned integer.
///
/// The protocol transports this counter as a string and does not specify a
/// machine-width limit. Canonicalization removes leading zeroes so `"01"` and
/// `"1"` cannot bypass replay protection while the original request envelope
/// remains available for an exact response-ID echo.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct RpcRequestId(String);

impl RpcRequestId {
    /// Returns the canonical decimal representation used for ordering.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RpcRequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for RpcRequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RpcRequestId {
    type Err = SessionStateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || !value.as_bytes().iter().all(u8::is_ascii_digit) {
            return Err(SessionStateError::InvalidRequestId);
        }
        let without_zeroes = value.trim_start_matches('0');
        let canonical = if without_zeroes.is_empty() {
            "0"
        } else {
            without_zeroes
        };
        Ok(Self(canonical.to_owned()))
    }
}

impl Ord for RpcRequestId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.cmp(&other.0))
    }
}

impl PartialOrd for RpcRequestId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Serialize for RpcRequestId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RpcRequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

/// Durable lifecycle phase of one wallet-side TON Connect session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletSessionPhase {
    /// A deep-link request exists but no connect event has completed it.
    PendingConnect,
    /// The connect event succeeded and dApp RPC requests are accepted.
    Connected,
    /// A connect error or either side's disconnect permanently ended the session.
    Disconnected,
}

/// Wallet event transition whose identifier is allocated by the session reducer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WalletEventKind {
    /// Successful initial connection.
    Connect,
    /// Failed initial connection.
    ConnectError,
    /// Wallet-initiated termination of an established session.
    Disconnect,
}

/// Persistable state required for monotonic IDs, replay rejection, and disconnect.
///
/// Transition methods are deliberately non-mutating. Atomically persist the
/// returned next state together with the accepted request or outgoing event
/// before running the wallet action or publishing. After a crash, restore that
/// durable work item and retry it directly; a replay from the bridge is then
/// rejected instead of signing the same request twice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WalletSessionState {
    phase: WalletSessionPhase,
    last_request_id: Option<RpcRequestId>,
    last_event_id: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWalletSessionState {
    phase: WalletSessionPhase,
    last_request_id: Option<RpcRequestId>,
    last_event_id: Option<u64>,
}

impl<'de> Deserialize<'de> for WalletSessionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawWalletSessionState::deserialize(deserializer)?;
        let valid = match raw.phase {
            WalletSessionPhase::PendingConnect => {
                raw.last_request_id.is_none() && raw.last_event_id.is_none()
            }
            WalletSessionPhase::Connected | WalletSessionPhase::Disconnected => {
                raw.last_event_id.is_some()
            }
        };
        if !valid {
            return Err(de::Error::custom(
                "persisted TON Connect session state violates lifecycle invariants",
            ));
        }
        Ok(Self {
            phase: raw.phase,
            last_request_id: raw.last_request_id,
            last_event_id: raw.last_event_id,
        })
    }
}

impl WalletSessionState {
    /// Creates state for a newly received connect request.
    #[must_use]
    pub const fn pending_connect() -> Self {
        Self {
            phase: WalletSessionPhase::PendingConnect,
            last_request_id: None,
            last_event_id: None,
        }
    }

    /// Returns the durable lifecycle phase.
    #[must_use]
    pub const fn phase(&self) -> WalletSessionPhase {
        self.phase
    }

    /// Returns the highest accepted dApp request ID, if any.
    #[must_use]
    pub fn last_request_id(&self) -> Option<&RpcRequestId> {
        self.last_request_id.as_ref()
    }

    /// Returns the last allocated wallet event ID, if any.
    #[must_use]
    pub const fn last_event_id(&self) -> Option<u64> {
        self.last_event_id
    }

    /// Prepares a wallet event and its durable next state.
    ///
    /// The first event ID is zero, matching the reference TypeScript SDK. A
    /// connect result may only finish a pending session, and a disconnect event
    /// may only finish a connected session.
    pub fn prepare_event(
        &self,
        kind: WalletEventKind,
    ) -> Result<PreparedWalletEvent, SessionStateError> {
        let next_phase = match (self.phase, kind) {
            (WalletSessionPhase::PendingConnect, WalletEventKind::Connect) => {
                WalletSessionPhase::Connected
            }
            (WalletSessionPhase::PendingConnect, WalletEventKind::ConnectError)
            | (WalletSessionPhase::Connected, WalletEventKind::Disconnect) => {
                WalletSessionPhase::Disconnected
            }
            (
                WalletSessionPhase::PendingConnect
                | WalletSessionPhase::Connected
                | WalletSessionPhase::Disconnected,
                WalletEventKind::Connect
                | WalletEventKind::ConnectError
                | WalletEventKind::Disconnect,
            ) => return Err(SessionStateError::InvalidEventTransition),
        };

        let id = match self.last_event_id {
            Some(previous) => previous
                .checked_add(1)
                .ok_or(SessionStateError::EventIdExhausted)?,
            None => 0,
        };
        let mut next_state = self.clone();
        next_state.phase = next_phase;
        next_state.last_event_id = Some(id);
        Ok(PreparedWalletEvent { id, next_state })
    }

    /// Accepts a strictly newer dApp request and prepares its durable next state.
    ///
    /// The raw envelope is used intentionally: unsupported or malformed methods
    /// still consume a fresh ID once processed, so replaying the same packet
    /// cannot repeatedly exercise parsing and approval code. A syntactically
    /// exact `disconnect` request closes the session in the same atomic state
    /// transition that consumes its ID.
    pub fn prepare_request(
        &self,
        request: &AppRequest,
    ) -> Result<PreparedAppRequest, SessionStateError> {
        if self.phase != WalletSessionPhase::Connected {
            return Err(SessionStateError::SessionNotConnected);
        }

        let request_id = RpcRequestId::from_str(&request.id)?;
        if self
            .last_request_id
            .as_ref()
            .is_some_and(|last| request_id <= *last)
        {
            return Err(SessionStateError::RequestIdNotIncreasing);
        }

        let closes_session = request.method == "disconnect" && request.params.is_empty();
        let mut next_state = self.clone();
        next_state.last_request_id = Some(request_id.clone());
        if closes_session {
            next_state.phase = WalletSessionPhase::Disconnected;
        }
        Ok(PreparedAppRequest {
            request_id,
            closes_session,
            next_state,
        })
    }
}

impl Default for WalletSessionState {
    fn default() -> Self {
        Self::pending_connect()
    }
}

/// Prepared event ID plus the state that must be persisted before publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedWalletEvent {
    id: u64,
    next_state: WalletSessionState,
}

impl PreparedWalletEvent {
    /// Returns the event ID to place in the wallet event envelope.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Borrows the state that must be persisted before event publication.
    #[must_use]
    pub const fn next_state(&self) -> &WalletSessionState {
        &self.next_state
    }

    /// Consumes the transition and returns its durable state.
    #[must_use]
    pub fn into_state(self) -> WalletSessionState {
        self.next_state
    }
}

/// Accepted request plus the state that must be persisted before processing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedAppRequest {
    request_id: RpcRequestId,
    closes_session: bool,
    next_state: WalletSessionState,
}

impl PreparedAppRequest {
    /// Returns the canonical numeric request ID used by replay protection.
    #[must_use]
    pub const fn request_id(&self) -> &RpcRequestId {
        &self.request_id
    }

    /// Reports whether this exact request atomically ends the session.
    #[must_use]
    pub const fn closes_session(&self) -> bool {
        self.closes_session
    }

    /// Borrows the state that must be persisted before request processing.
    #[must_use]
    pub const fn next_state(&self) -> &WalletSessionState {
        &self.next_state
    }

    /// Consumes the transition and returns its durable state.
    #[must_use]
    pub fn into_state(self) -> WalletSessionState {
        self.next_state
    }
}

/// A rejected session-state transition.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SessionStateError {
    /// Request ID is not a non-empty unsigned decimal string.
    #[error("TON Connect request id must be a non-empty unsigned decimal string")]
    InvalidRequestId,
    /// Request ID is equal to or below the durable session baseline.
    #[error("TON Connect request id is not strictly greater than the last processed id")]
    RequestIdNotIncreasing,
    /// RPC requests are only valid after a successful connect and before disconnect.
    #[error("TON Connect session is not connected")]
    SessionNotConnected,
    /// The requested event does not follow the session lifecycle.
    #[error("wallet event is invalid for the current TON Connect session phase")]
    InvalidEventTransition,
    /// No larger `u64` wallet event ID can be allocated.
    #[error("TON Connect wallet event id is exhausted")]
    EventIdExhausted,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn connected() -> Result<WalletSessionState, SessionStateError> {
        WalletSessionState::pending_connect()
            .prepare_event(WalletEventKind::Connect)
            .map(PreparedWalletEvent::into_state)
    }

    fn request(id: &str, method: &str, params: Vec<String>) -> AppRequest {
        AppRequest {
            method: method.to_owned(),
            params,
            id: id.to_owned(),
        }
    }

    #[test]
    fn numeric_order_is_not_lexicographic_or_machine_width_limited() {
        let nine = RpcRequestId::from_str("9");
        let ten = RpcRequestId::from_str("10");
        let huge = RpcRequestId::from_str("18446744073709551616000000000000000000");
        assert!(
            matches!((nine, ten, huge), (Ok(nine), Ok(ten), Ok(huge)) if nine < ten && ten < huge)
        );
    }

    #[test]
    fn leading_zeroes_cannot_bypass_replay_rejection() -> Result<(), SessionStateError> {
        let state = connected()?;
        let persisted = state
            .prepare_request(&request("001", "unknown", Vec::new()))?
            .into_state();
        assert_eq!(
            persisted.prepare_request(&request("1", "unknown", Vec::new())),
            Err(SessionStateError::RequestIdNotIncreasing)
        );
        Ok(())
    }

    #[test]
    fn restored_state_rejects_replay_before_wallet_action_runs_again()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = connected()?;
        let accepted =
            original.prepare_request(&request("7", "signData", vec!["{}".to_owned()]))?;
        assert_eq!(original.last_request_id(), None);

        let stored = serde_json::to_string(accepted.next_state())?;
        let restored = serde_json::from_str::<WalletSessionState>(&stored)?;
        assert_eq!(
            restored.prepare_request(&request("7", "signData", vec!["{}".to_owned()])),
            Err(SessionStateError::RequestIdNotIncreasing)
        );
        Ok(())
    }

    #[test]
    fn valid_disconnect_consumes_id_and_closes_in_one_transition() -> Result<(), SessionStateError>
    {
        let accepted = connected()?.prepare_request(&request("3", "disconnect", Vec::new()))?;
        assert!(accepted.closes_session());
        let closed = accepted.into_state();
        assert_eq!(closed.phase(), WalletSessionPhase::Disconnected);
        assert_eq!(
            closed.prepare_request(&request("4", "sendTransaction", vec!["{}".to_owned()])),
            Err(SessionStateError::SessionNotConnected)
        );
        Ok(())
    }

    #[test]
    fn malformed_disconnect_consumes_id_without_closing() -> Result<(), SessionStateError> {
        let accepted = connected()?.prepare_request(&request(
            "3",
            "disconnect",
            vec!["unexpected".to_owned()],
        ))?;
        assert!(!accepted.closes_session());
        assert_eq!(accepted.next_state().phase(), WalletSessionPhase::Connected);
        Ok(())
    }

    #[test]
    fn event_lifecycle_is_monotonic_and_terminal() -> Result<(), SessionStateError> {
        let pending = WalletSessionState::pending_connect();
        let connect = pending.prepare_event(WalletEventKind::Connect)?;
        assert_eq!(connect.id(), 0);
        let disconnect = connect
            .into_state()
            .prepare_event(WalletEventKind::Disconnect)?;
        assert_eq!(disconnect.id(), 1);
        assert_eq!(
            disconnect.next_state().phase(),
            WalletSessionPhase::Disconnected
        );
        assert_eq!(
            disconnect
                .next_state()
                .prepare_event(WalletEventKind::Disconnect),
            Err(SessionStateError::InvalidEventTransition)
        );
        Ok(())
    }

    #[test]
    fn persisted_state_rejects_impossible_lifecycle_combinations() {
        let pending_with_request =
            r#"{"phase":"pending_connect","last_request_id":"1","last_event_id":null}"#;
        let connected_without_event =
            r#"{"phase":"connected","last_request_id":null,"last_event_id":null}"#;
        assert!(serde_json::from_str::<WalletSessionState>(pending_with_request).is_err());
        assert!(serde_json::from_str::<WalletSessionState>(connected_without_event).is_err());
    }

    proptest! {
        #[test]
        fn every_strictly_increasing_u128_sequence_is_accepted(
            mut values in proptest::collection::vec(any::<u128>(), 1..64)
        ) {
            values.sort_unstable();
            values.dedup();
            let mut state = connected()
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            for value in values {
                let transition = state.prepare_request(&request(
                    &value.to_string(),
                    "unknown",
                    Vec::new(),
                ));
                prop_assert!(transition.is_ok());
                if let Ok(accepted) = transition {
                    state = accepted.into_state();
                }
            }
        }
    }
}
