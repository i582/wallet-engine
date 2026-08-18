import {Base64, SessionCrypto, hexToByteArray} from "@tonconnect/protocol"

import {
  BRIDGE_TTL_SECONDS,
  bridgeEndpoint,
  HTTP_TIMEOUT_MS,
  isTraceId,
} from "./ton-connect-protocol"
import {SseParser} from "./ton-connect-sse"
import type {AppRequest, BridgeMessage, SseEvent} from "./ton-connect-types"

export interface ReadBridgeEventsOptions {
  readonly bridgeUrl: string
  readonly clientId: string
  readonly lastEventId?: string
  readonly fetch: typeof globalThis.fetch
  readonly signal: AbortSignal
  readonly onEvent: (event: SseEvent) => Promise<void>
}

export interface PostBridgeMessageOptions {
  readonly bridgeUrl: string
  readonly crypto: SessionCrypto
  readonly peerClientId: string
  readonly payload: unknown
  readonly topic?: string
  readonly traceId?: string
  readonly fetch: typeof globalThis.fetch
  readonly signal: AbortSignal
}

export interface DecryptedBridgeRequest {
  readonly request: AppRequest
  readonly traceId?: string
}

export async function readBridgeEvents(options: ReadBridgeEventsOptions): Promise<void> {
  const url: URL = bridgeEndpoint(options.bridgeUrl, "events")
  url.searchParams.set("client_id", options.clientId)
  url.searchParams.set("heartbeat", "message")
  if (options.lastEventId !== undefined) {
    url.searchParams.set("last_event_id", options.lastEventId)
  }
  const response: Response = await options.fetch(url, {
    method: "GET",
    headers: {accept: "text/event-stream"},
    redirect: "error",
    credentials: "omit",
    cache: "no-store",
    signal: options.signal,
  })
  if (!(response.ok && response.body)) {
    throw new Error(`TON Connect bridge returned HTTP ${response.status}`)
  }
  const parser: SseParser = new SseParser()
  const reader: ReadableStreamDefaultReader<Uint8Array> = response.body.getReader()
  try {
    while (!options.signal.aborted) {
      const chunk = await reader.read()
      if (chunk.done) {
        return
      }
      for (const event of parser.push(chunk.value)) {
        await options.onEvent(event)
      }
    }
  } finally {
    reader.releaseLock()
  }
}

export async function postBridgeMessage(options: PostBridgeMessageOptions): Promise<void> {
  const encrypted: Uint8Array = options.crypto.encrypt(
    JSON.stringify(options.payload),
    hexToByteArray(options.peerClientId),
  )
  const url: URL = bridgeEndpoint(options.bridgeUrl, "message")
  url.searchParams.set("client_id", options.crypto.sessionId)
  url.searchParams.set("to", options.peerClientId)
  url.searchParams.set("ttl", String(BRIDGE_TTL_SECONDS))
  if (options.topic !== undefined) {
    url.searchParams.set("topic", options.topic)
  }
  if (options.traceId !== undefined && isTraceId(options.traceId)) {
    url.searchParams.set("trace_id", options.traceId)
  }
  const response: Response = await options.fetch(url, {
    method: "POST",
    headers: {"Content-Type": "text/plain; charset=utf-8"},
    body: Base64.encode(encrypted),
    redirect: "error",
    credentials: "omit",
    cache: "no-store",
    signal: AbortSignal.any([options.signal, AbortSignal.timeout(HTTP_TIMEOUT_MS)]),
  })
  if (!response.ok) {
    throw new Error(`TON Connect bridge returned HTTP ${response.status}`)
  }
}

export function decryptBridgeRequest(
  event: SseEvent,
  crypto: SessionCrypto,
  peerClientId: string,
): DecryptedBridgeRequest | undefined {
  if (event.event === "heartbeat" || event.data === "heartbeat" || event.data.length === 0) {
    return undefined
  }
  if (event.event !== "message") {
    return undefined
  }
  let envelope: BridgeMessage
  try {
    envelope = JSON.parse(event.data) as BridgeMessage
    if (envelope.from !== peerClientId || typeof envelope.message !== "string") {
      return undefined
    }
  } catch {
    return undefined
  }
  try {
    const plaintext: string = crypto.decrypt(
      Base64.decode(envelope.message).toUint8Array(),
      hexToByteArray(peerClientId),
    )
    return {request: JSON.parse(plaintext) as AppRequest, traceId: envelope.trace_id}
  } catch {
    // The reference crypto error includes key material. Never log it.
    return undefined
  }
}
