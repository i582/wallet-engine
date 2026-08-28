import {readFileSync} from "node:fs"
import {
  createServer as createHttpServer,
  type IncomingMessage,
  type ServerResponse,
} from "node:http"
import {createServer as createHttpsServer} from "node:https"
import path from "node:path"
import process from "node:process"

import {ActonLocalnet} from "./acton-localnet"

const DEFAULT_PORT: number = 5198
const WALLET_BALANCE_NANOGRAMS: string = "10000000000"
const SCRIPTED_NFT_OWNER: string =
  "0:1111111111111111111111111111111111111111111111111111111111111111"
const SCRIPTED_NFT_COLLECTION: string =
  "0:4444444444444444444444444444444444444444444444444444444444444444"
const REPOSITORY_ROOT: string = path.resolve(import.meta.dirname, "../../../..")
const TLS_FIXTURES: string = path.join(REPOSITORY_ROOT, "tests/ton-connect/dapp/fixtures")

const port: number = parsePort(process.argv, DEFAULT_PORT)
const servesInsecureHttp: boolean = process.argv.includes("--http")
const tracesRequests: boolean = process.env.WALLET_ENGINE_E2E_PROVIDER_LOG === "1"
let walletAddress: string | undefined
let providerMode: "localnet" | "scripted" = "scripted"
let localnet: ActonLocalnet | undefined
const fundingByAddress = new Map<string, Promise<void>>()

/** Dispatches one provider request and reports asynchronous localnet failures as JSON. */
function requestHandler(request: IncomingMessage, response: ServerResponse): void {
  if (tracesRequests) {
    console.error(`${request.method ?? "UNKNOWN"} ${request.url ?? "/"}`)
  }
  void route(request, response).catch(error => {
    const message: string = error instanceof Error ? error.message : String(error)
    console.error(`${request.method ?? "UNKNOWN"} ${request.url ?? "/"}: ${message}`)
    sendJson(response, 500, {error: message, ok: false})
  })
}

/** Routes health, control, rates, scripted, and real localnet provider requests. */
async function route(request: IncomingMessage, response: ServerResponse): Promise<void> {
  const requestUrl = new URL(request.url ?? "/", `http://127.0.0.1:${port}`)

  if (request.method === "OPTIONS") {
    response.writeHead(204, corsHeaders())
    response.end()
    return
  }

  if (request.method === "GET" && requestUrl.pathname === "/health") {
    sendJson(response, 200, {providerMode, status: "ok"})
    return
  }

  if (request.method === "POST" && requestUrl.pathname === "/e2e/provider") {
    const command: unknown = JSON.parse((await readBody(request)).toString("utf8"))
    if (!isProviderModeCommand(command)) {
      sendJson(response, 400, {error: "Expected scripted or localnet provider mode", ok: false})
      return
    }
    await selectProviderMode(command.mode)
    sendJson(response, 200, {mode: providerMode, ok: true})
    return
  }

  if (requestUrl.pathname === "/v2/rates") {
    sendJson(response, 200, {rates: {TON: {prices: {USD: 5}}}})
    return
  }

  if (request.method === "GET" && requestUrl.pathname === "/e2e/nft-art.svg") {
    sendBytes(
      response,
      200,
      Buffer.from(scriptedNftArtwork(requestUrl.searchParams.get("variant"))),
      "image/svg+xml; charset=utf-8",
    )
    return
  }

  if (providerMode === "localnet") {
    await proxyLocalnetRequest(request, requestUrl, response)
    return
  }

  scriptedResponse(request, requestUrl, response)
}

