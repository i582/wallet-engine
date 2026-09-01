import type {Base64Hash, SendEmulation, SendPhase} from "./types"

/** The amount policy applied by the wallet contract. */
export type SendAmount =
  | {
      readonly kind: "exact"
      /** Exact positive value in canonical base-10 nanograms. */
      readonly nanograms: string
    }
  | {
      /** Transfer the complete remaining balance after network fees. */
      readonly kind: "all"
    }

/** The body encoded into one outgoing internal TON message. */
export type SendMessageBody =
  | {
      /** Encode no body bits or references. */
      readonly kind: "empty"
    }
  | {
      /** Encode a zero opcode followed by this UTF-8 text as TON snake data. */
      readonly kind: "comment"
      readonly text: string
    }
  | {
      /** Preserve this Base64-encoded BOC cell as the message body. */
      readonly kind: "rawPayload"
      readonly boc: string
    }

/** One outgoing internal TON message. */
export interface SendMessage {
  readonly destination: string
  readonly amount: SendAmount
  readonly body: SendMessageBody
  /** Return the remaining value when a destination-contract call fails. */
  readonly bounce?: boolean
  /** Destination-contract StateInit encoded as a Base64 BOC. */
  readonly stateInit?: string | null
}

/** Explicit TON values used to execute a TEP-62 NFT transfer. */
export type NftTransferFunding = {
  readonly kind: "exact"
  /** Value attached to the NFT item contract call. */
  readonly attachedNanograms: string
  /** Value forwarded to the new owner with ownership_assigned. */
  readonly forwardNanograms: string
}

/** Optional payload forwarded to the new NFT owner. */
export type NftTransferPayload =
  | {readonly kind: "empty"}
  | {readonly kind: "comment"; readonly text: string}
  | {readonly kind: "rawPayload"; readonly boc: string}

/** Immutable choices for one TEP-62 NFT transfer. */
export interface NftTransferIntent {
  readonly nftAddress: string
  readonly recipient: string
  readonly funding: NftTransferFunding
  readonly payload: NftTransferPayload
  readonly expiration: SendExpiration
}

/** Requests fresh ownership validation and NFT transfer emulation. */
export interface NftTransferPreviewRequest {
  /** Reuse this ID for the signed request to preserve the TEP-62 query ID. */
  readonly operationId: string
  readonly intent: NftTransferIntent
}

/** Requests a signed TEP-62 NFT transfer. */
export interface NftTransferRequest {
  readonly operationId: string
  readonly force?: boolean
  readonly intent: NftTransferIntent
}

/** The policy used to select the wallet message expiration boundary. */
export type SendExpiration =
  | {
      /** Derive expiration from fresh provider time and engine configuration. */
      readonly kind: "engineDefault"
    }
  | {
      /** Preserve this Unix expiration timestamp in seconds. */
      readonly kind: "exact"
      readonly unixTimestamp: number
    }

/** The message and expiration choices accepted by preview and send. */
export interface SendIntent {
  readonly expiration: SendExpiration
  /** Non-empty ordered Wallet V5 action batch; at most 255 messages. */
  readonly messages: readonly SendMessage[]
}

/** Requests one signed wallet transfer. */
export interface SendRequest {
  readonly operationId: string
  /** Allows this send to replace an unresolved signed send after explicit user confirmation. */
  readonly force?: boolean
  readonly intent: SendIntent
}

/** Durably submits one already signed external wallet message. */
export interface SendBocRequest {
  readonly operationId: string
  /** Allows replacement of an unresolved durable send after explicit confirmation. */
  readonly force?: boolean
  readonly signedBoc: string
  /** Exact wallet seqno covered by signedBoc. */
  readonly seqno: number
  /** Exact Unix expiration timestamp covered by signedBoc. */
  readonly validUntil: number
}

/** Requests one signed internal Wallet V5 message without submission. */
export interface SignMessageRequest {
  readonly operationId: string
  /** Allows replacement of an unresolved signed message after explicit confirmation. */
  readonly force?: boolean
  readonly intent: SendIntent
}

/** Requests one transfer preview without signing or submission. */
export interface SendPreviewRequest {
  readonly intent: SendIntent
}

/** One emulated transfer and its resolved message fields. */
export interface SendPreview {
  readonly messages: readonly SendMessage[]
  /** Resolved Unix expiration timestamp used by this emulation. */
  readonly validUntil: number
  /** Complete fake-signed external-message BOC in standard padded Base64. */
  readonly messageBocBase64: string
  readonly emulation: SendEmulation
}

/** A validated sign-only request without a wallet-paid fee estimate. */
export interface SignMessagePreview {
  readonly messages: readonly SendMessage[]
  readonly validUntil: number
  readonly needsStateInit: boolean
}

/** The result of one signed wallet transfer. */
export interface SendResult {
  readonly operationId: string
  /** Normalized signed external-message hash in standard padded Base64. */
  readonly messageHash: Base64Hash
  /** Signed external-message BoC returned to TON Connect callers. */
  readonly signedBoc: string
  readonly phase: SendPhase
}

/** A durable internal signed message that the caller can deliver to a relayer. */
export interface SignMessageResult {
  readonly operationId: string
  /** Complete relaxed internal-message BOC. */
  readonly internalBoc: string
  readonly validUntil: number
  readonly phase: SendPhase
}
