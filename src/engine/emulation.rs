//! Toncenter Emulate API request construction and bounded response parsing.

use std::collections::HashMap;
use std::str::FromStr;

use base64::Engine as _;
use num_bigint::BigUint;
use serde::Deserialize;
use ton::ton_core::types::TonAddress;

use crate::domain::bounded_diagnostic;
use crate::{
    DomainError, ErrorCategory, ErrorCode, HttpHeader, HttpMethod, HttpRequest, HttpRequestId,
    RetryAdvice, SendEmulation, SendEmulationAction, WalletClientConfig, WalletClientError,
};

use super::http::build_toncenter_url;

const MAX_RESPONSE_BODY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESPONSE_HEADER_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
pub(super) struct EvaluatedEmulation {
    pub(super) summary: SendEmulation,
    pub(super) wallet_succeeded: bool,
    pub(super) compute_exit_code: Option<i32>,
    pub(super) action_result_code: Option<i32>,
}

pub(super) fn build_emulation_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    signed_boc: &[u8],
) -> Result<HttpRequest, WalletClientError> {
    let body = serde_json::to_vec(&serde_json::json!({
        "boc": base64::engine::general_purpose::STANDARD.encode(signed_boc),
        "ignore_chksig": true,
        "include_code_data": false,
        "include_address_book": false,
        "include_metadata": false,
        "with_actions": true,
        "mc_block_seqno": null,
    }))
    .map_err(|_| WalletClientError::StateUnavailable)?;

    Ok(HttpRequest {
        id,
        method: HttpMethod::Post,
        url: build_toncenter_url(config, &["api", "emulate", "v1", "emulateTrace"], &[])?,
        headers: vec![
            HttpHeader {
                name: "Accept".to_owned(),
                value: "application/json".to_owned(),
            },
            HttpHeader {
                name: "Content-Type".to_owned(),
                value: "application/json".to_owned(),
            },
        ],
        body,
        max_response_header_bytes: MAX_RESPONSE_HEADER_BYTES,
        max_response_body_bytes: MAX_RESPONSE_BODY_BYTES,
    })
}

pub(super) fn parse_emulation(
    body: &[u8],
    expected_source: &TonAddress,
) -> Result<EvaluatedEmulation, DomainError> {
    let response: EmulateTraceResponse =
        serde_json::from_slice(body).map_err(|error| invalid_response(error.to_string()))?;
    let wallet = response
        .transactions
        .get(&response.trace.tx_hash)
        .ok_or_else(|| invalid_response("emulation trace root transaction is missing"))?;
    let wallet_address = TonAddress::from_str(&wallet.account)
        .map_err(|error| invalid_response(format!("invalid emulated wallet address: {error}")))?;
    if &wallet_address != expected_source {
        return Err(invalid_response(
            "emulation trace root belongs to another account",
        ));
    }

    let wallet_fees = parse_nanograms(&wallet.total_fees, "wallet transaction fees")?;
    let trace_fees =
        response
            .transactions
            .values()
            .try_fold(BigUint::default(), |total, transaction| {
                parse_nanograms(&transaction.total_fees, "trace transaction fees")
                    .map(|fees| total + fees)
            })?;
    let transaction_count = u64::try_from(response.transactions.len())
        .map_err(|_| invalid_response("emulation transaction count does not fit u64"))?;
    let actions = response
        .actions
        .into_iter()
        .map(parse_action)
        .collect::<Result<Vec<_>, _>>()?;
    let wallet_succeeded = transaction_succeeded(wallet, true);
    let trace_succeeded = response
        .transactions
        .values()
        .all(|transaction| transaction_succeeded(transaction, false))
        && actions.iter().all(|action| action.succeeded);

    Ok(EvaluatedEmulation {
        summary: SendEmulation {
            mc_block_seqno: response.mc_block_seqno,
            wallet_fees_nanograms: wallet_fees.to_string(),
            trace_fees_nanograms: trace_fees.to_string(),
            transaction_count,
            actions,
            trace_succeeded,
            is_incomplete: response.is_incomplete,
        },
        wallet_succeeded,
        compute_exit_code: wallet
            .description
            .compute_ph
            .as_ref()
            .and_then(|phase| phase.exit_code),
        action_result_code: wallet
            .description
            .action
            .as_ref()
            .and_then(|phase| phase.result_code),
    })
}

pub(super) fn is_message_not_accepted(error: &DomainError) -> bool {
    let diagnostic = error.developer_message.to_ascii_lowercase();
    diagnostic.contains("external message") && diagnostic.contains("not accepted")
}