/** Returns the deterministic provider response used by non-localnet client scenarios. */
function scriptedResponse(
  request: IncomingMessage,
  requestUrl: URL,
  response: ServerResponse,
): void {
  if (requestUrl.pathname.endsWith("/api/v2/getAddressInformation")) {
    walletAddress = requestUrl.searchParams.get("address") ?? walletAddress
    sendJson(response, 200, {
      ok: true,
      result: {
        balance: WALLET_BALANCE_NANOGRAMS,
        state: "uninitialized",
        sync_utime: Math.floor(Date.now() / 1000),
      },
    })
    return
  }

  if (request.method === "POST" && requestUrl.pathname.endsWith("/api/emulate/v1/emulateTrace")) {
    if (walletAddress === undefined) {
      sendJson(response, 409, {error: "Wallet account must be loaded before emulation"})
      return
    }
    sendJson(response, 200, successfulEmulation(walletAddress))
    return
  }

  if (requestUrl.pathname.endsWith("/api/v2/getTransactions")) {
    sendJson(response, 200, {ok: true, result: []})
    return
  }

  if (requestUrl.pathname.endsWith("/api/v3/nft/items")) {
    sendJson(
      response,
      200,
      scriptedNftItems(requestUrl.searchParams.get("owner_address") ?? SCRIPTED_NFT_OWNER),
    )
    return
  }

  sendJson(response, 404, {ok: false, error: `No scripted response for ${requestUrl.pathname}`})
}

const server = servesInsecureHttp
  ? createHttpServer(requestHandler)
  : createHttpsServer(
      {
        cert: readFileSync(path.join(TLS_FIXTURES, "localhost-cert.pem")),
        key: readFileSync(path.join(TLS_FIXTURES, "localhost-key.pem")),
      },
      requestHandler,
    )

server.listen(port, "127.0.0.1")

process.on("SIGTERM", () => void shutdown())
process.on("SIGINT", () => void shutdown())

/** Selects an isolated real localnet or restores the default scripted provider. */
async function selectProviderMode(mode: "localnet" | "scripted"): Promise<void> {
  await localnet?.stop()
  localnet = undefined
  fundingByAddress.clear()
  walletAddress = undefined
  if (mode === "localnet") {
    localnet = await ActonLocalnet.start()
  }
  providerMode = mode
}

/** Funds new wallet addresses and forwards their provider traffic to Acton localnet. */
async function proxyLocalnetRequest(
  request: IncomingMessage,
  requestUrl: URL,
  response: ServerResponse,
): Promise<void> {
  const actor: ActonLocalnet = requiredLocalnet()
  const address: string | null = requestUrl.searchParams.get("address")
  if (
    address !== null &&
    (requestUrl.pathname.endsWith("/api/v2/getAddressInformation") ||
      requestUrl.pathname.endsWith("/api/v2/getTransactions"))
  ) {
    walletAddress = address
    await ensureFunded(actor, address)
  }

  const body: Buffer | undefined =
    request.method === "GET" || request.method === "HEAD" ? undefined : await readBody(request)
  const getMethodCall: WalletGetMethodCall | undefined = parseRunGetMethod(body)
  if (getMethodCall !== undefined) {
    const served: unknown | undefined = await actor.walletGetMethod(
      getMethodCall.address,
      getMethodCall.method,
    )
    if (served !== undefined) {
      sendJson(response, 200, served)
      return
    }
  }
  const submitsMessage: boolean = isSendBocRequest(body)
  const previousTransactionIds: readonly string[] | undefined =
    submitsMessage && walletAddress !== undefined
      ? await actor.transactionIds(walletAddress)
      : undefined
  const headers = new Headers()
  for (const name of ["accept", "content-type", "x-api-key"]) {
    const raw: string | string[] | undefined = request.headers[name]
    const value: string | undefined = Array.isArray(raw) ? raw.join(", ") : raw
    if (value !== undefined) {
      headers.set(name, value)
    }
  }
  const upstream: Response = await actor.forward(requestUrl, request.method ?? "GET", headers, body)
  const upstreamBody = Buffer.from(await upstream.arrayBuffer())
  if (tracesRequests) {
    console.error(`Acton HTTP ${upstream.status}: ${upstreamBody.toString("utf8").slice(0, 2_000)}`)
  }
  if (upstream.ok && submitsMessage) {
    if (walletAddress === undefined || previousTransactionIds === undefined) {
      throw new Error("Wallet account must be loaded before sendBoc")
    }
    await actor.mineUntilTransaction(walletAddress, previousTransactionIds)
  }
  sendBytes(
    response,
    upstream.status,
    upstreamBody,
    upstream.headers.get("content-type") ?? "application/json; charset=utf-8",
  )
}

