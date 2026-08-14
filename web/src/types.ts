export type Network = "mainnet" | "testnet"

export interface CredentialRef {
  readonly value: string
}

export interface ProviderConfig {
  readonly toncenterBaseUrl: string
  readonly toncenterCredential?: CredentialRef
  readonly toncenterCredentialOrigin?: string
}

export interface WalletClientConfig {
  readonly walletId: string
  readonly address: string
  readonly network: Network
  readonly providers: ProviderConfig
}

export interface HttpCallId {
  readonly value: number
}

export interface HttpHeader {
  readonly name: string
  readonly value: string
}

export interface HttpCall {
  readonly id: HttpCallId
  readonly method: "get" | "post"
  readonly url: string
  readonly headers: HttpHeader[]
  readonly body: number[]
  readonly credential?: CredentialRef
  readonly credentialOrigin?: string
  readonly maxResponseHeaderBytes: number
  readonly maxResponseBodyBytes: number
}

export interface HttpResponse {
  readonly status: number
  readonly headers: HttpHeader[]
  readonly body: number[]
  readonly finalUrl: string
}

export type AccountStatus = "nonexistent" | "uninitialized" | "active" | "frozen" | "unknown"

export interface AccountSnapshot {
  readonly balanceNanograms: string
  readonly balanceGrams: string
  readonly status: AccountStatus
  readonly syncUtime?: number
}

export interface ActivityItem {
  readonly id: string
  readonly transactionHash: string
  readonly logicalTime: string
  readonly timestamp: number
  readonly direction: "sent" | "received"
  readonly amountNanograms: string
  readonly amountGrams: string
  readonly counterparty?: string
}

export interface ActivityCursor {
  readonly logicalTime: string
  readonly hash: string
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
  | "failed"
  | "cancelled"

export interface SendSnapshot {
  readonly operationId?: string
  readonly phase: SendPhase
  readonly errorMessage?: string
}

export interface WalletSnapshot {
  readonly revision: number
  readonly walletId: string
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
  readonly walletId: string
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

export interface SendRequest {
  readonly operationId: string
  readonly destination: string
  readonly amountNanograms: string
  readonly secretRef: ProtectedSecretRef
}

export interface SendResult {
  readonly operationId: string
  readonly messageHash: string
  readonly phase: SendPhase
}

export interface WalletDescriptor {
  readonly walletId: string
  readonly address: string
  readonly network: Network
  readonly secretRef: ProtectedSecretRef
}

export interface CreateWalletRequest {
  readonly walletId: string
  readonly network: Network
}

export interface ImportWalletRequest extends CreateWalletRequest {
  readonly recoveryWords: string[]
}

export interface RecoveryPhrase {
  readonly words: string[]
}

export interface CreatedWallet {
  readonly descriptor: WalletDescriptor
  readonly recoveryPhrase: RecoveryPhrase
}

export interface HostFailure {
  readonly kind: string
  readonly diagnostic: string
}
