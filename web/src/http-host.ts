import type {HostFailure, HttpRequest, HttpRequestId, HttpHeader, HttpResponse} from "./types"

const MAX_REQUEST_BODY_BYTES = 256 * 1024
const MAX_RESPONSE_BODY_BYTES = 4 * 1024 * 1024
const MAX_RESPONSE_HEADER_BYTES = 64 * 1024
const MAX_RESPONSE_HEADERS = 64
const MAX_EARLY_CANCELLATIONS = 1024
const MAX_REQUEST_TIMEOUT_MS = 5 * 60 * 1000

export interface BrowserHttpHostOptions {
  readonly toncenterApiKey?: string
  readonly fetch?: typeof globalThis.fetch
}

export class BrowserHttpHost {
  private readonly allowedOrigin: string
  private readonly toncenterApiKey: string | undefined
  private readonly fetch: typeof globalThis.fetch
  private readonly tasks: Map<number, AbortController> = new Map()
  private readonly cancelledBeforeStart: Set<number> = new Set()
  private readonly cancelledOrder: number[] = []

  constructor(toncenterBaseUrl: string, options: BrowserHttpHostOptions = {}) {
    const providerUrl: URL = new URL(toncenterBaseUrl)
    if (providerUrl.protocol !== "https:") {
      throw hostFailure("policyViolation", "The Toncenter URL must use HTTPS")
    }
    this.allowedOrigin = providerUrl.origin
    this.toncenterApiKey = options.toncenterApiKey?.trim() || undefined
    this.fetch = options.fetch ?? globalThis.fetch.bind(globalThis)
  }

  async executeHttp(request: HttpRequest): Promise<HttpResponse> {
    this.validateRequest(request)
    const requestId = request.id.value
    if (this.cancelledBeforeStart.delete(requestId)) {
      throw hostFailure("cancelled", "The HTTP request was cancelled before it started")
    }

    const controller = new AbortController()
    let timedOut = false
    let timeoutHandle: ReturnType<typeof setTimeout> | undefined
    const timeout = new Promise<never>((_resolve, reject) => {
      timeoutHandle = setTimeout(() => {
        timedOut = true
        controller.abort()
        reject(hostFailure("timeout", "The HTTP request timed out"))
      }, request.timeoutMs)
    })
    this.tasks.set(requestId, controller)
    try {
      return await this.perform(request, controller, timeout)
    } catch (error) {
      if (timedOut) {
        throw hostFailure("timeout", "The HTTP request timed out")
      }
      throw normalizeHttpFailure(error, controller.signal.aborted)
    } finally {
      if (timeoutHandle !== undefined) {
        clearTimeout(timeoutHandle)
      }
      this.tasks.delete(requestId)
    }
  }

  async cancelHttp(requestId: HttpRequestId): Promise<void> {
    const task = this.tasks.get(requestId.value)
    if (task) {
      this.tasks.delete(requestId.value)
      task.abort()
      return
    }
    if (!this.cancelledBeforeStart.has(requestId.value)) {
      this.cancelledBeforeStart.add(requestId.value)
      this.cancelledOrder.push(requestId.value)
    }
    while (this.cancelledOrder.length > MAX_EARLY_CANCELLATIONS) {
      const expired = this.cancelledOrder.shift()
      if (expired !== undefined) {
        this.cancelledBeforeStart.delete(expired)
      }
    }
  }

  private async perform(
    request: HttpRequest,
    controller: AbortController,
    timeout: Promise<never>,
  ): Promise<HttpResponse> {
    const url = new URL(request.url)
    const headers = new Headers()
    for (const header of request.headers) {
      if (isReservedHeader(header.name)) {
        throw hostFailure("policyViolation", "The request contains a reserved header")
      }
      headers.append(header.name, header.value)
    }

    if (this.toncenterApiKey !== undefined) {
      headers.set("X-API-Key", this.toncenterApiKey)
    }

    const response = await Promise.race([
      this.fetch(url, {
        method: request.method === "get" ? "GET" : "POST",
        headers,
        body: request.method === "post" ? new Uint8Array(request.body) : undefined,
        redirect: "error",
        credentials: "omit",
        cache: "no-store",
        signal: controller.signal,
      }),
      timeout,
    ])
    const responseHeaders = collectHeaders(response.headers, this.toncenterApiKey)
    enforceHeaderLimit(responseHeaders, request.maxResponseHeaderBytes)
    const body = await Promise.race([
      collectBody(response, request.maxResponseBodyBytes, controller),
      timeout,
    ])
    return {
      status: response.status,
      headers: responseHeaders,
      body: [...body],
      finalUrl: response.url || request.url,
    }
  }

  private validateRequest(request: HttpRequest): void {
    const url = new URL(request.url)
    if (url.protocol !== "https:") {
      throw hostFailure("policyViolation", "Only HTTPS requests are permitted")
    }
    if (url.origin !== this.allowedOrigin) {
      throw hostFailure("policyViolation", "The request origin does not match Toncenter")
    }
    if (request.body.length > MAX_REQUEST_BODY_BYTES) {
      throw hostFailure("policyViolation", "The request body is too large")
    }
    if (
      !Number.isSafeInteger(request.timeoutMs) ||
      request.timeoutMs <= 0 ||
      request.timeoutMs > MAX_REQUEST_TIMEOUT_MS
    ) {
      throw hostFailure("policyViolation", "The request timeout is invalid")
    }
    if (
      request.maxResponseBodyBytes <= 0 ||
      request.maxResponseBodyBytes > MAX_RESPONSE_BODY_BYTES ||
      request.maxResponseHeaderBytes <= 0 ||
      request.maxResponseHeaderBytes > MAX_RESPONSE_HEADER_BYTES
    ) {
      throw hostFailure("policyViolation", "The response limit is invalid")
    }
  }
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
    return hostFailure("cancelled", "The HTTP request was cancelled")
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
