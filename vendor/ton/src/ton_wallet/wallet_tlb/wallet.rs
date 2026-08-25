use crate::ton_wallet::wallet_tlb::wallet_ext_msg_utils::validate_msgs_count;
use ton_core::TLB;
use ton_core::bail_ton_core;
use ton_core::cell::{CellBuilder, CellParser, TonCell, TonHash};
use ton_core::errors::TonCoreError;
use ton_core::traits::tlb::TLB as _;

const SEND_ONE_INTERNAL_OPCODE: u32 = 0x6389_6e74;
const SEND_ONE_EXTERNAL_OPCODE: u32 = 0x6389_6e75;
const SEND_BULK_INTERNAL_OPCODE: u32 = 0x7389_6e74;
const SEND_BULK_EXTERNAL_OPCODE: u32 = 0x7389_6e75;

/// Initial data for Wallet revision 00.
#[derive(Debug, PartialEq, Clone, TLB)]
pub struct WalletData {
    pub revision: u8,
    pub seqno: u32,
    pub wallet_id: u32,
    pub public_key: TonHash,
    pub was_key_changed: bool,
}

impl WalletData {
    /// Creates undeployed revision-00 storage with key rotation unused.
    pub fn new(wallet_id: i32, public_key: TonHash) -> Self {
        Self {
            revision: 0,
            seqno: 0,
            wallet_id: u32::from_be_bytes(wallet_id.to_be_bytes()),
            public_key,
            was_key_changed: false,
        }
    }
}

/// An unsigned external Wallet request.
#[derive(Debug, PartialEq, Clone)]
pub struct WalletExtMsgBody {
    pub wallet_id: i32,
    pub valid_until: u32,
    pub msg_seqno: u32,
    pub msgs_modes: Vec<u8>,
    pub msgs: Vec<TonCell>,
}

impl WalletExtMsgBody {
    pub fn to_cell(&self) -> Result<TonCell, TonCoreError> {
        build_request(
            self.wallet_id,
            self.valid_until,
            self.msg_seqno,
            &self.msgs,
            &self.msgs_modes,
            true,
        )
    }

    /// Decodes a signature-prefixed external Wallet request.
    pub fn read_signed(parser: &mut CellParser) -> Result<(Self, Vec<u8>), TonCoreError> {
        let signature = parser.read_bits(512)?;
        let request = read_request(parser, true)?;
        Ok((
            Self {
                wallet_id: request.wallet_id,
                valid_until: request.valid_until,
                msg_seqno: request.msg_seqno,
                msgs_modes: request.msgs_modes,
                msgs: request.msgs,
            },
            signature,
        ))
    }
}

/// An unsigned owner-authorized Wallet request delivered internally.
#[derive(Debug, PartialEq, Clone)]
pub struct WalletInternalSignedBody {
    pub wallet_id: i32,
    pub valid_until: u32,
    pub msg_seqno: u32,
    pub msgs_modes: Vec<u8>,
    pub msgs: Vec<TonCell>,
}

impl WalletInternalSignedBody {
    pub fn to_cell(&self) -> Result<TonCell, TonCoreError> {
        build_request(
            self.wallet_id,
            self.valid_until,
            self.msg_seqno,
            &self.msgs,
            &self.msgs_modes,
            false,
        )
    }

    /// Decodes a signature-prefixed internal Wallet request.
    pub fn read_signed(parser: &mut CellParser) -> Result<(Self, Vec<u8>), TonCoreError> {
        let signature = parser.read_bits(512)?;
        let request = read_request(parser, false)?;
        Ok((
            Self {
                wallet_id: request.wallet_id,
                valid_until: request.valid_until,
                msg_seqno: request.msg_seqno,
                msgs_modes: request.msgs_modes,
                msgs: request.msgs,
            },
            signature,
        ))
    }
}

struct ParsedRequest {
    wallet_id: i32,
    valid_until: u32,
    msg_seqno: u32,
    msgs_modes: Vec<u8>,
    msgs: Vec<TonCell>,
}

