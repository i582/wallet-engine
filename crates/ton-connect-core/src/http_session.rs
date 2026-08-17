//! Durable HTTP bridge session identity and replay state.

use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;

use crate::{
    ClientId, HttpBridgeUrl, PersistedSessionKeyPair, RawAccountAddress, SessionCrypto,
    SessionKeyPairError, WalletSessionPhase, WalletSessionState,
};

/// Confidential state required to resume one wallet-side HTTP bridge session.
///
/// The serialized value contains the session secret key and must receive the
/// same storage protection as other authentication credentials.
#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedHttpSession {
    key_pair: PersistedSessionKeyPair,
    peer_client_id: ClientId,
    bridge_url: HttpBridgeUrl,
    reducer: WalletSessionState,
    connected_address: Option<RawAccountAddress>,
    last_bridge_event_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedHttpSessionWire {
    key_pair: PersistedSessionKeyPair,
    peer_client_id: ClientId,
    bridge_url: HttpBridgeUrl,
    reducer: WalletSessionState,
    connected_address: Option<RawAccountAddress>,
    last_bridge_event_id: Option<String>,
}

impl<'de> Deserialize<'de> for PersistedHttpSession {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PersistedHttpSessionWire::deserialize(deserializer)?;
        validate_event_id(wire.last_bridge_event_id.as_deref()).map_err(de::Error::custom)?;
        validate_account_binding(wire.reducer.phase(), wire.connected_address)
            .map_err(de::Error::custom)?;
        let _ = SessionCrypto::from_persisted(&wire.key_pair).map_err(de::Error::custom)?;
        Ok(Self {
            key_pair: wire.key_pair,
            peer_client_id: wire.peer_client_id,
            bridge_url: wire.bridge_url,
            reducer: wire.reducer,
            connected_address: wire.connected_address,
            last_bridge_event_id: wire.last_bridge_event_id,
        })
    }
}

impl PersistedHttpSession {
    /// Captures a complete resumable session snapshot.
    pub fn new(
        crypto: &SessionCrypto,
        peer_client_id: ClientId,
        bridge_url: HttpBridgeUrl,
        reducer: WalletSessionState,
        connected_address: Option<RawAccountAddress>,
        last_bridge_event_id: Option<String>,
    ) -> Result<Self, HttpSessionError> {
        validate_event_id(last_bridge_event_id.as_deref())?;
        validate_account_binding(reducer.phase(), connected_address)?;
        Ok(Self {
            key_pair: crypto.persisted_keypair(),
            peer_client_id,
            bridge_url,
            reducer,
            connected_address,
            last_bridge_event_id,
        })
    }

    /// Restores and verifies the local session crypto identity.
    pub fn restore_crypto(&self) -> Result<SessionCrypto, HttpSessionError> {
        SessionCrypto::from_persisted(&self.key_pair).map_err(Into::into)
    }

    /// Returns the dApp bridge client identifier fixed for this session.
    #[must_use]
    pub const fn peer_client_id(&self) -> ClientId {
        self.peer_client_id
    }

    /// Returns the bridge base used by this session.
    #[must_use]
    pub const fn bridge_url(&self) -> &HttpBridgeUrl {
        &self.bridge_url
    }

    /// Returns the durable request/event reducer state.
    #[must_use]
    pub const fn reducer(&self) -> &WalletSessionState {
        &self.reducer
    }

    /// Returns the account fixed for the connected session lifetime.
    #[must_use]
    pub const fn connected_address(&self) -> Option<RawAccountAddress> {
        self.connected_address
    }

    /// Returns the SSE cursor to use on the next subscription.
    #[must_use]
    pub fn last_bridge_event_id(&self) -> Option<&str> {
        self.last_bridge_event_id.as_deref()
    }

    /// Updates the SSE cursor after the surrounding state is durably committed.
    pub fn set_last_bridge_event_id(
        &mut self,
        value: Option<String>,
    ) -> Result<(), HttpSessionError> {
        validate_event_id(value.as_deref())?;
        self.last_bridge_event_id = value;
        Ok(())
    }

    /// Replaces the reducer after the prepared transition is durably committed.
    pub fn set_reducer(
        &mut self,
        reducer: WalletSessionState,
        connected_address: Option<RawAccountAddress>,
    ) -> Result<(), HttpSessionError> {
        validate_account_binding(reducer.phase(), connected_address)?;
        self.reducer = reducer;
        self.connected_address = connected_address;
        Ok(())
    }
}

/// Persisted HTTP bridge session is malformed or cryptographically inconsistent.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum HttpSessionError {
    /// Persisted public and secret session keys do not match.
    #[error(transparent)]
    InvalidKeyPair(#[from] SessionKeyPairError),
    /// SSE cursor contains characters forbidden by the `EventSource` model.
    #[error("bridge event id must not contain null, CR, or LF characters")]
    InvalidEventId,
    /// Connected reducer state has no immutable account identity, or pending
    /// state exposes one before connect succeeds.
    #[error("persisted session account does not match its lifecycle phase")]
    InvalidAccountBinding,
}

const fn validate_account_binding(
    phase: WalletSessionPhase,
    address: Option<RawAccountAddress>,
) -> Result<(), HttpSessionError> {
    match (phase, address) {
        (WalletSessionPhase::PendingConnect, None)
        | (WalletSessionPhase::Connected, Some(_))
        | (WalletSessionPhase::Disconnected, None | Some(_)) => Ok(()),
        (WalletSessionPhase::PendingConnect, Some(_)) | (WalletSessionPhase::Connected, None) => {
            Err(HttpSessionError::InvalidAccountBinding)
        }
    }
}

