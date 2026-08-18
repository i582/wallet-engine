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
  /** Destination-contract StateInit encoded as a Base64 BOC. */
  readonly stateInit?: string
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
  readonly message: SendMessage
}

/** Requests one signed wallet transfer. */
export interface SendRequest {
  readonly operationId: string
  readonly intent: SendIntent
}

/** Requests one transfer preview without signing or submission. */
export interface SendPreviewRequest {
  readonly intent: SendIntent
}

/** One emulated transfer and its resolved message fields. */
export interface SendPreview {
  readonly message: SendMessage
  /** Resolved Unix expiration timestamp used by this emulation. */
  readonly validUntil: number
  /** Complete fake-signed external-message BOC in standard padded Base64. */
  readonly messageBocBase64: string
  readonly emulation: SendEmulation
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
