import type {
  CredentialRef,
  HostFailure,
  HttpCall,
  HttpCallId,
  HttpHeader,
  HttpResponse,
} from "./types"

const MAX_REQUEST_BODY_BYTES = 256 * 1024
const MAX_RESPONSE_BODY_BYTES = 4 * 1024 * 1024
const MAX_RESPONSE_HEADER_BYTES = 64 * 1024
const MAX_RESPONSE_HEADERS = 64
const MAX_EARLY_CANCELLATIONS = 1024

export type CredentialProvider = (
  reference: CredentialRef,
) => Promise<string | undefined> | string | undefined

export interface BrowserHttpHostOptions {
  readonly credentialProvider?: CredentialProvider
  readonly fetch?: typeof globalThis.fetch
}

export class BrowserHttpHost {
  private readonly credentialProvider: CredentialProvider
  private readonly fetch: typeof globalThis.fetch
  private readonly tasks: Map<number, AbortController> = new Map()
  private readonly cancelledBeforeStart: Set<number> = new Set()
  private readonly cancelledOrder: number[] = []

  constructor(options: BrowserHttpHostOptions = {}) {
    this.credentialProvider = options.credentialProvider ?? (() => undefined)
    this.fetch = options.fetch ?? globalThis.fetch.bind(globalThis)
  }

  async executeHttp(call: HttpCall): Promise<HttpResponse> {
    this.validateCall(call)
    const callId = call.id.value
    if (this.cancelledBeforeStart.delete(callId)) {
      throw hostFailure("cancelled", "The HTTP call was cancelled before it started")
    }

    const controller = new AbortController()
    this.tasks.set(callId, controller)
    let credential: string | undefined
    try {
      const url = new URL(call.url)
      const headers = new Headers()
      for (const header of call.headers) {
        if (isReservedHeader(header.name)) {
          throw hostFailure("policyViolation", "The request contains a reserved header")
        }
        headers.append(header.name, header.value)
      }

      if (call.credential !== undefined) {
        credential = await this.credentialProvider(call.credential)
        if (!credential) {
          throw hostFailure("policyViolation", "The requested credential is unavailable")
        }
        if (effectiveOrigin(url) !== call.credentialOrigin) {
          throw hostFailure("policyViolation", "The credential origin does not match the request")
        }
        headers.set("X-API-Key", credential)
      }

      const response = await this.fetch(url, {
        method: call.method === "get" ? "GET" : "POST",
        headers,
        body: call.method === "post" ? new Uint8Array(call.body) : undefined,
        redirect: "error",
        credentials: "omit",
        cache: "no-store",
        signal: controller.signal,
      })
      const responseHeaders = collectHeaders(response.headers, credential)
      enforceHeaderLimit(responseHeaders, call.maxResponseHeaderBytes)
      const body = await collectBody(response, call.maxResponseBodyBytes, controller)
      return {
        status: response.status,
        headers: responseHeaders,
        body: [...body],
        finalUrl: response.url || call.url,
      }
    } catch (error) {
      throw normalizeHttpFailure(error, controller.signal.aborted)
    } finally {
      this.tasks.delete(callId)
    }
  }

  async cancelHttp(callId: HttpCallId): Promise<void> {
    const task = this.tasks.get(callId.value)
    if (task) {
      this.tasks.delete(callId.value)
      task.abort()
      return
    }
    if (!this.cancelledBeforeStart.has(callId.value)) {
      this.cancelledBeforeStart.add(callId.value)
      this.cancelledOrder.push(callId.value)
    }
    while (this.cancelledOrder.length > MAX_EARLY_CANCELLATIONS) {
      const expired = this.cancelledOrder.shift()
      if (expired !== undefined) {
        this.cancelledBeforeStart.delete(expired)
      }
    }
  }

  private validateCall(call: HttpCall): void {
    const url = new URL(call.url)
    if (url.protocol !== "https:") {
      throw hostFailure("policyViolation", "Only HTTPS requests are permitted")
    }
    if (call.body.length > MAX_REQUEST_BODY_BYTES) {
      throw hostFailure("policyViolation", "The request body is too large")
    }
    if (
      call.maxResponseBodyBytes <= 0 ||
      call.maxResponseBodyBytes > MAX_RESPONSE_BODY_BYTES ||
      call.maxResponseHeaderBytes <= 0 ||
      call.maxResponseHeaderBytes > MAX_RESPONSE_HEADER_BYTES
    ) {
      throw hostFailure("policyViolation", "The response limit is invalid")
    }
    if ((call.credential === undefined) !== (call.credentialOrigin === undefined)) {
      throw hostFailure("policyViolation", "The credential policy is incomplete")
    }
  }
}

function effectiveOrigin(url: URL): string {
  const port = url.port || (url.protocol === "https:" ? "443" : "80")
  return `${url.protocol}//${url.hostname.toLowerCase()}:${port}`
}

function isReservedHeader(name: string): boolean {
  const lower = name.toLowerCase()
  return lower === "x-api-key" || lower === "authorization" || lower === "cookie"
}

function collectHeaders(headers: Headers, credential?: string): HttpHeader[] {
  const result: HttpHeader[] = []
  for (const [name, value] of headers) {
    if (isReservedHeader(name) || (credential !== undefined && value === credential)) {
      continue
    }
    result.push({name, value})
  }
  if (result.length > MAX_RESPONSE_HEADERS) {
    throw hostFailure("responseTooLarge", "The response has too many headers")
  }
  return result
}

function enforceHeaderLimit(headers: HttpHeader[], maximum: number): void {
  let bytes = 0
  const encoder = new TextEncoder()
  for (const header of headers) {
    bytes += encoder.encode(header.name).byteLength
    bytes += encoder.encode(header.value).byteLength
    bytes += 4
  }
  if (bytes > maximum) {
    throw hostFailure("responseTooLarge", "The response headers are too large")
  }
}

async function collectBody(
  response: Response,
  maximum: number,
  controller: AbortController,
): Promise<Uint8Array> {
  if (!response.body) {
    return new Uint8Array()
  }
  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let length = 0
  try {
    while (true) {
      const {done, value} = await reader.read()
      if (done) {
        break
      }
      length += value.byteLength
      if (length > maximum) {
        controller.abort()
        throw hostFailure("responseTooLarge", "The response body is too large")
      }
      chunks.push(value)
    }
  } finally {
    reader.releaseLock()
  }
  const body = new Uint8Array(length)
  let offset = 0
  for (const chunk of chunks) {
    body.set(chunk, offset)
    offset += chunk.byteLength
  }
  return body
}

function hostFailure(kind: string, diagnostic: string): HostFailure {
  return {kind, diagnostic}
}

function normalizeHttpFailure(error: unknown, cancelled: boolean): HostFailure {
  if (isHostFailure(error)) {
    return error
  }
  if (cancelled || (error instanceof DOMException && error.name === "AbortError")) {
    return hostFailure("cancelled", "The HTTP call was cancelled")
  }
  if (error instanceof TypeError) {
    return hostFailure("connectionLost", "The browser fetch request failed")
  }
  return hostFailure("other", "The browser HTTP host failed")
}

function isHostFailure(value: unknown): value is HostFailure {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    typeof value.kind === "string" &&
    "diagnostic" in value &&
    typeof value.diagnostic === "string"
  )
}
