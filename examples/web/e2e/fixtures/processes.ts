import {spawn, type ChildProcess} from "node:child_process"
import {existsSync} from "node:fs"
import {request as requestHttps} from "node:https"
import {createServer} from "node:net"
import path from "node:path"
import process from "node:process"

const REPOSITORY_ROOT: string = path.resolve(import.meta.dirname, "../../../..")
const DEFAULT_BRIDGE_BINARY: string = "/tmp/ton-connect-research/bridge/bridge3"
const PROCESS_START_TIMEOUT_MS: number = 10_000

export interface DappManifestConfig {
  readonly iconUrl: string
  readonly name: string
  readonly url: string
}

export interface DappActorConfig {
  readonly inNetwork?: string
  readonly manifest: DappManifestConfig
  readonly manifestUrl: string
  readonly universalLink: string
}

export interface DappActorOptions {
  readonly secure?: boolean
}

interface RenderedDappActorConfig extends DappActorConfig {
  readonly bridgeUrl: string
}

export interface DappActorError {
  readonly cause?: unknown
  readonly message: string
  readonly name: string
}

export interface DappActorState {
  readonly account: {
    readonly address: string
    readonly chain: string
    readonly publicKey: string | null
    readonly walletStateInit: string
  } | null
  readonly config: RenderedDappActorConfig
  readonly device: {
    readonly appName: string
    readonly appVersion: string
    readonly features: readonly unknown[]
    readonly maxProtocolVersion: number
    readonly platform: string
  } | null
  readonly error: DappActorError | null
  readonly journal: readonly {
    readonly at: number
    readonly details?: unknown
    readonly type: string
  }[]
  readonly signMessage: DappOperationState
  readonly status: string
  readonly transaction: DappOperationState
}

interface DappOperationState {
  readonly error: DappActorError | null
  readonly request: unknown | null
  readonly result: unknown | null
  readonly status: string
}

export type DappCommand =
  | {readonly proofPayload?: string; readonly type: "connect"}
  | {readonly type: "clear_storage" | "disconnect" | "reset" | "restore"}
  | {readonly transaction: unknown; readonly type: "send_transaction" | "sign_message"}

/** Owns one child process and retains its output for failed E2E scenarios. */
class ManagedProcess {
  private readonly child: ChildProcess
  private readonly name: string
  private readonly output: string[] = []

  /** Captures output and lifecycle state for one child process. */
  constructor(name: string, child: ChildProcess) {
    this.name = name
    this.child = child
    child.stdout?.on("data", chunk => this.output.push(String(chunk)))
    child.stderr?.on("data", chunk => this.output.push(String(chunk)))
  }

  /** Returns whether the process exited before its fixture was released. */
  exited(): boolean {
    return this.child.exitCode !== null || this.child.signalCode !== null
  }

  /** Returns the complete stdout and stderr captured for diagnostics. */
  logs(): string {
    return this.output.join("")
  }

  /** Stops the fixture and waits until its operating-system process exits. */
  async stop(): Promise<void> {
    if (this.exited()) {
      return
    }
    this.child.kill("SIGTERM")
    await new Promise<void>(resolve => {
      const fallback = setTimeout(() => {
        if (!this.exited()) {
          this.child.kill("SIGKILL")
        }
        resolve()
      }, 2000)
      this.child.once("exit", () => {
        clearTimeout(fallback)
        resolve()
      })
    })
  }

  /** Throws a startup error that includes output emitted before process exit. */
  assertRunning(): void {
    if (this.exited()) {
      throw new Error(`${this.name} exited during startup\n${this.logs()}`)
    }
  }
}

/** Runs the official Go bridge with isolated in-memory storage. */
export class OfficialBridge {
  private readonly managedProcess: ManagedProcess
  readonly url: string

  /** Retains the managed bridge process and its isolated endpoint. */
  private constructor(process: ManagedProcess, url: string) {
    this.managedProcess = process
    this.url = url
  }

