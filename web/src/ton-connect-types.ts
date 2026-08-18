import type {KeyPair} from "@tonconnect/protocol"

import type {SendPreview} from "./send-types"
import type {WalletDescriptor} from "./types"
import type {WalletClient} from "./wallet-client"
import type {WalletLifecycle} from "./wallet-lifecycle"

/** Public account material returned to a dApp for `ton_addr`. */
export interface TonConnectAccountInfo {
  /** Canonical raw TON address. */
  readonly address: string
  /** TON network global ID encoded as a decimal string. */
  readonly network: string
  /** Wallet StateInit encoded as a standard Base64 BoC. */
  readonly walletStateInit: string
  /** Exact 32-byte Ed25519 public key. */
  readonly publicKey: number[]
}

/** Requests one address-bound TON Connect ownership proof. */
export interface TonConnectProofSignRequest {
  /** Wallet whose protected key signs the proof. */
  readonly descriptor: WalletDescriptor
  /** Exact manifest domain that the user approved. */
  readonly domain: string
  /** Unix signing timestamp in seconds. */
  readonly timestamp: number
  /** Exact dApp challenge from the connect request. */
  readonly payload: string
}

/** Signed TON Connect ownership proof. */
export interface TonConnectProofSignature {
  /** Exact 64-byte Ed25519 signature. */
  readonly signature: number[]
}

/** One capability requested during TON Connect connection. */
export interface ConnectItem {
  /** Protocol item name, such as `ton_addr` or `ton_proof`. */
  readonly name: string
  /** Optional TON network global ID for `ton_addr`. */
  readonly network?: string
  /** Optional dApp challenge for `ton_proof`. */
  readonly payload?: string
  readonly [key: string]: unknown
}

/** Initial request encoded in a TON Connect link. */
export interface ConnectRequest {
  /** HTTPS URL of the dApp manifest. */
  readonly manifestUrl: string
  /** Non-empty list of requested connection items. */
  readonly items: ConnectItem[]
}

export interface AppRequest {
  readonly method: string
  readonly params: string[]
  readonly id: string
}

export interface BridgeMessage {
  readonly from: string
  readonly message: string
  readonly trace_id?: string
}

export interface AppManifest {
  readonly url: string
  readonly name: string
  readonly iconUrl: string
  readonly termsOfUseUrl?: string
  readonly privacyPolicyUrl?: string
}

/** Validated fields from a complete TON Connect v2 link. */
export interface ParsedConnectLink {
  /** dApp bridge client identifier. */
  readonly peerClientId: string
  /** Initial connection request. */
  readonly request: ConnectRequest
  /** Optional request trace identifier. */
  readonly traceId?: string
}

export interface PendingBridgePost {
  readonly payload: unknown
  readonly topic?: string
  readonly traceId?: string
}

export interface PersistedSession {
  readonly version: number
  readonly keyPair: KeyPair
  readonly peerClientId: string
  readonly bridgeUrl: string
  readonly lastRequestId?: string
  readonly lastEventId?: string
  readonly nextWalletEventId: number
  readonly manifest: AppManifest
  readonly account: TonConnectAccountInfo
  readonly pendingPost?: PendingBridgePost
}

export interface PendingApproval {
  readonly resolve: (decision: TonConnectApprovalDecision) => void
}

/** Explicit wallet-user decision for one connection or transaction prompt. */
export interface TonConnectApprovalDecision {
  /** Whether the user approved the prompt. */
  readonly approved: boolean
  /** Whether an approved transaction may replace an unresolved signed send. */
  readonly force: boolean
}

export interface SseEvent {
  readonly id?: string
  readonly event: string
  readonly data: string
}

export interface RawMessage {
  readonly address: string
  readonly amount: string
  readonly payload?: string
  readonly stateInit?: string
  readonly extra_currency?: Record<string, string>
}

export interface RawTransactionPayload {
  readonly valid_until?: number
  readonly network?: string
  readonly from?: string
  readonly messages?: RawMessage[]
  readonly items?: unknown[]
}

/** Storage used for resumable TON Connect sessions. */
export interface TonConnectStorage {
  /** Loads one session value. */
  readonly load: (key: string) => Promise<string | undefined>
  /** Atomically replaces one complete session value. */
  readonly save: (key: string, value: string) => Promise<void>
  /** Removes one session value. */
  readonly remove: (key: string) => Promise<void>
}

/** Wallet identity advertised to a dApp in the TON Connect device record. */
export interface TonConnectWalletIdentity {
  /** Wallet registry identifier. */
  readonly appName: string
  /** Wallet application version. */
  readonly appVersion: string
}

/** Browser transport and wallet dependencies. */
export interface TonConnectWalletOptions {
  /** Wallet exposed to the connected dApp. */
  readonly descriptor: WalletDescriptor
  /** Client used to preview, sign, journal, and submit transactions. */
  readonly walletClient: WalletClient
  /** Lifecycle service used for account data and `ton_proof`. */
  readonly lifecycle: WalletLifecycle
  /** Wallet identity advertised in the TON Connect device record. */
  readonly identity: TonConnectWalletIdentity
  /** Wallet-owned bridge base URL. Defaults to the public TON Connect bridge. */
  readonly bridgeUrl?: string
  /** Fetch implementation used for manifest and bridge requests. */
  readonly fetch?: typeof globalThis.fetch
  /** Protected storage for the resumable session. */
  readonly storage: TonConnectStorage
}

/** User-visible request emitted by the wallet-side TON Connect runtime. */
export type TonConnectInteraction =
  | {
      readonly kind: "connect"
      readonly id: string
      readonly dappName: string
      readonly origin: string
      readonly iconUrl: string
      readonly account: string
      readonly proofPayload?: string
    }
  | {
      readonly kind: "transaction"
      readonly id: string
      readonly dappName: string
      readonly destination: string
      readonly amountNanograms: string
      readonly deploysContract: boolean
      readonly hasPayload: boolean
      readonly preview: SendPreview
    }

/** Observable event emitted by the TON Connect wallet runtime. */
export type TonConnectWalletEvent =
  | {readonly kind: "interaction"; readonly interaction: TonConnectInteraction}
  | {readonly kind: "connected"; readonly dappName: string; readonly account: string}
  | {readonly kind: "transactionFinished"; readonly message: string}
  | {readonly kind: "disconnected"}
  | {readonly kind: "error"; readonly message: string}