fn build_request(
    wallet_id: i32,
    valid_until: u32,
    msg_seqno: u32,
    msgs: &[TonCell],
    msgs_modes: &[u8],
    external: bool,
) -> Result<TonCell, TonCoreError> {
    validate_msgs_count(msgs, msgs_modes, u8::MAX.into())?;
    if msgs.is_empty() {
        bail_ton_core!("Wallet request must contain at least one message");
    }

    let mut builder = TonCell::builder();
    let opcode = match (external, msgs.len()) {
        (true, 1) => SEND_ONE_EXTERNAL_OPCODE,
        (false, 1) => SEND_ONE_INTERNAL_OPCODE,
        (true, _) => SEND_BULK_EXTERNAL_OPCODE,
        (false, _) => SEND_BULK_INTERNAL_OPCODE,
    };
    opcode.write(&mut builder)?;
    u32::from_be_bytes(wallet_id.to_be_bytes()).write(&mut builder)?;
    valid_until.write(&mut builder)?;
    msg_seqno.write(&mut builder)?;

    if msgs.len() == 1 {
        msgs_modes[0].write(&mut builder)?;
        builder.write_ref(msgs[0].clone())?;
    } else {
        write_message_array(&mut builder, msgs, msgs_modes)?;
    }
    builder.build()
}

fn write_message_array(
    builder: &mut CellBuilder,
    msgs: &[TonCell],
    msgs_modes: &[u8],
) -> Result<(), TonCoreError> {
    let len = u8::try_from(msgs.len())
        .map_err(|_| TonCoreError::Custom("Wallet message array exceeds 255 items".to_owned()))?;
    len.write(builder)?;

    let mut next = None;
    let mut end = msgs.len();
    while end > 0 {
        // Tolk arrays reserve one reference for the next chunk even for the
        // tail, so MessageToSend arrays use at most three items per chunk.
        let chunk_size = end.min(3);
        let start = end - chunk_size;
        let mut chunk = TonCell::builder();
        chunk.write_bit(next.is_some())?;
        if let Some(next) = next {
            chunk.write_ref(next)?;
        }
        for index in start..end {
            msgs_modes[index].write(&mut chunk)?;
            chunk.write_ref(msgs[index].clone())?;
        }
        next = Some(chunk.build()?);
        end = start;
    }

    builder.write_bit(true)?;
    let head = next
        .ok_or_else(|| TonCoreError::Custom("Wallet message array must have a head".to_owned()))?;
    builder.write_ref(head)?;
    Ok(())
}

fn read_request(parser: &mut CellParser, external: bool) -> Result<ParsedRequest, TonCoreError> {
    let opcode = parser.read_num::<u32>(32)?;
    let wallet_id = i32::from_be_bytes(parser.read_num::<u32>(32)?.to_be_bytes());
    let valid_until = parser.read_num::<u32>(32)?;
    let msg_seqno = parser.read_num::<u32>(32)?;

    let single_opcode = if external {
        SEND_ONE_EXTERNAL_OPCODE
    } else {
        SEND_ONE_INTERNAL_OPCODE
    };
    let bulk_opcode = if external {
        SEND_BULK_EXTERNAL_OPCODE
    } else {
        SEND_BULK_INTERNAL_OPCODE
    };
    let (msgs_modes, msgs) = if opcode == single_opcode {
        let mode = parser.read_num::<u8>(8)?;
        let msg = parser.read_next_ref()?.clone();
        (vec![mode], vec![msg])
    } else if opcode == bulk_opcode {
        read_message_array(parser)?
    } else {
        bail_ton_core!("unsupported Wallet request opcode {opcode:#010x}");
    };
    parser.ensure_empty()?;

    Ok(ParsedRequest {
        wallet_id,
        valid_until,
        msg_seqno,
        msgs_modes,
        msgs,
    })
}