  /** Starts the bridge selected by TON_CONNECT_BRIDGE_BIN on free loopback ports. */
  static async start(): Promise<OfficialBridge> {
    const binary: string = process.env.TON_CONNECT_BRIDGE_BIN ?? DEFAULT_BRIDGE_BINARY
    if (!existsSync(binary)) {
      throw new Error(
        `Official TON Connect bridge not found at ${binary}; set TON_CONNECT_BRIDGE_BIN`,
      )
    }
    const [port, metricsPort] = await Promise.all([freePort(), freePort()])
    const child = spawn(binary, [], {
      env: {
        ...process.env,
        CORS_ENABLE: "true",
        METRICS_PORT: metricsPort.toString(),
        NTP_ENABLED: "false",
        PORT: port.toString(),
        STORAGE: "memory",
      },
      stdio: ["ignore", "pipe", "pipe"],
    })
    const managed = new ManagedProcess("official TON Connect bridge", child)
    await waitForHttp(`http://127.0.0.1:${metricsPort}/readyz`, managed, false)
    return new OfficialBridge(managed, `http://127.0.0.1:${port}/bridge`)
  }

  /** Returns output emitted by the bridge process. */
  logs(): string {
    return this.managedProcess.logs()
  }

  /** Stops the bridge after all scenarios in its worker have completed. */
  async stop(): Promise<void> {
    await this.managedProcess.stop()
  }
}

/** Controls one official-SDK dApp actor and exposes its recorded state. */
export class DappActor {
  readonly config: RenderedDappActorConfig
  readonly origin: string
  private readonly managedProcess: ManagedProcess

  /** Retains the managed dApp process, endpoint, and rendered manifest configuration. */
  private constructor(process: ManagedProcess, origin: string, config: RenderedDappActorConfig) {
    this.managedProcess = process
    this.origin = origin
    this.config = config
  }

  /** Starts a dApp actor with a real local manifest and the supplied bridge. */
  static async start(
    bridgeUrl: string,
    config: DappActorConfig,
    options: DappActorOptions = {},
  ): Promise<DappActor> {
    const actorPath: string = path.join(REPOSITORY_ROOT, "tests/ton-connect/dapp/dist/server.js")
    if (!existsSync(actorPath)) {
      throw new Error(`TON Connect dApp is not built at ${actorPath}; run bun run e2e:prepare`)
    }
    const port: number = await freePort()
    const secure: boolean = options.secure ?? true
    const origin: string = `${secure ? "https" : "http"}://127.0.0.1:${port}`
    const renderedConfig: RenderedDappActorConfig = {
      ...config,
      bridgeUrl,
      manifest: renderManifest(config.manifest, origin),
      manifestUrl: renderOrigin(config.manifestUrl, origin),
    }
    const fixtures: string = path.join(REPOSITORY_ROOT, "tests/ton-connect/dapp/fixtures")
    const child = spawn(process.execPath, [actorPath], {
      env: {
        ...process.env,
        PORT: port.toString(),
        TON_CONNECT_DAPP_CONFIG: JSON.stringify(renderedConfig),
        ...(secure
          ? {
              TON_CONNECT_TLS_CERTIFICATE: path.join(fixtures, "localhost-cert.pem"),
              TON_CONNECT_TLS_KEY: path.join(fixtures, "localhost-key.pem"),
            }
          : {TON_CONNECT_INSECURE_HTTP: "1"}),
      },
      stdio: ["ignore", "pipe", "pipe"],
    })
    const managed = new ManagedProcess("TON Connect dApp actor", child)
    await waitForHttp(`${origin}/health`, managed, secure)
    return new DappActor(managed, origin, renderedConfig)
  }

  /** Sends one validated command to the dApp SDK instance. */
  async command<T>(command: DappCommand): Promise<T> {
    return await requestJson<T>(`${this.origin}/command`, "POST", command)
  }

  /** Reads the latest dApp account, request, response, and journal snapshot. */
  async state(): Promise<DappActorState> {
    return await requestJson<DappActorState>(`${this.origin}/state`, "GET")
  }

  /** Waits until a state predicate observes the requested protocol outcome. */
  async waitFor(
    predicate: (state: DappActorState) => boolean,
    description: string,
  ): Promise<DappActorState> {
    const deadline: number = Date.now() + PROCESS_START_TIMEOUT_MS
    while (Date.now() < deadline) {
      const state: DappActorState = await this.state()
      if (predicate(state)) {
        return state
      }
      await delay(50)
    }
    throw new Error(
      `Timed out waiting for dApp ${description}: ${JSON.stringify(await this.state())}`,
    )
  }

