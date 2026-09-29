//! Toncenter v3 key-change history of this wallet.
//!
//! Every Wallet rev00 rotation publishes the replaced signing key encrypted with
//! the new one. Toncenter classifies these rotations as `change_wallet_key`
//! actions and reports the new public key and the encrypted old key. Decryption
//! of comments sent to a replaced key reads this history; the records hold no
//! secret, so the client keeps the last copy in memory.

use serde::Deserialize;
use serde_json::Value;

use crate::transport::build_toncenter_v3_request;
use crate::wallet::key_history::KeyChange;
use crate::{DomainError, HttpRequest, HttpRequestId, TonAddressString, WalletClientConfig};

use super::provider::invalid_response;

/// Rotations requested per page. One page covers every realistic wallet.
pub(super) const KEY_CHANGE_PAGE_SIZE: usize = 100;

/// Upper bound on pages read for one history, so a misbehaving provider cannot
/// keep the decryption slot forever.
pub(super) const MAX_KEY_CHANGE_PAGES: usize = 10;

const CHANGE_WALLET_KEY_ACTION: &str = "change_wallet_key";

#[derive(Debug, Deserialize)]
struct ActionsResponse {
    actions: Vec<RawAction>,
}

#[derive(Debug, Deserialize)]
struct RawAction {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    details: Value,
}

#[derive(Debug, Deserialize)]
struct ChangeWalletKeyDetails {
    #[serde(default)]
    destination: Option<String>,
    #[serde(default)]
    new_public_key: Option<String>,
    #[serde(default)]
    encrypted_old_private_key: Option<String>,
}

/// One provider page of this wallet's rotations.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct KeyChangePage {
    /// Successful rotations of this wallet, in provider order.
    pub(super) changes: Vec<KeyChange>,
    /// The number of actions the provider returned, including skipped ones.
    pub(super) raw_count: usize,
}

impl KeyChangePage {
    /// Reports whether the provider can have another page after this one.
    pub(super) const fn has_more(&self) -> bool {
        self.raw_count >= KEY_CHANGE_PAGE_SIZE
    }
}

/// Reports whether a history contains the rotation that installed `public_key`.
pub(super) fn history_reaches(changes: &[KeyChange], public_key: &[u8; 32]) -> bool {
    changes
        .iter()
        .any(|change| change.new_public_key == *public_key)
}

/// Builds one page request for this wallet's `change_wallet_key` actions, newest first.
pub(super) fn build_key_change_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    offset: usize,
) -> Result<HttpRequest, crate::WalletClientError> {
    let limit = KEY_CHANGE_PAGE_SIZE.to_string();
    let offset = offset.to_string();
    build_toncenter_v3_request(
        config,
        id,
        "actions",
        &[
            ("account", config.address.as_str()),
            ("action_type", CHANGE_WALLET_KEY_ACTION),
            ("limit", &limit),
            ("offset", &offset),
            ("sort", "desc"),
        ],
    )
}

/// Parses one `/api/v3/actions` page and keeps the successful rotations of `wallet`.
///
/// Failed rotations, other action types, and rotations of other wallets are
/// skipped: an account filter also returns actions this wallet only relayed.
/// Toncenter indexer v1.3 reports a rotation only together with its key-changed
/// log, so a rotation of this wallet without both keys is a provider protocol
/// error.
pub(super) fn parse_key_change_page(
    body: &[u8],
    wallet: &TonAddressString,
) -> Result<KeyChangePage, DomainError> {
    let response: ActionsResponse = serde_json::from_slice(body)
        .map_err(|error| invalid_response(format!("invalid key-change actions: {error}")))?;
    let raw_count = response.actions.len();
    let mut changes = Vec::new();
    for action in response.actions {
        if action.kind != CHANGE_WALLET_KEY_ACTION || action.success != Some(true) {
            continue;
        }
        let details: ChangeWalletKeyDetails =
            serde_json::from_value(action.details).map_err(|error| {
                invalid_response(format!("invalid change_wallet_key details: {error}"))
            })?;
        let Some(destination) = details.destination else {
            continue;
        };
        let destination = TonAddressString::try_from(destination).map_err(|error| {
            invalid_response(format!("invalid change_wallet_key destination: {error}"))
        })?;
        if destination.as_address() != wallet.as_address() {
            continue;
        }
        changes.push(KeyChange {
            new_public_key: parse_key_hex(details.new_public_key, "new_public_key")?,
            encrypted_old_private_key: parse_key_hex(
                details.encrypted_old_private_key,
                "encrypted_old_private_key",
            )?,
        });
    }
    Ok(KeyChangePage { changes, raw_count })
}

