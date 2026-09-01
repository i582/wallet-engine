export type Network = "mainnet" | "testnet"

declare const base64HashBrand: unique symbol

/** A runtime-validated 256-bit hash in standard padded Base64. */
export type Base64Hash = string & {
  readonly [base64HashBrand]: "Base64Hash"
}

export interface ProviderConfig {
  readonly toncenterBaseUrl: string
  /** Optional TON DNS root override; omitted uses the current network default. */
  readonly dnsRootAddress?: string | null
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
  /** Total transaction fee; repeated when one transaction has multiple activity rows. */
  readonly transactionFeeNanograms: string
  readonly status: "success" | "failed" | "bounced"
  readonly comment?: string
  /** Complete Base64 BOC for an explicitly decryptable TON encrypted comment. */
  readonly encryptedComment?: string
  readonly counterparty?: string
}

/** Builds an encrypted-comment BOC after provider lookup and key authorization. */
export interface CreateEncryptedCommentRequest {
  readonly recipient: string
  readonly comment: string
}

/** Decrypts an encrypted-comment BOC from the specified sender. */
export interface DecryptCommentRequest {
  readonly sender: string
  readonly body: string
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
  | "handedOff"
  | "confirmed"
  | "replaced"
  | "sequenceNumberConsumed"
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

export interface SendEmulationTransaction {
  readonly account: string
  readonly succeeded: boolean
  readonly isRoot: boolean
}

export interface SendEmulation {
  readonly mcBlockSeqno: number
  readonly walletFeesNanograms: string
  readonly traceFeesNanograms: string
  readonly transactionCount: number
  readonly actions: SendEmulationAction[]
  readonly transactions: SendEmulationTransaction[]
  readonly traceSucceeded: boolean
  readonly isIncomplete: boolean
}

export interface ActivityList {
  readonly items: ActivityItem[]
  readonly resource: ResourceState
  readonly paginationResource: ResourceState
  readonly hasMore: boolean
}

export interface NftCollectionDescriptor {
  readonly address: string
  readonly name?: string
  readonly description?: string
  readonly image?: string
  readonly content: Readonly<Record<string, string>>
}

export interface NftItem {
  readonly address: string
  readonly collectionAddress?: string
  readonly collection?: NftCollectionDescriptor
  readonly ownerAddress?: string
  readonly realOwner?: string
  readonly saleContractAddress?: string
  readonly auctionContractAddress?: string
  readonly index: string
  readonly lastTransactionLt: string
  readonly initialized: boolean
  readonly onSale: boolean
  readonly codeHash: string
  readonly dataHash: string
  readonly content: Readonly<Record<string, string>>
  readonly isNsfw?: boolean
  readonly isScam?: boolean
}

export interface NftList {
  readonly items: NftItem[]
  readonly resource: ResourceState
  readonly paginationResource: ResourceState
  readonly hasMore: boolean
}

export interface WalletSnapshot {
  readonly revision: number
  readonly recordId: string
  readonly address: string
  readonly network: Network
  readonly account?: AccountSnapshot
  readonly accountResource: ResourceState
  readonly activity: ActivityList
  readonly activityCursor?: ActivityCursor
  readonly nfts: NftList
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
  readonly nftItemsAdded: number
  readonly snapshot: WalletSnapshot
}

export interface ProtectedSecretRef {
  readonly value: string
}

export interface ProtectedSecretRead {
  readonly secretRef: ProtectedSecretRef
  readonly reason:
    | "createWallet"
    | "signTransfer"
    | "signTonConnectProof"
    | "prepareKeyRotation"
    | "encryptComment"
    | "decryptComment"
    | "revealRecoveryPhrase"
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

export type KeyRotationMessageKind = "external" | "internal"

export interface PrepareKeyRotationRequest {
  readonly validUntil: number
  readonly messageKind: KeyRotationMessageKind
}

export interface PreparedKeyRotation {
  readonly replacementRecoveryPhrase: RecoveryPhrase
  readonly newPublicKey: number[]
  /** Complete external or relaxed internal signed message in Base64 BOC form. */
  readonly signedBoc: string
  readonly seqno: number
  readonly validUntil: number
  readonly messageKind: KeyRotationMessageKind
}

export interface HostFailure {
  readonly kind: string
  readonly diagnostic: string
}
