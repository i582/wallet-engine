use crate::block_tlb::{SEND_MODE_IGNORE_ERRORS, SEND_MODE_PAY_FEES_SEPARATELY};
use crate::errors::TonError;
use crate::ton_wallet::WalletVersion::*;
use crate::ton_wallet::*;
use ton_core::bail_ton_core;
use ton_core::cell::{TonCell, TonHash};
use ton_core::errors::TonCoreError;
use ton_core::traits::tlb::TLB;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum WalletVersion {
    V1R1,
    V1R2,
    V1R3,
    V2R1,
    V2R2,
    V3R1,
    V3R2,
    V4R1,
    V4R2,
    V5R1,
    Wallet,
    HLV1R1,
    HLV1R2,
    HLV2,
    HLV2R1,
    HLV2R2,
}

impl WalletVersion {
    pub fn get_default_data(
        version: WalletVersion,
        key_pair: &KeyPair,
        wallet_id: i32,
    ) -> Result<TonCell, TonCoreError> {
        let public_key = TonHash::from_slice(&key_pair.public_key)?;
        match version {
            V1R1 | V1R2 | V1R3 | V2R1 | V2R2 => WalletV1V2Data::new(public_key).to_cell(),
            V3R1 | V3R2 => WalletV3Data::new(wallet_id, public_key).to_cell(),
            V4R1 | V4R2 => WalletV4Data::new(wallet_id, public_key).to_cell(),
            V5R1 => WalletV5Data::new(wallet_id, public_key).to_cell(),
            Wallet => WalletData::new(wallet_id, public_key).to_cell(),
            HLV2R2 => WalletHLV2R2Data::new(wallet_id, public_key).to_cell(),
            HLV1R1 | HLV1R2 | HLV2 | HLV2R1 => {
                bail_ton_core!("initial_data for {version:?} is unsupported");
            }
        }
    }

    pub fn get_code(version: WalletVersion) -> Result<&'static TonCell, TonCoreError> {
        TON_WALLET_CODE_BY_VERSION
            .get(&version)
            .ok_or_else(|| TonCoreError::Custom(format!("No code found for {version:?}")))
    }

    pub fn get_version_by_code(code_hash: TonHash) -> Result<WalletVersion, TonCoreError> {
        TON_WALLET_VERSION_BY_CODE
            .get(&code_hash)
            .copied()
            .ok_or_else(|| {
                TonCoreError::Custom(format!("No version found for code_hash: {code_hash}"))
            })
    }

    pub fn build_ext_in_body(
        version: WalletVersion,
        valid_until: u32,
        msg_seqno: u32,
        wallet_id: i32,
        msgs: Vec<TonCell>,
    ) -> Result<TonCell, TonError> {
        let modes = vec![SEND_MODE_PAY_FEES_SEPARATELY | SEND_MODE_IGNORE_ERRORS; msgs.len()];
        Self::build_ext_in_body_with_modes(version, valid_until, msg_seqno, wallet_id, msgs, modes)
    }

    pub fn build_ext_in_body_with_modes(
        version: WalletVersion,
        valid_until: u32,
        msg_seqno: u32,
        wallet_id: i32,
        msgs: Vec<TonCell>,
        msgs_modes: Vec<u8>,
    ) -> Result<TonCell, TonError> {
        let res = match version {
            V2R1 | V2R2 => WalletV2ExtMsgBody {
                msg_seqno,
                valid_until,
                msgs_modes,
                msgs,
            }
            .to_cell(),
            V3R1 | V3R2 => WalletV3ExtMsgBody {
                subwallet_id: wallet_id,
                msg_seqno,
                valid_until,
                msgs_modes,
                msgs,
            }
            .to_cell(),
            V4R1 | V4R2 => WalletV4ExtMsgBody {
                subwallet_id: wallet_id,
                valid_until,
                msg_seqno,
                opcode: 0,
                msgs_modes,
                msgs,
            }
            .to_cell(),
            V5R1 | Wallet => WalletV5ExtMsgBody {
                wallet_id,
                valid_until,
                msg_seqno,
                msgs_modes,
                msgs,
            }
            .to_cell(),
            _ => Err(TonCoreError::Custom(format!(
                "build_ext_in_body for {version:?} is unsupported"
            ))),
        };
        res.map_err(TonError::from)
    }

    /// Builds an unsigned owner-authorized request for delivery by an internal message.
    pub fn build_internal_signed_body_with_modes(
        version: WalletVersion,
        valid_until: u32,
        msg_seqno: u32,
        wallet_id: i32,
        msgs: Vec<TonCell>,
        msgs_modes: Vec<u8>,
    ) -> Result<TonCell, TonError> {
        match version {
            V5R1 | Wallet => WalletV5InternalSignedBody {
                wallet_id,
                valid_until,
                msg_seqno,
                msgs_modes,
                msgs,
            }
            .to_cell()
            .map_err(TonError::from),
            _ => Err(TonError::from(TonCoreError::Custom(format!(
                "internal signed requests are unsupported for {version:?}"
            )))),
        }
    }

    pub(super) fn sign_msg(
        version: WalletVersion,
        msg_cell: &TonCell,
        sign: &[u8],
    ) -> Result<TonCell, TonError> {
        match version {
            // different order
            V5R1 | Wallet => {
                let mut builder = TonCell::builder();
                builder.write_cell(msg_cell)?;
                builder.write_bits(sign, sign.len() * 8)?;
                Ok(builder.build()?)
            }
            _ => {
                let mut builder = TonCell::builder();
                builder.write_bits(sign, sign.len() * 8)?;
                builder.write_cell(msg_cell)?;
                Ok(builder.build()?)
            }
        }
    }
}
