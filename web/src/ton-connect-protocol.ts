import type {DeviceInfo} from "@tonconnect/protocol"

import type {TonConnectAccountInfo} from "./ton-connect-types"
import type {
  AppManifest,
  ConnectItem,
  ConnectRequest,
  ParsedConnectLink,
  PersistedSession,
  TonConnectWalletIdentity,
} from "./ton-connect-types"

export const DEFAULT_BRIDGE_URL: string = "https://connect.ton.org/bridge"
export const HTTP_TIMEOUT_MS: number = 15_000
export const BRIDGE_TTL_SECONDS: number = 300
export const RECONNECT_DELAY_MS: number = 1000
export const STORAGE_VERSION: number = 1
export const MAX_MANIFEST_BYTES: number = 1024 * 1024

const CLIENT_ID: RegExp = /^[0-9a-f]{64}$/u
const UNSIGNED_DECIMAL: RegExp = /^\d+$/u
const LEADING_ZEROES: RegExp = /^0+(?=\d)/u
const TRAILING_SLASH: RegExp = /\/$/u
const TRACE_ID: RegExp =
  /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/u

/** Parses and validates the singleton fields of a TON Connect v2 link. */
export function parseTonConnectLink(value: string): ParsedConnectLink {
  const url: URL = new URL(value)
  for (const name of ["v", "id", "r"] as const) {
    if (url.searchParams.getAll(name).length !== 1) {
      throw new Error(`TON Connect link requires exactly one ${name} parameter`)
    }
  }
  if (url.searchParams.get("v") !== "2") {
    throw new Error("Unsupported TON Connect protocol version")
  }
  const peerClientId: string = url.searchParams.get("id") ?? ""
  if (!CLIENT_ID.test(peerClientId)) {
    throw new Error("TON Connect client id is invalid")
  }
  const request = JSON.parse(url.searchParams.get("r") ?? "") as ConnectRequest
  if (
    typeof request.manifestUrl !== "string" ||
    !Array.isArray(request.items) ||
    request.items.length === 0
  ) {
    throw new Error("TON Connect request is invalid")
  }
  const traceId: string | undefined = url.searchParams.get("trace_id") ?? undefined
  if (traceId !== undefined && !isTraceId(traceId)) {
    throw new Error("TON Connect trace id is invalid")
  }
  return {peerClientId, request, traceId}
}

/** Compares arbitrary-precision unsigned decimal request identifiers. */
export function isNewerRequestId(value: string, previous?: string): boolean {
  if (!UNSIGNED_DECIMAL.test(value)) {
    return false
  }
  if (previous === undefined) {
    return true
  }
  const current: string = canonicalRequestId(value)
  const baseline: string = canonicalRequestId(previous)
  return (
    current.length > baseline.length || (current.length === baseline.length && current > baseline)
  )
}

export function canonicalRequestId(value: string): string {
  const canonical: string = value.replace(LEADING_ZEROES, "")
  return canonical || "0"
}

export function enforceConnectRequest(request: ConnectRequest, activeNetwork: string): void {
  const addresses: ConnectItem[] = request.items.filter(item => item.name === "ton_addr")
  if (addresses.length !== 1) {
    throw new Error("TON Connect request must contain exactly one ton_addr item")
  }
  const requestedNetwork: string | undefined = addresses[0]?.network
  if (requestedNetwork !== undefined && requestedNetwork !== activeNetwork) {
    throw new Error("The dApp requested a different TON network")
  }
}

export async function loadManifest(
  fetchImplementation: typeof globalThis.fetch,
  value: string,
  signal: AbortSignal,
): Promise<AppManifest> {
  const url: URL = new URL(value)
  if (url.protocol !== "https:") {
    throw new Error("TON Connect manifest must use HTTPS")
  }
  const response: Response = await fetchImplementation(url, {
    method: "GET",
    redirect: "error",
    credentials: "omit",
    cache: "no-store",
    signal: AbortSignal.any([signal, AbortSignal.timeout(HTTP_TIMEOUT_MS)]),
  })
  if (!(response.ok && response.body)) {
    throw new Error(`TON Connect manifest returned HTTP ${response.status}`)
  }
  const bytes: Uint8Array = await readBoundedBody(response.body, MAX_MANIFEST_BYTES)
  const manifest = JSON.parse(new TextDecoder().decode(bytes)) as AppManifest
  if (
    typeof manifest.name !== "string" ||
    manifest.name.length === 0 ||
    typeof manifest.url !== "string" ||
    new URL(manifest.url).protocol !== "https:" ||
    typeof manifest.iconUrl !== "string" ||
    new URL(manifest.iconUrl).protocol !== "https:"
  ) {
    throw new Error("TON Connect manifest is invalid")
  }
  return manifest
}

