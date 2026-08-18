//! Resolution of public send-expiration policies against fresh provider time.

use crate::SendExpiration;

/// Why an expiration policy cannot produce a valid wallet timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum SendExpirationError {
    /// The exact caller boundary has already passed according to the provider.
    #[error("transfer expiration timestamp is not after fresh provider time")]
    NotAfterProviderTime,
    /// Provider time and the configured default validity cannot fit in `u64`.
    #[error("transfer expiration timestamp overflow")]
    Overflow,
}

/// Resolves one expiration policy using the provider time fetched for this operation.
pub(super) fn resolve_send_expiration(
    expiration: &SendExpiration,
    provider_time: u64,
    default_validity_seconds: u64,
) -> Result<u64, SendExpirationError> {
    match expiration {
        SendExpiration::EngineDefault => provider_time
            .checked_add(default_validity_seconds)
            .ok_or(SendExpirationError::Overflow),
        SendExpiration::Exact { unix_timestamp } if *unix_timestamp > provider_time => {
            Ok(*unix_timestamp)
        }
        SendExpiration::Exact { .. } => Err(SendExpirationError::NotAfterProviderTime),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that default expiration starts at fresh provider time.
    #[test]
    fn engine_default_adds_configured_validity() {
        assert_eq!(
            resolve_send_expiration(&SendExpiration::EngineDefault, 100, 300),
            Ok(400)
        );
    }

    /// Verifies that exact expiration is preserved without applying defaults.
    #[test]
    fn exact_expiration_is_preserved() {
        assert_eq!(
            resolve_send_expiration(
                &SendExpiration::Exact {
                    unix_timestamp: 500,
                },
                100,
                300,
            ),
            Ok(500)
        );
    }

    /// Verifies that an exact boundary must remain in the provider's future.
    #[test]
    fn exact_expiration_rejects_current_or_past_time() {
        assert_eq!(
            resolve_send_expiration(
                &SendExpiration::Exact {
                    unix_timestamp: 100,
                },
                100,
                300,
            ),
            Err(SendExpirationError::NotAfterProviderTime)
        );
    }

    /// Verifies that default arithmetic cannot silently wrap.
    #[test]
    fn engine_default_rejects_overflow() {
        assert_eq!(
            resolve_send_expiration(&SendExpiration::EngineDefault, u64::MAX, 1),
            Err(SendExpirationError::Overflow)
        );
    }
}
