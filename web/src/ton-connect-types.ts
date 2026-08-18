import type {KeyPair} from "@tonconnect/protocol"

import type {SendPreview} from "./send-types"
import type {WalletDescriptor} from "./types"
import type {WalletClient} from "./wallet-client"
import type {WalletLifecycle} from "./wallet-lifecycle"

export interface TonConnectAccountInfo {
  readonly address: string
  readonly network: string
  readonly walletStateInit: string
  readonly publicKey: number[]
}

export interface TonConnectProofSignRequest {
  readonly descriptor: WalletDescriptor
  readonly domain: string
  readonly timestamp: number
  readonly payload: string
}

export interface TonConnectProofSignature {
  readonly signature: number[]
}

export interface ConnectItem {
  readonly name: string
  readonly network?: string
  readonly payload?: string
  readonly [key: string]: unknown
}

export interface ConnectRequest {
  readonly manifestUrl: string
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

export interface ParsedConnectLink {
  readonly peerClientId: string
  readonly request: ConnectRequest
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
  readonly resolve: (approved: boolean) => void
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

/** Storage used for resumable encrypted TON Connect sessions. */
export interface TonConnectStorage {
  readonly load: (key: string) => Promise<string | undefined>
  readonly save: (key: string, value: string) => Promise<void>
  readonly remove: (key: string) => Promise<void>
}

/** Browser transport and wallet dependencies. */
export interface TonConnectWalletOptions {
  readonly descriptor: WalletDescriptor
  readonly walletClient: WalletClient
  readonly lifecycle: WalletLifecycle
  readonly bridgeUrl?: string
  readonly fetch?: typeof globalThis.fetch
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
