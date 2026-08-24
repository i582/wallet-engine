//! Fresh NFT ownership validation and typed TEP-62 preview/send entry points.

use crate::domain::bounded_diagnostic;
use crate::transport::process_response;
use crate::wallet::nft_transfer::canonicalize_nft_transfer;
use crate::{
    NftItem, NftTransferPreviewRequest, NftTransferRequest, SendPreview, SendPreviewRequest,
    SendRequest, SendResult, TonAddressString, WalletClientError,
};

use super::WalletClient;
use super::nft::{build_nft_item_request, parse_single_nft_item};
use super::state::ensure_running;

#[uniffi::export]
impl WalletClient {
    /// Validates current ownership and emulates one typed TEP-62 NFT transfer.
    ///
    /// Reuse this request's operation ID for [`Self::send_nft_transfer`] after
    /// user confirmation so the TEP-62 query ID remains unchanged.
    pub async fn preview_nft_transfer(
        &self,
        request: NftTransferPreviewRequest,
    ) -> Result<SendPreview, WalletClientError> {
        let source = self.nft_transfer_source()?;
        let canonical = canonicalize_nft_transfer(&request.operation_id, &source, &request.intent)
            .map_err(|error| {
                nft_unavailable(format!("failed to build TEP-62 transfer: {error}"))
            })?;
        let item = self
            .load_fresh_nft_for_transfer(&request.intent.nft_address)
            .await?;
        validate_transferable_nft(&item, &source)?;

        let preview = self
            .preview_send(SendPreviewRequest {
                intent: canonical.intent,
            })
            .await?;
        validate_nft_emulation(&preview, &request.intent.nft_address)?;
        Ok(preview)
    }

    /// Revalidates current ownership and signs/submits one typed TEP-62 transfer.
    ///
    /// Provider acceptance confirms the source wallet message, not final NFT
    /// ownership. Applications must refresh NFT state after confirmation.
    pub async fn send_nft_transfer(
        &self,
        request: NftTransferRequest,
    ) -> Result<SendResult, WalletClientError> {
        let source = self.nft_transfer_source()?;
        let canonical = canonicalize_nft_transfer(&request.operation_id, &source, &request.intent)
            .map_err(|error| {
                nft_unavailable(format!("failed to build TEP-62 transfer: {error}"))
            })?;
        let item = self
            .load_fresh_nft_for_transfer(&request.intent.nft_address)
            .await?;
        validate_transferable_nft(&item, &source)?;

        self.send(SendRequest {
            operation_id: request.operation_id,
            force: request.force,
            intent: canonical.intent,
        })
        .await
    }
}

impl WalletClient {
    fn nft_transfer_source(&self) -> Result<TonAddressString, WalletClientError> {
        let state = self.lock()?;
        ensure_running(&state)?;
        Ok(state.config.address.clone())
    }

    async fn load_fresh_nft_for_transfer(
        &self,
        address: &TonAddressString,
    ) -> Result<NftItem, WalletClientError> {
        let (request, network) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            let request_id = state.allocate_request_id()?;
            let request = build_nft_item_request(&state.config, request_id, address)?;
            (request, state.config.network)
        };

        let body = process_response(&request, self.http_host.execute_http(request.clone()).await)
            .map_err(|error| {
            nft_unavailable(format!(
                "failed to load fresh NFT state: {}",
                error.developer_message
            ))
        })?;
        {
            let state = self.lock()?;
            ensure_running(&state)?;
        }
        parse_single_nft_item(&body, address, network).map_err(|error| {
            nft_unavailable(format!(
                "failed to parse fresh NFT state: {}",
                error.developer_message
            ))
        })
    }
}

fn validate_transferable_nft(
    item: &NftItem,
    source: &TonAddressString,
) -> Result<(), WalletClientError> {
    if !item.initialized {
        return Err(nft_unavailable("NFT item is not initialized"));
    }
    if item.owner_address.as_ref() != Some(source) {
        return Err(nft_unavailable(
            "the current wallet is not the NFT item's direct owner",
        ));
    }
    if item.on_sale
        || item.sale_contract_address.is_some()
        || item.auction_contract_address.is_some()
    {
        return Err(nft_unavailable(
            "NFT item is controlled by an active sale or auction",
        ));
    }
    Ok(())
}

fn validate_nft_emulation(
    preview: &SendPreview,
    nft_address: &TonAddressString,
) -> Result<(), WalletClientError> {
    if preview.emulation.is_incomplete {
        return Err(nft_emulation_rejected(
            "Toncenter returned an incomplete NFT transfer trace",
        ));
    }
    let item_succeeded = preview
        .emulation
        .transactions
        .iter()
        .any(|transaction| transaction.account == *nft_address && transaction.succeeded);
    if item_succeeded {
        Ok(())
    } else {
        Err(nft_emulation_rejected(
            "emulation did not contain a successful NFT item transaction",
        ))
    }
}

fn nft_unavailable(diagnostic: impl Into<String>) -> WalletClientError {
    WalletClientError::NftTransferUnavailable {
        diagnostic: bounded_diagnostic(diagnostic.into()),
    }
}

