import {spawn, type ChildProcess} from "node:child_process"
import {existsSync, mkdtempSync, rmSync, writeFileSync} from "node:fs"
import {createServer} from "node:net"
import {homedir, tmpdir} from "node:os"
import path from "node:path"
import process from "node:process"

const READY_TIMEOUT_MS: number = 15_000
const TRANSACTION_TIMEOUT_MS: number = 5_000
const FIXED_LOCALNET_TIME: number = 2_000_000_000

/** Owns one isolated Acton localnet process used by client E2E scenarios. */
export class ActonLocalnet {
  readonly url: string

  private readonly child: ChildProcess
  private readonly directory: string
  private readonly output: string[] = []

  /** Retains the process, temporary project, and loopback endpoint until shutdown. */
  private constructor(child: ChildProcess, directory: string, url: string) {
    this.child = child
    this.directory = directory
    this.url = url
    child.stdout?.on("data", chunk => this.output.push(String(chunk)))
    child.stderr?.on("data", chunk => this.output.push(String(chunk)))
  }

  /** Starts a manually mined localnet with a stable clock for deterministic activity rows. */
  static async start(): Promise<ActonLocalnet> {
    const directory: string = mkdtempSync(path.join(tmpdir(), "wallet-engine-client-e2e-"))
    writeFileSync(
      path.join(directory, "Acton.toml"),
      [
        "[package]",
        'name = "wallet-engine-client-e2e"',
        'description = "Wallet Engine client E2E localnet"',
        'version = "0.0.0"',
        'license = "MIT"',
        "",
        "[localnet]",
        "",
      ].join("\n"),
    )
    const port: number = await freePort()
    const binary: string = actonBinary()
    const child = spawn(
      binary,
      [
        "--project-root",
        directory,
        "localnet",
        "start",
        "--port",
        port.toString(),
        "--block-interval-ms",
        "50",
        "--no-mining",
      ],
      {
        cwd: directory,
        env: {...process.env, NO_COLOR: "1", TOKIO_WORKER_THREADS: "1"},
        stdio: ["ignore", "pipe", "pipe"],
      },
    )
    const localnet = new ActonLocalnet(child, directory, `http://127.0.0.1:${port}`)
    try {
      await localnet.waitUntilReady()
      await localnet.postControl("/acton_setTime", {timestamp: FIXED_LOCALNET_TIME})
      return localnet
    } catch (error) {
      await localnet.stop()
      throw error
    }
  }

  /** Queues a faucet transfer and mines the block that confirms it for the new wallet. */
  async fundAccount(address: string, amount: number): Promise<void> {
    await this.postControl("/acton_fundAccount", {address, amount})
    await this.mine()
  }

  /** Mines all external messages submitted since the preceding block. */
  async mine(): Promise<void> {
    await this.postControl("/acton_mine", {})
  }

  /** Returns stable identifiers for the transactions currently visible on an account. */
  async transactionIds(address: string): Promise<readonly string[]> {
    const query = new URLSearchParams({address, limit: "20"})
    const response: Response = await fetch(`${this.url}/api/v2/getTransactions?${query}`)
    const body: unknown = await response.json()
    if (!response.ok) {
      throw new Error(
        `Acton localnet getTransactions returned HTTP ${response.status}: ${JSON.stringify(body)}`,
      )
    }
    return transactionIds(body)
  }

  /** Mines until the wallet account exposes a transaction that was absent before submission. */
  async mineUntilTransaction(address: string, previousIds: readonly string[]): Promise<void> {
    const previous = new Set(previousIds)
    const deadline: number = Date.now() + TRANSACTION_TIMEOUT_MS
    while (Date.now() < deadline) {
      await this.mine()
      const currentIds: readonly string[] = await this.transactionIds(address)
      if (currentIds.some(id => !previous.has(id))) {
        return
      }
      await delay(50)
    }
    throw new Error(`Timed out waiting for a transaction on localnet account ${address}`)
  }