fn parse_key_hex(value: Option<String>, field: &str) -> Result<[u8; 32], DomainError> {
    let malformed = || invalid_response(format!("change_wallet_key {field} is not 32 hex bytes"));
    let value = value.ok_or_else(malformed)?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(malformed());
    }
    let mut bytes = [0_u8; 32];
    for (byte, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| malformed())?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| malformed())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Network, NonEmptyString, ProviderConfig};

    const WALLET: &str = "0:1350B4C79830A1FC14F4DE2537A27911DC0FF37D9FDDC05AC3008F218D75F4DB";
    const RELAYER: &str = "0:E15488B65F7E904F1D14C84AC6F603CCC4A404B03BEFB568D5F5CC8ED6866887";
    const NEW_KEY: &str = "38fc3898a2ce60ef78dd116059e81a5a80a57b74350aac9e8aa24c97bb644d33";
    const ENCRYPTED: &str = "4c5cf96e0d7591f1797e579be823c7f96a05ba8b1907fed898e5d2b76708d026";

    fn wallet() -> TonAddressString {
        TonAddressString::try_from(WALLET).expect("wallet address")
    }

    fn bytes(hex: &str) -> [u8; 32] {
        parse_key_hex(Some(hex.to_owned()), "test").expect("valid hex")
    }

    fn action(kind: &str, success: Value, details: Value) -> Value {
        json!({
            "trace_id": "ybABMr+c9dJPt7wFMfUQyjd9HmweAIBz77Ym9Bh3M7Y=",
            "action_id": "NJ+4G1wFzvsHnES0cXli0phw24NlMjD7wQuiFtPIv8Q=",
            "start_lt": "99612240000002",
            "end_lt": "99612240000003",
            "transactions": ["ybABMr+c9dJPt7wFMfUQyjd9HmweAIBz77Ym9Bh3M7Y="],
            "success": success,
            "type": kind,
            "details": details,
            "finality": "finalized"
        })
    }

    fn rotation(destination: &str, new_public_key: Value, encrypted: Value) -> Value {
        action(
            "change_wallet_key",
            json!(true),
            json!({
                "source": RELAYER,
                "destination": destination,
                "value": "50000000",
                "new_public_key": new_public_key,
                "rotation_signature": "289e76cf42c5339c872c903e0a8ca0768178d39cbcfa8fe634a493323a0abe581dc130c87ff0dcd29280bc8841464e915d02b0b2294d0a0ad17fdd58713c2c00",
                "encrypted_old_private_key": encrypted
            }),
        )
    }

    fn page(actions: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "actions": actions,
            "address_book": {},
            "metadata": {}
        }))
        .expect("page JSON")
    }

    #[test]
    fn parses_toncenter_change_wallet_key_actions() {
        let body = page(vec![
            rotation(WALLET, json!(NEW_KEY), json!(ENCRYPTED)),
            rotation(
                "0:1350b4c79830a1fc14f4de2537a27911dc0ff37d9fddc05ac3008f218d75f4db",
                json!(NEW_KEY.to_ascii_uppercase()),
                json!(ENCRYPTED.to_ascii_uppercase()),
            ),
        ]);

        let page = parse_key_change_page(&body, &wallet()).expect("page parses");

        let expected = KeyChange {
            new_public_key: bytes(NEW_KEY),
            encrypted_old_private_key: bytes(ENCRYPTED),
        };
        assert_eq!(page.changes, vec![expected, expected]);
        assert_eq!(page.raw_count, 2);
        assert!(!page.has_more());
        assert_eq!(bytes(NEW_KEY)[0], 0x38);
        assert_eq!(bytes(NEW_KEY)[31], 0x33);
    }

    #[test]
    fn skips_failed_foreign_and_relayed_actions() {
        let body = page(vec![
            rotation(RELAYER, json!(NEW_KEY), json!(ENCRYPTED)),
            action(
                "change_wallet_key",
                json!(false),
                json!({ "destination": WALLET, "new_public_key": NEW_KEY }),
            ),
            action(
                "change_wallet_key",
                Value::Null,
                json!({ "destination": WALLET, "new_public_key": NEW_KEY }),
            ),
            action(
                "call_contract",
                json!(true),
                json!({ "opcode": "0xfbba99c8", "source": null, "destination": WALLET }),
            ),
            action("change_wallet_key", json!(true), json!({ "source": null })),
        ]);

        let page = parse_key_change_page(&body, &wallet()).expect("page parses");

        assert!(page.changes.is_empty());
        assert_eq!(page.raw_count, 5);
    }

    #[test]
    fn a_rotation_of_this_wallet_must_report_both_keys() {
        for rotation_without_keys in [
            rotation(WALLET, Value::Null, json!(ENCRYPTED)),
            rotation(WALLET, json!(NEW_KEY), Value::Null),
            action(
                "change_wallet_key",
                json!(true),
                json!({ "source": null, "destination": WALLET }),
            ),
        ] {
            let body = page(vec![rotation_without_keys]);
            assert!(
                parse_key_change_page(&body, &wallet()).is_err(),
                "{}",
                String::from_utf8_lossy(&body)
            );
        }

        let relayed_without_keys = page(vec![rotation(RELAYER, Value::Null, Value::Null)]);
        let page = parse_key_change_page(&relayed_without_keys, &wallet())
            .expect("another wallet's rotation is skipped before its keys are read");
        assert!(page.changes.is_empty());
    }

    #[test]
    fn rejects_malformed_pages_and_key_fields() {
        for body in [
            b"not json".to_vec(),
            b"{}".to_vec(),
            page(vec![rotation(WALLET, json!("00"), json!(ENCRYPTED))]),
            page(vec![rotation(
                WALLET,
                json!(NEW_KEY),
                json!(format!("{ENCRYPTED}00")),
            )]),
            page(vec![rotation(
                WALLET,
                json!(format!("+{}", "a".repeat(63))),
                json!(ENCRYPTED),
            )]),
            page(vec![rotation(
                WALLET,
                json!(format!("zz{}", "a".repeat(62))),
                json!(ENCRYPTED),
            )]),
            page(vec![rotation(
                "not an address",
                json!(NEW_KEY),
                json!(ENCRYPTED),
            )]),
            page(vec![action(
                "change_wallet_key",
                json!(true),
                json!("details"),
            )]),
        ] {
            assert!(
                parse_key_change_page(&body, &wallet()).is_err(),
                "{}",
                String::from_utf8_lossy(&body)
            );
        }
    }

    #[test]
    fn a_full_page_reports_more_rows() {
        let body = page(vec![
            action("change_wallet_key", json!(false), json!({}));
            KEY_CHANGE_PAGE_SIZE
        ]);

        let page = parse_key_change_page(&body, &wallet()).expect("page parses");

        assert!(page.changes.is_empty());
        assert!(page.has_more());
    }

    #[test]
    fn history_reaches_only_a_reported_key() {
        let changes = [KeyChange {
            new_public_key: [1; 32],
            encrypted_old_private_key: [2; 32],
        }];

        assert!(history_reaches(&changes, &[1; 32]));
        assert!(!history_reaches(&changes, &[2; 32]));
        assert!(!history_reaches(&[], &[1; 32]));
    }

    #[test]
    fn requests_this_wallets_rotations_newest_first() {
        let config = WalletClientConfig {
            record_id: NonEmptyString::try_from("key-history").expect("record ID"),
            address: wallet(),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig {
                toncenter_base_url: "https://provider.example/base".to_owned(),
                dns_root_address: None,
                request_timeout_ms: 15_000,
            },
        };

        let request = build_key_change_request(&config, HttpRequestId { value: 7 }, 200)
            .expect("request builds");

        assert_eq!(request.id.value, 7);
        assert_eq!(
            request.url,
            format!(
                "https://provider.example/base/api/v3/actions?account={}&action_type=change_wallet_key&limit=100&offset=200&sort=desc",
                WALLET.replace(':', "%3A")
            )
        );
    }
}