export function accountReply(account: TonConnectAccountInfo): unknown {
  return {
    name: "ton_addr",
    address: account.address,
    network: account.network,
    walletStateInit: account.walletStateInit,
    publicKey: bytesToHex(account.publicKey),
  }
}

export function deviceInfo(identity: TonConnectWalletIdentity): DeviceInfo {
  if (identity.appName.trim().length === 0 || identity.appVersion.trim().length === 0) {
    throw new Error("TON Connect wallet identity is invalid")
  }
  return {
    platform: "browser",
    appName: identity.appName,
    appVersion: identity.appVersion,
    maxProtocolVersion: 2,
    features: [
      {name: "SendTransaction", maxMessages: 255, extraCurrencySupported: false},
      {name: "SignMessage", maxMessages: 255, extraCurrencySupported: false},
    ],
  }
}

export function bridgeEndpoint(base: string, path: string): URL {
  const url: URL = new URL(base)
  url.pathname = `${url.pathname.replace(TRAILING_SLASH, "")}/${path}`
  url.search = ""
  url.hash = ""
  return url
}

export function normalizeBridgeUrl(value: string): string {
  const url: URL = new URL(value)
  const loopback: boolean =
    url.protocol === "http:" &&
    (url.hostname === "localhost" || url.hostname === "127.0.0.1" || url.hostname === "[::1]")
  if ((url.protocol !== "https:" && !loopback) || url.search || url.hash) {
    throw new Error("TON Connect bridge URL is invalid")
  }
  return url.toString()
}

export function parsePersistedSession(value: string): PersistedSession {
  const parsed = JSON.parse(value) as PersistedSession
  if (
    parsed.version !== STORAGE_VERSION ||
    !CLIENT_ID.test(parsed.peerClientId) ||
    !isNewerOrInitialRequestId(parsed.lastRequestId) ||
    !Number.isSafeInteger(parsed.nextWalletEventId) ||
    parsed.nextWalletEventId < 1
  ) {
    throw new Error("Persisted TON Connect session is invalid")
  }
  return parsed
}

export function isTraceId(value: string): boolean {
  return TRACE_ID.test(value)
}

export function isUnsignedDecimal(value: string): boolean {
  return UNSIGNED_DECIMAL.test(value)
}

export function requireValue<T>(value: T | undefined, name: string): T {
  if (value === undefined) {
    throw new Error(`TON Connect ${name} is unavailable`)
  }
  return value
}

export function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "TON Connect operation failed"
}

export function delay(milliseconds: number, signal: AbortSignal): Promise<void> {
  return new Promise(resolve => {
    if (signal.aborted) {
      resolve()
      return
    }
    const timeout: ReturnType<typeof setTimeout> = setTimeout(resolve, milliseconds)
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(timeout)
        resolve()
      },
      {once: true},
    )
  })
}

async function readBoundedBody(
  stream: ReadableStream<Uint8Array>,
  maximum: number,
): Promise<Uint8Array> {
  const reader: ReadableStreamDefaultReader<Uint8Array> = stream.getReader()
  const chunks: Uint8Array[] = []
  let length: number = 0
  try {
    while (true) {
      const chunk = await reader.read()
      if (chunk.done) {
        break
      }
      length += chunk.value.byteLength
      if (length > maximum) {
        throw new Error("TON Connect response body is too large")
      }
      chunks.push(chunk.value)
    }
  } finally {
    reader.releaseLock()
  }
  const result: Uint8Array = new Uint8Array(length)
  let offset: number = 0
  for (const chunk of chunks) {
    result.set(chunk, offset)
    offset += chunk.byteLength
  }
  return result
}

function isNewerOrInitialRequestId(value?: string): boolean {
  return value === undefined || UNSIGNED_DECIMAL.test(value)
}

function bytesToHex(bytes: readonly number[]): string {
  return bytes.map(byte => byte.toString(16).padStart(2, "0")).join("")
}
