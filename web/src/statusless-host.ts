import type {HttpRequest, HttpRequestId} from "./types"

export type StatuslessHostErrorKind =
  | "offline"
  | "timeout"
  | "connectionLost"
  | "policyViolation"
  | "responseTooLarge"
  | "cancelled"
  | "other"

/** A provider host backed by a relay that exposes only body or error. */
export interface WalletStatuslessHost {
  executeStatusless: (request: HttpRequest) => Promise<Uint8Array | number[]>
  cancelStatusless: (requestId: HttpRequestId) => Promise<void>
}