fn read_message_array(parser: &mut CellParser) -> Result<(Vec<u8>, Vec<TonCell>), TonCoreError> {
    let expected_len = usize::from(parser.read_num::<u8>(8)?);
    if !parser.read_bit()? {
        bail_ton_core!("Wallet message array must have a head");
    }

    let mut head = Some(parser.read_next_ref()?.clone());
    let mut msgs_modes = Vec::with_capacity(expected_len);
    let mut msgs = Vec::with_capacity(expected_len);
    while let Some(chunk) = head {
        let mut chunk_parser = chunk.parser();
        head = if chunk_parser.read_bit()? {
            Some(chunk_parser.read_next_ref()?.clone())
        } else {
            None
        };
        while chunk_parser.refs_left() > 0 {
            msgs_modes.push(chunk_parser.read_num::<u8>(8)?);
            msgs.push(chunk_parser.read_next_ref()?.clone());
        }
        chunk_parser.ensure_empty()?;
    }
    if msgs.is_empty() || msgs.len() != expected_len {
        bail_ton_core!(
            "Wallet message array length mismatch: expected {expected_len}, got {}",
            msgs.len()
        );
    }
    Ok((msgs_modes, msgs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(count: usize) -> Vec<TonCell> {
        (0..count)
            .map(|index| {
                let mut builder = TonCell::builder();
                builder.write_num(&index, usize::BITS as usize).unwrap();
                builder.build().unwrap()
            })
            .collect()
    }

    #[test]
    fn wallet_storage_matches_revision_zero_layout() -> anyhow::Result<()> {
        let public_key = TonHash::from([0x5a; 32]);
        let data = WalletData::new(0x7fff_7ffd, public_key).to_cell()?;
        let mut parser = data.parser();

        assert_eq!(parser.read_num::<u8>(8)?, 0);
        assert_eq!(parser.read_num::<u32>(32)?, 0);
        assert_eq!(parser.read_num::<u32>(32)?, 0x7fff_7ffd);
        assert_eq!(parser.read_bits(256)?, vec![0x5a; 32]);
        assert!(!parser.read_bit()?);
        parser.ensure_empty()?;
        Ok(())
    }

    #[test]
    fn external_single_request_uses_single_opcode_and_inline_item() -> anyhow::Result<()> {
        let msgs = messages(1);
        let body = WalletExtMsgBody {
            wallet_id: 0x7fff_7ffd,
            valid_until: 1_900_000_000,
            msg_seqno: 17,
            msgs_modes: vec![3],
            msgs: msgs.clone(),
        }
        .to_cell()?;
        let mut parser = body.parser();

        assert_eq!(parser.read_num::<u32>(32)?, SEND_ONE_EXTERNAL_OPCODE);
        assert_eq!(parser.read_num::<u32>(32)?, 0x7fff_7ffd);
        assert_eq!(parser.read_num::<u32>(32)?, 1_900_000_000);
        assert_eq!(parser.read_num::<u32>(32)?, 17);
        assert_eq!(parser.read_num::<u8>(8)?, 3);
        assert_eq!(parser.read_next_ref()?, &msgs[0]);
        parser.ensure_empty()?;
        Ok(())
    }

    #[test]
    fn bulk_request_uses_standard_array_chunks_in_source_order() -> anyhow::Result<()> {
        let msgs = messages(8);
        let modes = (0_u8..8).collect::<Vec<_>>();
        let body = WalletExtMsgBody {
            wallet_id: 0x7fff_7ffd,
            valid_until: 1_900_000_000,
            msg_seqno: 17,
            msgs_modes: modes.clone(),
            msgs: msgs.clone(),
        }
        .to_cell()?;
        let mut parser = body.parser();

        assert_eq!(parser.read_num::<u32>(32)?, SEND_BULK_EXTERNAL_OPCODE);
        let _ = parser.read_bits(96)?;
        assert_eq!(parser.read_num::<u8>(8)?, 8);
        assert!(parser.read_bit()?);
        let mut head = Some(parser.read_next_ref()?.clone());
        let mut index = 0;
        while let Some(chunk) = head {
            let mut chunk_parser = chunk.parser();
            head = if chunk_parser.read_bit()? {
                Some(chunk_parser.read_next_ref()?.clone())
            } else {
                None
            };
            while chunk_parser.refs_left() > 0 {
                assert_eq!(chunk_parser.read_num::<u8>(8)?, modes[index]);
                assert_eq!(chunk_parser.read_next_ref()?, &msgs[index]);
                index += 1;
            }
            chunk_parser.ensure_empty()?;
        }
        assert_eq!(index, msgs.len());
        parser.ensure_empty()?;
        Ok(())
    }

    #[test]
    fn internal_request_uses_channel_specific_opcode() -> anyhow::Result<()> {
        let body = WalletInternalSignedBody {
            wallet_id: 0x7fff_7ffd,
            valid_until: 1_900_000_000,
            msg_seqno: 17,
            msgs_modes: vec![3],
            msgs: messages(1),
        }
        .to_cell()?;

        assert_eq!(body.parser().read_num::<u32>(32)?, SEND_ONE_INTERNAL_OPCODE);
        Ok(())
    }

    #[test]
    fn request_rejects_empty_mismatched_and_oversized_arrays() {
        let request = |msgs, msgs_modes| WalletExtMsgBody {
            wallet_id: 0x7fff_7ffd,
            valid_until: 1_900_000_000,
            msg_seqno: 17,
            msgs_modes,
            msgs,
        };

        assert!(request(vec![], vec![]).to_cell().is_err());
        assert!(request(messages(1), vec![]).to_cell().is_err());
        assert!(request(messages(256), vec![3; 256]).to_cell().is_err());
    }
}