/** Funds each test wallet once even when account and activity requests race. */
async function ensureFunded(actor: ActonLocalnet, address: string): Promise<void> {
  let funding: Promise<void> | undefined = fundingByAddress.get(address)
  if (funding === undefined) {
    funding = actor.fundAccount(address, Number(WALLET_BALANCE_NANOGRAMS))
    fundingByAddress.set(address, funding)
  }
  await funding
}

/** Returns the active localnet after a successful mode-selection command. */
function requiredLocalnet(): ActonLocalnet {
  if (localnet === undefined) {
    throw new Error("Acton localnet mode is active without a running process")
  }
  return localnet
}

/** One JSON-RPC get-method invocation addressed to a localnet account. */
type WalletGetMethodCall = {
  readonly address: string
  readonly method: string
}

/** Extracts the target of a JSON-RPC `runGetMethod` body, if it is one. */
function parseRunGetMethod(body: Buffer | undefined): WalletGetMethodCall | undefined {
  if (body === undefined || body.length === 0) {
    return undefined
  }
  try {
    const value: unknown = JSON.parse(body.toString("utf8"))
    if (
      typeof value !== "object" ||
      value === null ||
      !("method" in value) ||
      value.method !== "runGetMethod" ||
      !("params" in value) ||
      typeof value.params !== "object" ||
      value.params === null ||
      !("address" in value.params) ||
      typeof value.params.address !== "string" ||
      !("method" in value.params) ||
      typeof value.params.method !== "string"
    ) {
      return undefined
    }
    return {address: value.params.address, method: value.params.method}
  } catch {
    return undefined
  }
}

/** Reports whether a JSON-RPC body submits a signed wallet message. */
function isSendBocRequest(body: Buffer | undefined): boolean {
  if (body === undefined || body.length === 0) {
    return false
  }
  try {
    const value: unknown = JSON.parse(body.toString("utf8"))
    return (
      typeof value === "object" && value !== null && "method" in value && value.method === "sendBoc"
    )
  } catch {
    return false
  }
}

/** Narrows an untrusted control body to one supported provider mode. */
function isProviderModeCommand(value: unknown): value is {readonly mode: "localnet" | "scripted"} {
  return (
    typeof value === "object" &&
    value !== null &&
    "mode" in value &&
    (value.mode === "localnet" || value.mode === "scripted")
  )
}

/** Reads a bounded provider request body without accepting unbounded test input. */
async function readBody(request: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = []
  let size: number = 0
  for await (const chunk of request) {
    const buffer: Buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk)
    size += buffer.length
    if (size > 2 * 1024 * 1024) {
      throw new Error("Provider request body exceeds 2 MiB")
    }
    chunks.push(buffer)
  }
  return Buffer.concat(chunks)
}

/** Stops the HTTP listener and any localnet process owned by this provider. */
async function shutdown(): Promise<void> {
  await localnet?.stop()
  server.close()
}

/** Returns the requested loopback port or the default used by the Web E2E runner. */
function parsePort(arguments_: readonly string[], fallback: number): number {
  const portIndex: number = arguments_.indexOf("--port")
  const value: string | undefined = portIndex === -1 ? undefined : arguments_[portIndex + 1]
  if (value === undefined) {
    return fallback
  }
  const parsed: number = Number.parseInt(value, 10)
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65_535) {
    throw new Error(`Invalid provider port: ${value}`)
  }
  return parsed
}

/** Sends a JSON response that browser clients can read from another local origin. */
function sendJson(response: ServerResponse, status: number, value: unknown): void {
  const body: string = JSON.stringify(value)
  response.writeHead(status, {
    ...corsHeaders(),
    "content-length": Buffer.byteLength(body).toString(),
    "content-type": "application/json; charset=utf-8",
  })
  response.end(body)
}