fn validate_event_id(value: Option<&str>) -> Result<(), HttpSessionError> {
    if value.is_some_and(|value| value.contains(['\0', '\r', '\n'])) {
        Err(HttpSessionError::InvalidEventId)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WalletEventKind, WalletSessionState};

    #[test]
    fn serialized_session_restores_crypto_reducer_and_cursor()
    -> Result<(), Box<dyn std::error::Error>> {
        let local = SessionCrypto::generate()?;
        let peer = SessionCrypto::generate()?;
        let reducer = WalletSessionState::pending_connect()
            .prepare_event(WalletEventKind::Connect)?
            .into_state();
        let persisted = PersistedHttpSession::new(
            &local,
            peer.client_id(),
            HttpBridgeUrl::try_from("https://bridge.example/bridge")?,
            reducer.clone(),
            Some(RawAccountAddress::new(0, [9_u8; 32])),
            Some("41".to_owned()),
        )?;
        let encoded = serde_json::to_vec(&persisted)?;
        let decoded = serde_json::from_slice::<PersistedHttpSession>(&encoded)?;
        let restored = decoded.restore_crypto()?;

        assert_eq!(restored.client_id(), local.client_id());
        assert_eq!(decoded.peer_client_id(), peer.client_id());
        assert_eq!(decoded.reducer(), &reducer);
        assert_eq!(
            decoded.connected_address(),
            Some(RawAccountAddress::new(0, [9_u8; 32]))
        );
        assert_eq!(decoded.last_bridge_event_id(), Some("41"));
        assert_eq!(
            decoded.bridge_url().as_str(),
            "https://bridge.example/bridge"
        );
        Ok(())
    }

    #[test]
    fn event_cursor_rejects_eventsource_control_characters()
    -> Result<(), Box<dyn std::error::Error>> {
        let local = SessionCrypto::generate()?;
        let peer = SessionCrypto::generate()?;
        let bridge = HttpBridgeUrl::try_from("https://bridge.example/bridge")?;
        for invalid in ["a\0b", "a\rb", "a\nb"] {
            assert!(matches!(
                PersistedHttpSession::new(
                    &local,
                    peer.client_id(),
                    bridge.clone(),
                    WalletSessionState::pending_connect(),
                    None,
                    Some(invalid.to_owned()),
                ),
                Err(HttpSessionError::InvalidEventId)
            ));
        }
        Ok(())
    }

    #[test]
    fn connected_phase_requires_a_fixed_account() -> Result<(), Box<dyn std::error::Error>> {
        let local = SessionCrypto::generate()?;
        let peer = SessionCrypto::generate()?;
        let connected = WalletSessionState::pending_connect()
            .prepare_event(WalletEventKind::Connect)?
            .into_state();
        let bridge = HttpBridgeUrl::try_from("https://bridge.example/bridge")?;

        assert!(matches!(
            PersistedHttpSession::new(
                &local,
                peer.client_id(),
                bridge.clone(),
                connected,
                None,
                None
            ),
            Err(HttpSessionError::InvalidAccountBinding)
        ));
        assert!(matches!(
            PersistedHttpSession::new(
                &local,
                peer.client_id(),
                bridge,
                WalletSessionState::pending_connect(),
                Some(RawAccountAddress::new(0, [1_u8; 32])),
                None,
            ),
            Err(HttpSessionError::InvalidAccountBinding)
        ));
        Ok(())
    }

    #[test]
    fn failed_session_updates_do_not_corrupt_the_persisted_snapshot()
    -> Result<(), Box<dyn std::error::Error>> {
        let local = SessionCrypto::generate()?;
        let peer = SessionCrypto::generate()?;
        let bridge = HttpBridgeUrl::try_from("https://bridge.example/bridge")?;
        let mut persisted = PersistedHttpSession::new(
            &local,
            peer.client_id(),
            bridge,
            WalletSessionState::pending_connect(),
            None,
            Some("7".to_owned()),
        )?;

        assert_eq!(
            persisted.set_last_bridge_event_id(Some("bad\nvalue".to_owned())),
            Err(HttpSessionError::InvalidEventId)
        );
        assert_eq!(persisted.last_bridge_event_id(), Some("7"));
        persisted.set_last_bridge_event_id(Some("8".to_owned()))?;
        assert_eq!(persisted.last_bridge_event_id(), Some("8"));

        let connected = WalletSessionState::pending_connect()
            .prepare_event(WalletEventKind::Connect)?
            .into_state();
        assert_eq!(
            persisted.set_reducer(connected.clone(), None),
            Err(HttpSessionError::InvalidAccountBinding)
        );
        assert_eq!(
            persisted.reducer().phase(),
            WalletSessionPhase::PendingConnect
        );
        assert_eq!(persisted.connected_address(), None);

        let address = RawAccountAddress::new(0, [3_u8; 32]);
        persisted.set_reducer(connected, Some(address))?;
        assert_eq!(persisted.reducer().phase(), WalletSessionPhase::Connected);
        assert_eq!(persisted.connected_address(), Some(address));
        Ok(())
    }
}