fn parse_action(action: RawEmulationAction) -> Result<SendEmulationAction, DomainError> {
    if action.kind.trim().is_empty() {
        return Err(invalid_response("emulation action type is empty"));
    }

    let accounts = action
        .accounts
        .into_iter()
        .map(|account| {
            TonAddress::from_str(&account)
                .map(|_| account)
                .map_err(|error| {
                    invalid_response(format!("invalid emulation action account: {error}"))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let transaction_hashes = action
        .transactions
        .into_iter()
        .map(|hash| {
            crate::Base64Hash::try_from(hash).map_err(|error| {
                invalid_response(format!(
                    "invalid emulation action transaction hash: {error}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let details_json = serde_json::to_string(&action.details)
        .map_err(|error| invalid_response(format!("invalid emulation action details: {error}")))?;

    Ok(SendEmulationAction {
        action_id: crate::Base64Hash::try_from(action.action_id)
            .map_err(|error| invalid_response(format!("invalid emulation action id: {error}")))?,
        kind: action.kind,
        succeeded: action.success,
        accounts,
        transaction_hashes,
        details_json,
    })
}

fn transaction_succeeded(transaction: &EmulatedTransaction, require_phases: bool) -> bool {
    if transaction.description.aborted != Some(false) {
        return false;
    }

    let compute = transaction.description.compute_ph.as_ref();
    let action = transaction.description.action.as_ref();
    let compute_succeeded =
        compute.map(|phase| phase.success == Some(true) && matches!(phase.exit_code, Some(0 | 1)));
    let action_succeeded =
        action.map(|phase| phase.success == Some(true) && phase.result_code == Some(0));

    if require_phases {
        compute_succeeded == Some(true) && action_succeeded == Some(true)
    } else {
        compute_succeeded != Some(false) && action_succeeded != Some(false)
    }
}

fn parse_nanograms(value: &str, field: &str) -> Result<BigUint, DomainError> {
    BigUint::from_str(value).map_err(|error| invalid_response(format!("invalid {field}: {error}")))
}

fn invalid_response(message: impl Into<String>) -> DomainError {
    DomainError {
        code: ErrorCode::InvalidProviderResponse,
        category: ErrorCategory::ProviderProtocol,
        retry: RetryAdvice::None,
        developer_message: bounded_diagnostic(message.into()),
        provider_status: None,
        retry_after_ms: None,
        host_kind: None,
    }
}

#[derive(Deserialize)]
struct EmulateTraceResponse {
    mc_block_seqno: u32,
    trace: EmulatedTraceNode,
    transactions: HashMap<String, EmulatedTransaction>,
    #[serde(default)]
    actions: Vec<RawEmulationAction>,
    #[serde(default)]
    is_incomplete: bool,
}

#[derive(Deserialize)]
struct RawEmulationAction {
    action_id: String,
    #[serde(rename = "type")]
    kind: String,
    success: bool,
    #[serde(default)]
    accounts: Vec<String>,
    #[serde(default)]
    transactions: Vec<String>,
    #[serde(default)]
    details: serde_json::Value,
}

#[derive(Deserialize)]
struct EmulatedTraceNode {
    tx_hash: String,
}

#[derive(Deserialize)]
struct EmulatedTransaction {
    account: String,
    total_fees: String,
    description: EmulatedTransactionDescription,
}

#[derive(Deserialize)]
struct EmulatedTransactionDescription {
    aborted: Option<bool>,
    compute_ph: Option<EmulatedComputePhase>,
    action: Option<EmulatedActionPhase>,
}

#[derive(Deserialize)]
struct EmulatedComputePhase {
    success: Option<bool>,
    exit_code: Option<i32>,
}

#[derive(Deserialize)]
struct EmulatedActionPhase {
    success: Option<bool>,
    result_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Network, ProviderConfig};

    const ADDRESS: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn builds_the_emulate_trace_request_from_the_provider_base() {
        let config = config("https://provider.example/custom/");
        let request = build_emulation_request(&config, HttpRequestId { value: 9 }, b"boc")
            .expect("emulation request must build");
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("request body must be JSON");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.url,
            "https://provider.example/custom/api/emulate/v1/emulateTrace"
        );
        assert_eq!(body["boc"], "Ym9j");
        assert_eq!(body["ignore_chksig"], true);
        assert_eq!(body["with_actions"], true);
    }

    #[test]
    fn parses_fees_and_detects_a_failed_child_transaction() {
        let source = TonAddress::from_str(ADDRESS).expect("test address must parse");
        let parsed = parse_emulation(
            serde_json::to_vec(&serde_json::json!({
                "mc_block_seqno": 17,
                "trace": { "tx_hash": "root", "children": [] },
                "transactions": {
                    "root": transaction(ADDRESS, "11", false, true, true, 0, 0),
                    "child": transaction(ADDRESS, "7", true, false, false, 32, 37),
                },
                "rand_seed": "seed",
                "is_incomplete": true,
            }))
            .expect("fixture must serialize")
            .as_slice(),
            &source,
        )
        .expect("fixture must parse");

        assert!(parsed.wallet_succeeded);
        assert_eq!(parsed.summary.mc_block_seqno, 17);
        assert_eq!(parsed.summary.wallet_fees_nanograms, "11");
        assert_eq!(parsed.summary.trace_fees_nanograms, "18");
        assert_eq!(parsed.summary.transaction_count, 2);
        assert!(!parsed.summary.trace_succeeded);
        assert!(parsed.summary.is_incomplete);
    }

    #[test]
    fn returns_validated_high_level_actions() {
        let source = TonAddress::from_str(ADDRESS).expect("test address must parse");
        let action_id = base64::engine::general_purpose::STANDARD.encode([3_u8; 32]);
        let transaction_hash = base64::engine::general_purpose::STANDARD.encode([4_u8; 32]);
        let parsed = parse_emulation(
            serde_json::to_vec(&serde_json::json!({
                "mc_block_seqno": 17,
                "trace": { "tx_hash": "root" },
                "transactions": {
                    "root": transaction(ADDRESS, "11", false, true, true, 0, 0),
                },
                "actions": [{
                    "action_id": action_id,
                    "type": "ton_transfer",
                    "success": true,
                    "accounts": [ADDRESS],
                    "transactions": [transaction_hash],
                    "details": { "value": "7", "destination": ADDRESS }
                }],
                "is_incomplete": false,
            }))
            .expect("fixture must serialize")
            .as_slice(),
            &source,
        )
        .expect("fixture must parse");

        assert_eq!(parsed.summary.actions.len(), 1);
        let action = &parsed.summary.actions[0];
        assert_eq!(action.kind, "ton_transfer");
        assert!(action.succeeded);
        assert_eq!(action.accounts, [ADDRESS]);
        assert_eq!(action.action_id.as_str(), action_id);
        assert_eq!(action.transaction_hashes[0].as_str(), transaction_hash);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&action.details_json)
                .expect("details must remain valid JSON"),
            serde_json::json!({ "value": "7", "destination": ADDRESS })
        );
    }

    #[test]
    fn nonzero_compute_and_action_codes_reject_even_inconsistent_success_flags() {
        let source = TonAddress::from_str(ADDRESS).expect("test address must parse");

        for (compute_exit_code, action_result_code) in [(33, 0), (0, 34)] {
            let parsed = parse_emulation(
                serde_json::to_vec(&serde_json::json!({
                    "mc_block_seqno": 17,
                    "trace": { "tx_hash": "root" },
                    "transactions": {
                        "root": transaction(
                            ADDRESS,
                            "11",
                            false,
                            true,
                            true,
                            compute_exit_code,
                            action_result_code,
                        ),
                    },
                    "is_incomplete": false,
                }))
                .expect("fixture must serialize")
                .as_slice(),
                &source,
            )
            .expect("fixture must parse");

            assert!(!parsed.wallet_succeeded);
        }
    }

    #[test]
    fn tvm_alternative_success_exit_code_is_accepted() {
        let source = TonAddress::from_str(ADDRESS).expect("test address must parse");
        let parsed = parse_emulation(
            serde_json::to_vec(&serde_json::json!({
                "mc_block_seqno": 17,
                "trace": { "tx_hash": "root" },
                "transactions": {
                    "root": transaction(ADDRESS, "11", false, true, true, 1, 0),
                },
                "is_incomplete": false,
            }))
            .expect("fixture must serialize")
            .as_slice(),
            &source,
        )
        .expect("fixture must parse");

        assert!(parsed.wallet_succeeded);
    }

    #[test]
    fn recognizes_the_provider_message_not_accepted_diagnostic() {
        let error = DomainError {
            code: ErrorCode::HttpRejected,
            category: ErrorCategory::ProviderProtocol,
            retry: RetryAdvice::None,
            developer_message: "TVM execution error: External message was not accepted".to_owned(),
            provider_status: Some(422),
            retry_after_ms: None,
            host_kind: None,
        };

        assert!(is_message_not_accepted(&error));
    }

    #[test]
    fn rejects_a_trace_root_for_another_wallet() {
        let source = TonAddress::from_str(ADDRESS).expect("test address must parse");
        let other = "0:2222222222222222222222222222222222222222222222222222222222222222";
        let body = serde_json::to_vec(&serde_json::json!({
            "mc_block_seqno": 17,
            "trace": { "tx_hash": "root" },
            "transactions": {
                "root": transaction(other, "1", false, true, true, 0, 0),
            },
            "is_incomplete": false,
        }))
        .expect("fixture must serialize");

        let error = parse_emulation(&body, &source).expect_err("wrong source must fail");
        assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
        assert_eq!(
            error.developer_message,
            "emulation trace root belongs to another account"
        );
    }

    fn transaction(
        account: &str,
        total_fees: &str,
        aborted: bool,
        compute_success: bool,
        action_success: bool,
        exit_code: i32,
        result_code: i32,
    ) -> serde_json::Value {
        serde_json::json!({
            "account": account,
            "total_fees": total_fees,
            "description": {
                "type": "ord",
                "aborted": aborted,
                "compute_ph": { "success": compute_success, "exit_code": exit_code },
                "action": { "success": action_success, "result_code": result_code },
            }
        })
    }

    fn config(base: &str) -> WalletClientConfig {
        WalletClientConfig {
            record_id: "record".to_owned(),
            address: ADDRESS.to_owned(),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            providers: ProviderConfig {
                toncenter_base_url: base.to_owned(),
            },
        }
    }
}
