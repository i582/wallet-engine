import {
  WalletClient,
  type BrowserPlatformHost,
  type HostFailure,
  type HttpRequest,
  type HttpRequestId,
  type StatuslessHostErrorKind,
  type WalletClientConfig,
  type WalletStatuslessHost,
} from "@ton/wallet-engine"

const MAX_RESPONSE_BODY_BYTES = 4 * 1024 * 1024
const MAX_EARLY_CANCELLATIONS = 1024

/** The application-specific physical provider transport. */
export interface ProviderRelay {
  execute: (request: HttpRequest, signal: AbortSignal) => Promise<Uint8Array | readonly number[]>
  cancel: (requestId: number) => Promise<void>
}

/**
 * Runnable body-only relay backed by fetch.
 *
 * It intentionally discards HTTP status, headers, and final URL. Replace this
 * class with the application's actual relay while keeping ProviderRelay.
 */
export class BodyOnlyFetchRelay implements ProviderRelay {
  private readonly allowedOrigin: string

  constructor(providerBaseUrl: string) {
    this.allowedOrigin = new URL(providerBaseUrl).origin
  }

  async execute(request: HttpRequest, signal: AbortSignal): Promise<Uint8Array> {
    const url = new URL(request.url)
    if (url.protocol !== "https:" || url.origin !== this.allowedOrigin) {
      throw failure("policyViolation", "Provider destination is not allowed")
    }

    const headers = new Headers()
    for (const header of request.headers) {
      headers.append(header.name, header.value)
    }

    try {
      const response = await fetch(url, {
        method: request.method === "get" ? "GET" : "POST",
        headers,
        body: request.method === "post" ? new Uint8Array(request.body) : undefined,
        redirect: "error",
        credentials: "omit",
        signal,
      })
      return new Uint8Array(await response.arrayBuffer())
    } catch (cause) {
      if (signal.aborted) {
        throw failure("cancelled", "Provider request was cancelled")
      }
      if (isHostFailure(cause)) {
        throw cause
      }
      throw failure("connectionLost", "Provider relay request failed")
    }
  }

  cancel(_requestId: number): Promise<void> {
    // RelayProviderHost already aborts the signal before calling this method.
    return Promise.resolve()
  }
}

/** Creates a client whose provider returns only body or a host failure. */
export async function createRelayClient(
  config: WalletClientConfig,
  platformHost: BrowserPlatformHost,
  relay: ProviderRelay,
): Promise<WalletClient> {
  return WalletClient.createStatusless(config, {
    platformHost,
    statuslessHost: new RelayProviderHost(relay),
  })
}

export class RelayProviderHost implements WalletStatuslessHost {
  private readonly relay: ProviderRelay
  private readonly inFlight = new Map<number, AbortController>()
  private readonly cancelledBeforeStart = new Set<number>()
  private readonly cancelledOrder: number[] = []

  constructor(relay: ProviderRelay) {
    this.relay = relay
  }

  async executeStatusless(request: HttpRequest): Promise<Uint8Array> {
    const id = request.id.value
    if (this.cancelledBeforeStart.delete(id)) {
      throw failure("cancelled", "Provider request was cancelled before it started")
    }

    const controller = new AbortController()
    let timedOut = false
    let timeoutHandle: ReturnType<typeof setTimeout> | undefined
    const timeout = new Promise<never>((_resolve, reject) => {
      timeoutHandle = setTimeout(() => {
        timedOut = true
        controller.abort()
        reject(failure("timeout", "Provider relay timed out"))
      }, request.timeoutMs)
    })

    this.inFlight.set(id, controller)
    try {
      // Convert request.url, method, headers, and body to the relay's wire
      // format inside relay.execute. There is no direct-origin claim here.
      const body = await Promise.race([this.relay.execute(request, controller.signal), timeout])
      const bytes = Uint8Array.from(body)
      if (bytes.byteLength > MAX_RESPONSE_BODY_BYTES) {
        throw failure("responseTooLarge", "Provider response is too large")
      }
      return bytes
    } catch (cause) {
      if (timedOut) {
        throw failure("timeout", "Provider relay timed out")
      }
      if (isHostFailure(cause)) {
        throw failure(cause.kind, cause.diagnostic)
      }
      if (controller.signal.aborted) {
        throw failure("cancelled", "Provider request was cancelled")
      }
      // Use a bounded, non-sensitive diagnostic. Do not forward arbitrary
      // exception text when it can contain request data or credentials.
      throw failure("other", "Provider relay failed")
    } finally {
      if (timeoutHandle !== undefined) {
        clearTimeout(timeoutHandle)
      }
      this.inFlight.delete(id)
    }
  }

  async cancelStatusless(requestId: HttpRequestId): Promise<void> {
    const id = requestId.value
    const controller = this.inFlight.get(id)
    if (controller) {
      controller.abort()
    } else if (!this.cancelledBeforeStart.has(id)) {
      this.cancelledBeforeStart.add(id)
      this.cancelledOrder.push(id)
    }

    while (this.cancelledOrder.length > MAX_EARLY_CANCELLATIONS) {
      const expired = this.cancelledOrder.shift()
      if (expired !== undefined) {
        this.cancelledBeforeStart.delete(expired)
      }
    }
    await this.relay.cancel(id)
  }
}

function failure(kind: StatuslessHostErrorKind, diagnostic: string): Error & HostFailure {
  return Object.assign(new Error(diagnostic), {kind, diagnostic})
}

function isHostFailure(value: unknown): value is HostFailure & {kind: StatuslessHostErrorKind} {
  return (
    value instanceof Error &&
    "kind" in value &&
    isStatuslessHostErrorKind(value.kind) &&
    "diagnostic" in value &&
    typeof value.diagnostic === "string"
  )
}

function isStatuslessHostErrorKind(value: unknown): value is StatuslessHostErrorKind {
  return (
    value === "offline" ||
    value === "timeout" ||
    value === "connectionLost" ||
    value === "policyViolation" ||
    value === "responseTooLarge" ||
    value === "cancelled" ||
    value === "other"
  )
}