  /** Forwards one wallet provider request to the TON-compatible localnet API. */
  async forward(
    requestUrl: URL,
    method: string,
    headers: HeadersInit,
    body: Buffer | undefined,
  ): Promise<Response> {
    return await fetch(`${this.url}${requestUrl.pathname}${requestUrl.search}`, {
      body: body?.toString("utf8"),
      headers,
      method,
    })
  }

  /** Stops the localnet and removes only its owned temporary Acton project. */
  async stop(): Promise<void> {
    if (this.child.exitCode === null && this.child.signalCode === null) {
      this.child.kill("SIGTERM")
      await new Promise<void>(resolve => {
        const fallback = setTimeout(() => {
          if (this.child.exitCode === null && this.child.signalCode === null) {
            this.child.kill("SIGKILL")
          }
          resolve()
        }, 2_000)
        this.child.once("exit", () => {
          clearTimeout(fallback)
          resolve()
        })
      })
    }
    rmSync(this.directory, {force: true, recursive: true})
  }

  /** Waits until Toncenter-compatible masterchain information is available. */
  private async waitUntilReady(): Promise<void> {
    const deadline: number = Date.now() + READY_TIMEOUT_MS
    while (Date.now() < deadline) {
      if (this.child.exitCode !== null || this.child.signalCode !== null) {
        throw new Error(`Acton localnet exited during startup\n${this.logs()}`)
      }
      try {
        const response: Response = await fetch(`${this.url}/api/v2/getMasterchainInfo`)
        if (response.ok) {
          return
        }
      } catch {
        // The listener can refuse connections while Acton finishes startup.
      }
      await delay(50)
    }
    throw new Error(`Timed out waiting for Acton localnet at ${this.url}\n${this.logs()}`)
  }

  /** Sends one localnet control command and preserves its response in failures. */
  private async postControl(route: string, value: unknown): Promise<void> {
    const response: Response = await fetch(`${this.url}${route}`, {
      body: JSON.stringify(value),
      headers: {"content-type": "application/json"},
      method: "POST",
    })
    const body: string = await response.text()
    if (!response.ok) {
      throw new Error(`Acton localnet ${route} returned HTTP ${response.status}: ${body}`)
    }
  }

  /** Returns output emitted by the localnet process for startup and proxy diagnostics. */
  private logs(): string {
    return this.output.join("")
  }
}

/** Returns an explicit Acton binary, its standard installation, or the PATH command. */
function actonBinary(): string {
  const configured: string | undefined = process.env.WALLET_ENGINE_ACTON_BIN
  if (configured !== undefined && configured.length > 0) {
    return configured
  }
  const standard: string = path.join(homedir(), ".acton/bin/acton")
  return existsSync(standard) ? standard : "acton"
}

/** Returns a currently unused loopback port for one short-lived localnet process. */
async function freePort(): Promise<number> {
  return await new Promise<number>((resolve, reject) => {
    const server = createServer()
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      if (address === null || typeof address === "string") {
        server.close()
        reject(new Error("Could not allocate a localnet port"))
        return
      }
      server.close(error => (error ? reject(error) : resolve(address.port)))
    })
  })
}

/** Delays a readiness poll without blocking the provider server process. */
async function delay(milliseconds: number): Promise<void> {
  await new Promise<void>(resolve => setTimeout(resolve, milliseconds))
}

/** Extracts stable transaction identifiers from a Toncenter-compatible response. */
function transactionIds(value: unknown): readonly string[] {
  if (
    typeof value !== "object" ||
    value === null ||
    !("result" in value) ||
    !Array.isArray(value.result)
  ) {
    throw new Error(`Unexpected localnet getTransactions response: ${JSON.stringify(value)}`)
  }
  return value.result.map((transaction, index) => {
    if (
      typeof transaction !== "object" ||
      transaction === null ||
      !("transaction_id" in transaction) ||
      typeof transaction.transaction_id !== "object" ||
      transaction.transaction_id === null ||
      !("lt" in transaction.transaction_id) ||
      !("hash" in transaction.transaction_id)
    ) {
      throw new Error(`Transaction ${index} does not contain transaction_id`)
    }
    return `${String(transaction.transaction_id.lt)}:${String(transaction.transaction_id.hash)}`
  })
}