/** Sends an upstream localnet response with browser-readable CORS headers. */
function sendBytes(
  response: ServerResponse,
  status: number,
  body: Buffer,
  contentType: string,
): void {
  response.writeHead(status, {
    ...corsHeaders(),
    "content-length": body.length.toString(),
    "content-type": contentType,
  })
  response.end(body)
}

/** Declares the methods and headers accepted by the scripted provider. */
function corsHeaders(): Record<string, string> {
  return {
    "access-control-allow-headers": "content-type, x-api-key",
    "access-control-allow-methods": "GET, POST, OPTIONS",
    "access-control-allow-origin": "*",
  }
}

/** Returns collectible fixtures so screenshots exercise real metadata and artwork rendering. */
function scriptedNftItems(ownerAddress: string): unknown {
  return {
    nft_items: [
      {
        address: "0:2222222222222222222222222222222222222222222222222222222222222222",
        code_hash: "scripted-code-aurora",
        collection: {
          address: SCRIPTED_NFT_COLLECTION,
          collection_content: {name: "Acton Originals"},
        },
        content: {
          description: "A deterministic collectible used by the wallet example.",
          image: scriptedNftDataUri("aurora"),
          name: "Aurora Relay",
        },
        data_hash: "scripted-data-aurora",
        index: "1",
        init: true,
        last_transaction_lt: "200",
        on_sale: true,
        owner_address: ownerAddress,
      },
      {
        address: "0:3333333333333333333333333333333333333333333333333333333333333333",
        code_hash: "scripted-code-signal",
        collection: {
          address: SCRIPTED_NFT_COLLECTION,
          collection_content: {name: "Acton Originals"},
        },
        content: {
          description: "A second fixture that verifies horizontal collection layout.",
          image_url: scriptedNftArtworkUrl("signal"),
          name: "Signal Bloom",
        },
        data_hash: "scripted-data-signal",
        index: "2",
        init: true,
        last_transaction_lt: "100",
        on_sale: false,
        owner_address: ownerAddress,
      },
    ],
  }
}

function scriptedNftArtworkUrl(variant: string): string {
  const protocol: string = servesInsecureHttp ? "http" : "https"
  return `${protocol}://127.0.0.1:${port}/e2e/nft-art.svg?variant=${variant}`
}

function scriptedNftDataUri(variant: string): string {
  return `data:image/svg+xml,${encodeURIComponent(scriptedNftArtwork(variant))}`
}

/** Produces local flat artwork without relying on third-party image hosts. */
function scriptedNftArtwork(variant: string | null): string {
  const signal: boolean = variant === "signal"
  const background: string = signal ? "#182119" : "#181c2b"
  const accent: string = signal ? "#9dd49a" : "#b9b5ff"
  const secondary: string = signal ? "#f1c68a" : "#87d7d0"
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640" role="img">
  <rect width="640" height="640" fill="${background}"/>
  <circle cx="180" cy="170" r="112" fill="${accent}"/>
  <circle cx="465" cy="430" r="150" fill="${secondary}"/>
  <path d="M92 510 327 98l221 414Z" fill="none" stroke="#f7f5ef" stroke-width="26"/>
  <circle cx="320" cy="320" r="54" fill="${background}" stroke="#f7f5ef" stroke-width="18"/>
</svg>`
}

/** Returns a successful trace rooted at the wallet most recently loaded by the engine. */
function successfulEmulation(account: string): unknown {
  return {
    actions: [],
    is_incomplete: false,
    mc_block_seqno: 42,
    rand_seed: "",
    trace: {children: [], tx_hash: "root"},
    transactions: {
      root: {
        account,
        description: {
          aborted: false,
          action: {result_code: 0, success: true},
          compute_ph: {exit_code: 0, success: true},
          type: "ord",
        },
        total_fees: "1000000",
      },
    },
  }
}