  /** Returns output emitted by the dApp actor process. */
  logs(): string {
    return this.managedProcess.logs()
  }

  /** Stops the dApp actor and its active SDK connection. */
  async stop(): Promise<void> {
    await this.managedProcess.stop()
  }
}

/** Replaces actor-origin placeholders in the manifest served to the wallet. */
function renderManifest(config: DappManifestConfig, origin: string): DappManifestConfig {
  return {
    iconUrl: renderOrigin(config.iconUrl, origin),
    name: config.name,
    url: renderOrigin(config.url, origin),
  }
}

/** Replaces every runtime origin placeholder in one dApp configuration value. */
function renderOrigin(value: string, origin: string): string {
  return value.replaceAll("{actor_origin}", origin)
}

/** Reserves and releases one currently unused TCP port on loopback. */
async function freePort(): Promise<number> {
  return await new Promise<number>((resolve, reject) => {
    const server = createServer()
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      if (address === null || typeof address === "string") {
        server.close()
        reject(new Error("Could not allocate a loopback port"))
        return
      }
      server.close(error => (error ? reject(error) : resolve(address.port)))
    })
  })
}

/** Polls a process health endpoint until it becomes reachable or startup times out. */
async function waitForHttp(
  url: string,
  process_: ManagedProcess,
  allowInvalidCertificate: boolean,
): Promise<void> {
  const deadline: number = Date.now() + PROCESS_START_TIMEOUT_MS
  while (Date.now() < deadline) {
    process_.assertRunning()
    try {
      const status: number = allowInvalidCertificate
        ? await httpsStatus(url)
        : (await fetch(url)).status
      if (status >= 200 && status < 300) {
        return
      }
    } catch {
      // The fixture can refuse connections while its listener is starting.
    }
    await delay(50)
  }
  throw new Error(`Timed out waiting for ${url}\n${process_.logs()}`)
}

/** Reads an HTTPS status while accepting the repository's loopback test certificate. */
async function httpsStatus(url: string): Promise<number> {
  return await new Promise<number>((resolve, reject) => {
    const request = requestHttps(url, {rejectUnauthorized: false}, response => {
      response.resume()
      resolve(response.statusCode ?? 0)
    })
    request.once("error", reject)
    request.end()
  })
}

/** Exchanges one JSON document with the local HTTPS dApp actor. */
async function requestJson<T>(url: string, method: "GET" | "POST", value?: unknown): Promise<T> {
  const body: string | undefined = value === undefined ? undefined : JSON.stringify(value)
  if (url.startsWith("http://")) {
    const response: Response = await fetch(url, {
      body,
      headers:
        body === undefined
          ? {accept: "application/json"}
          : {accept: "application/json", "content-type": "application/json"},
      method,
    })
    const responseBody: string = await response.text()
    if (!response.ok) {
      throw new Error(`dApp actor returned HTTP ${response.status}: ${responseBody}`)
    }
    return JSON.parse(responseBody) as T
  }
  return await new Promise<T>((resolve, reject) => {
    const request = requestHttps(
      url,
      {
        headers:
          body === undefined
            ? {accept: "application/json"}
            : {
                accept: "application/json",
                "content-length": Buffer.byteLength(body).toString(),
                "content-type": "application/json",
              },
        method,
        rejectUnauthorized: false,
      },
      response => {
        const chunks: Buffer[] = []
        response.on("data", chunk => chunks.push(Buffer.from(chunk)))
        response.on("end", () => {
          const responseBody: string = Buffer.concat(chunks).toString("utf8")
          const status: number = response.statusCode ?? 0
          if (status < 200 || status >= 300) {
            reject(new Error(`dApp actor returned HTTP ${status}: ${responseBody}`))
            return
          }
          try {
            resolve(JSON.parse(responseBody) as T)
          } catch (error) {
            reject(error)
          }
        })
      },
    )
    request.once("error", reject)
    if (body !== undefined) {
      request.write(body)
    }
    request.end()
  })
}

/** Waits for one short polling interval. */
async function delay(milliseconds: number): Promise<void> {
  await new Promise<void>(resolve => setTimeout(resolve, milliseconds))
}