fn nft_emulation_rejected(diagnostic: impl Into<String>) -> WalletClientError {
    WalletClientError::NftTransferEmulationRejected {
        diagnostic: bounded_diagnostic(diagnostic.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{
        Base64Hash, NonEmptyString, SendEmulation, SendEmulationAction, SendEmulationTransaction,
        UnsignedDecimalString,
    };

    use super::*;

    const SOURCE: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";
    const NFT: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";
    const RECIPIENT: &str = "0:3333333333333333333333333333333333333333333333333333333333333333";

    #[test]
    fn direct_owner_can_transfer_an_initialized_unsold_item() {
        assert!(validate_transferable_nft(&item(SOURCE), &address(SOURCE)).is_ok());
    }

    #[test]
    fn effective_owner_cannot_transfer_while_sale_contract_is_direct_owner() {
        let mut item = item(NFT);
        item.real_owner = Some(address(SOURCE));
        item.on_sale = true;
        item.sale_contract_address = Some(address(NFT));

        assert!(matches!(
            validate_transferable_nft(&item, &address(SOURCE)),
            Err(WalletClientError::NftTransferUnavailable { .. })
        ));
    }

    #[test]
    fn preview_requires_a_complete_successful_item_transaction() {
        let mut preview = preview();
        assert!(validate_nft_emulation(&preview, &address(NFT)).is_ok());

        // A notification or excess message can fail after the item contract
        // has already completed the ownership change.
        preview.emulation.trace_succeeded = false;
        assert!(validate_nft_emulation(&preview, &address(NFT)).is_ok());

        preview.emulation.is_incomplete = true;
        assert!(matches!(
            validate_nft_emulation(&preview, &address(NFT)),
            Err(WalletClientError::NftTransferEmulationRejected { .. })
        ));
    }

    #[test]
    fn preview_does_not_depend_on_toncenter_action_recognition() {
        let mut preview = preview();
        preview.emulation.actions[0].succeeded = false;
        preview.emulation.actions[0].details_json = "not even json".to_owned();

        assert!(validate_nft_emulation(&preview, &address(NFT)).is_ok());
    }

    #[test]
    fn preview_rejects_a_failed_item_transaction() {
        let mut preview = preview();
        preview.emulation.transactions[0].succeeded = false;

        assert!(matches!(
            validate_nft_emulation(&preview, &address(NFT)),
            Err(WalletClientError::NftTransferEmulationRejected { .. })
        ));
    }

    #[test]
    fn self_transfer_accepts_a_successful_item_transaction_when_follow_up_fails() {
        let mut preview = preview();
        preview.emulation.trace_succeeded = false;
        preview.emulation.actions[0].details_json = serde_json::json!({
            "query_id": "7",
            "nft_item": NFT,
            "new_owner": SOURCE,
        })
        .to_string();

        assert!(validate_nft_emulation(&preview, &address(NFT)).is_ok());
    }

    fn address(value: &str) -> TonAddressString {
        TonAddressString::try_from(value).expect("valid address")
    }

    fn item(owner: &str) -> NftItem {
        NftItem {
            address: address(NFT),
            collection_address: None,
            collection: None,
            owner_address: Some(address(owner)),
            real_owner: None,
            sale_contract_address: None,
            auction_contract_address: None,
            index: UnsignedDecimalString::from(0_u64),
            last_transaction_lt: UnsignedDecimalString::from(1_u64),
            initialized: true,
            on_sale: false,
            code_hash: "code".to_owned(),
            data_hash: "data".to_owned(),
            content: HashMap::new(),
            is_nsfw: None,
            is_scam: None,
        }
    }

    fn preview() -> SendPreview {
        SendPreview {
            messages: Vec::new(),
            valid_until: 1,
            message_boc_base64: Boc::try_from(TonCell::EMPTY_BOC.to_vec()).expect("BOC"),
            emulation: SendEmulation {
                mc_block_seqno: 1,
                wallet_fees_nanograms: UnsignedDecimalString::from(1_u64),
                trace_fees_nanograms: UnsignedDecimalString::from(2_u64),
                transaction_count: 2,
                actions: vec![SendEmulationAction {
                    action_id: Base64Hash::from_bytes(&[1; 32]).expect("action ID"),
                    kind: NonEmptyString::try_from("nft_transfer").expect("kind"),
                    succeeded: true,
                    accounts: vec![address(NFT), address(SOURCE), address(RECIPIENT)],
                    transaction_hashes: vec![
                        Base64Hash::from_bytes(&[2; 32]).expect("transaction hash"),
                    ],
                    details_json: serde_json::json!({
                        "query_id": "7",
                        "nft_item": NFT,
                        "new_owner": RECIPIENT,
                    })
                    .to_string(),
                }],
                transactions: vec![SendEmulationTransaction {
                    account: address(NFT),
                    succeeded: true,
                    is_root: false,
                }],
                trace_succeeded: true,
                is_incomplete: false,
            },
        }
    }

    use crate::Boc;
    use ton::ton_core::cell::TonCell;
}
