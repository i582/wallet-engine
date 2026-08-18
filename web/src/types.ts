export type Network = "mainnet" | "testnet"

declare const base64HashBrand: unique symbol

/** A runtime-validated 256-bit hash in standard padded Base64. */
export type Base64Hash = string & {
  readonly [base64HashBrand]: "Base64Hash"
}

export interface ProviderConfig {
  readonly toncenterBaseUrl: string
  /** End-to-end timeout for each provider request, in milliseconds. */
  readonly requestTimeoutMs: number
}

export interface WalletClientConfig {
  readonly recordId: string
  readonly address: string
  readonly publicKey: number[]
  /** Protected mnemonic reference for local signing; omit for a public-key-only wallet. */
  readonly localSecretRef?: ProtectedSecretRef
  readonly network: Network
  readonly sendValiditySeconds: number
  readonly resolutionMarginSeconds: number
  readonly providers: ProviderConfig
}

export interface HttpRequestId {
  readonly value: number
}

export interface HttpHeader {
  readonly name: string
  readonly value: string
}

export interface HttpRequest {
  readonly id: HttpRequestId
  readonly method: "get" | "post"
  readonly url: string
  readonly headers: HttpHeader[]
  readonly body: number[]
  readonly timeoutMs: number
}

export interface HttpResponse {
  readonly status: number
  readonly headers: HttpHeader[]
  readonly body: number[]
  readonly finalUrl: string
}

export type AccountStatus = "nonexistent" | "uninitialized" | "active" | "frozen" | "unknown"

export interface AccountSnapshot {
  /** Exact unsigned balance in nanograms, encoded as a base-10 integer string. */
  readonly balanceNanograms: string
  readonly status: AccountStatus
  readonly syncUtime: number
}

export interface ActivityItem {
  readonly id: string
  /** Transaction hash in standard padded Base64. */
  readonly transactionHash: Base64Hash
  readonly logicalTime: string
  readonly timestamp: number
  readonly direction: "sent" | "received"
  readonly amountNanograms: string
  readonly counterparty?: string
}

export interface ActivityCursor {
  readonly logicalTime: string
  /** Oldest loaded transaction hash in standard padded Base64. */
  readonly hash: Base64Hash
}

export interface DomainError {
  readonly code: string
  readonly category: string
  readonly retry: "none" | "safe" | "afterDelay"
  readonly developerMessage: string
  readonly providerStatus?: number
  readonly retryAfterMs?: number
  readonly hostKind?: string
}

export interface ResourceState {
  readonly phase: "idle" | "loading" | "ready" | "failed"
  readonly error?: DomainError
}

export type SendPhase =
  | "idle"
  | "validating"
  | "authorizing"
  | "preparing"
  | "persisting"
  | "readyToSubmit"
  | "submitting"
  | "submissionUnknown"
  | "submitted"
  | "confirmed"
  | "replaced"
  | "expired"
  | "superseded"
  | "failed"
  | "cancelled"

export type PendingReason = "inMempool" | "awaitingWindow"

export interface ResolutionInfo {
  readonly transactionHash?: Base64Hash
  readonly transactionLt?: string
  readonly pendingReason?: PendingReason
  readonly canForceRetry: boolean
  readonly retryAfterHintMs?: number
}

export interface SendSnapshot {
  readonly operationId?: string
  readonly phase: SendPhase
  readonly errorMessage?: string
  readonly resolution?: ResolutionInfo
}

export interface SendEmulationAction {
  /** Validated action identifier in standard padded Base64. */
  readonly actionId: Base64Hash
  readonly kind: string
  readonly succeeded: boolean
  readonly accounts: string[]
  /** Validated transaction hashes in standard padded Base64. */
  readonly transactionHashes: Base64Hash[]
  readonly detailsJson: string
}

export interface SendEmulation {
  readonly mcBlockSeqno: number
  readonly walletFeesNanograms: string
  readonly traceFeesNanograms: string
  readonly transactionCount: number
  readonly actions: SendEmulationAction[]
  readonly traceSucceeded: boolean
  readonly isIncomplete: boolean
}

export interface WalletSnapshot {
  readonly revision: number
  readonly recordId: string
  readonly address: string
  readonly network: Network
  readonly account?: AccountSnapshot
  readonly accountResource: ResourceState
  readonly activity: ActivityItem[]
  readonly activityResource: ResourceState
  readonly activityPaginationResource: ResourceState
  readonly activityCursor?: ActivityCursor
  readonly activityHasMore: boolean
  readonly send: SendSnapshot
}

export interface WalletUpdate {
  readonly outcome:
    | "completed"
    | "partiallyCompleted"
    | "failed"
    | "cancelled"
    | "superseded"
    | "skipped"
  readonly activityItemsAdded: number
  readonly snapshot: WalletSnapshot
}

export interface ProtectedSecretRef {
  readonly value: string
}

export interface ProtectedSecretRead {
  readonly secretRef: ProtectedSecretRef
  readonly reason: "createWallet" | "signTransfer" | "revealRecoveryPhrase"
  readonly prompt: string
}

export interface ProtectedSecretStore {
  readonly secretRef: ProtectedSecretRef
  readonly bytes: number[]
  readonly requireUserPresence: boolean
}

export interface JournalKey {
  readonly recordId: string
  readonly slot: string
}

export interface JournalRecord {
  readonly version: number
  readonly payload: number[]
}

export interface JournalCompareExchange {
  readonly key: JournalKey
  readonly expectedVersion?: number
  readonly replacement: JournalRecord
}

export interface JournalCompareExchangeResult {
  readonly applied: boolean
  readonly current?: JournalRecord
}

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

export interface SendRequest {
  readonly operationId: string
  readonly intent: SendIntent
}

export interface SendPreviewRequest {
  readonly intent: SendIntent
}

export interface SendPreview {
  readonly message: SendMessage
  /** Resolved Unix expiration timestamp used by this emulation. */
  readonly validUntil: number
  /** Complete fake-signed external-message BOC in standard padded Base64. */
  readonly messageBocBase64: string
  readonly emulation: SendEmulation
}

export interface SendResult {
  readonly operationId: string
  /** Normalized signed external-message hash in standard padded Base64. */
  readonly messageHash: Base64Hash
  /** Signed external-message BoC returned to TON Connect callers. */
  readonly signedBoc: string
  readonly phase: SendPhase
}

export interface WalletDescriptor {
  readonly recordId: string
  readonly address: string
  readonly publicKey: number[]
  readonly network: Network
  readonly secretRef: ProtectedSecretRef
}

export interface CreateWalletRequest {
  readonly recordId: string
  readonly network: Network
}

export interface ImportWalletRequest extends CreateWalletRequest {
  readonly recoveryWords: string[]
}

export interface RecoveryPhrase {
  readonly phrase: string
}

export interface CreatedWallet {
  readonly descriptor: WalletDescriptor
  readonly recoveryPhrase: RecoveryPhrase
}

export interface HostFailure {
  readonly kind: string
  readonly diagnostic: string
}
