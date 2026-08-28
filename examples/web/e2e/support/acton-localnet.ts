import {spawn, type ChildProcess} from "node:child_process"
import {existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from "node:fs"
import {createServer} from "node:net"
import {homedir, tmpdir} from "node:os"
import path from "node:path"
import process from "node:process"

import {beginCell, Cell, Dictionary} from "@ton/core"

const READY_TIMEOUT_MS: number = 15_000
const TRANSACTION_TIMEOUT_MS: number = 5_000
const FIXED_LOCALNET_TIME: number = 2_000_000_000
const BYTECODE_CONFIG_KEY: number = -123
const WALLET_TG_CODE_PATH: string = path.resolve(
  import.meta.dirname,
  "../../../../tests/support/wallet_tg_rev00.code",
)
const WALLET_STORAGE_BITS: number = 328
const WALLET_GET_METHODS: readonly string[] = ["seqno", "get_subwallet_id", "get_public_key"]

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
      await localnet.installWalletBytecode()
      await localnet.postControl("/acton_setTime", {timestamp: FIXED_LOCALNET_TIME})
      return localnet
    } catch (error) {
      await localnet.stop()
      throw error
    }
  }

  /**
   * Publishes the real wallet bytecode into the localnet blockchain config.
   *
   * Deployed accounts carry only the wallet trampoline, whose code jumps into
   * the bytecode stored at config param -123 (already present on testnet). A
   * fresh localnet config lacks that param, so every wallet execution would
   * fail without this patch.
   */
  private async installWalletBytecode(): Promise<void> {
    const response: Response = await fetch(`${this.url}/acton_dumpState`)
    const state: unknown = await response.json()
    if (!response.ok || !isLocalnetState(state)) {
      throw new Error(`Acton localnet state dump failed with HTTP ${response.status}`)
    }

    const configHash: string = state.globals.config_boc_hash
    const entry: [string, string] | undefined = state.cas_entries.find(
      pair => pair[0] === configHash,
    )
    if (entry === undefined) {
      throw new Error("Acton localnet cell storage has no config cell")
    }

    const params = Dictionary.loadDirect(
      Dictionary.Keys.Int(32),
      Dictionary.Values.Cell(),
      Cell.fromBase64(entry[1]).asSlice(),
    )
    params.set(
      BYTECODE_CONFIG_KEY,
      Cell.fromBase64(readFileSync(WALLET_TG_CODE_PATH, "utf8").trim()),
    )
    const patched: Cell = beginCell().storeDictDirect(params).endCell()

    entry[0] = patched.hash().toString("hex")
    entry[1] = patched.toBoc().toString("base64")
    state.globals.config_boc_hash = entry[0]
    await this.postControl("/acton_loadState", state)
  }

  /**
   * Serves a wallet get method from the account's storage cell.
   *
   * Localnet's `runGetMethod` executes the on-account trampoline without the
   * chain-state config, so `CONFIGOPTPARAM -123` fails there even though
   * transactions see the patched config. Real networks run get methods
   * against the full masterchain config, so the provider answers the wallet's
   * storage-backed get methods locally instead. Returns `undefined` for other
   * methods or accounts without parsable wallet storage.
   */
  async walletGetMethod(address: string, method: string): Promise<unknown | undefined> {
    if (!WALLET_GET_METHODS.includes(method)) {
      return undefined
    }

    const query = new URLSearchParams({address})
    const response: Response = await fetch(`${this.url}/api/v2/getAddressInformation?${query}`)
    const body: unknown = await response.json()
    if (!response.ok) {
      throw new Error(
        `Acton localnet getAddressInformation returned HTTP ${response.status}: ${JSON.stringify(body)}`,
      )
    }
    const data: string | undefined = accountData(body)
    if (data === undefined || data.length === 0) {
      return undefined
    }

    let storage: WalletStorage
    try {
      storage = parseWalletStorage(Cell.fromBase64(data))
    } catch {
      return undefined
    }
    const value: string =
      method === "seqno"
        ? `0x${storage.seqno.toString(16)}`
        : method === "get_subwallet_id"
          ? `0x${storage.subwalletId.toString(16)}`
          : `0x${storage.publicKey.toString("hex")}`
    return {
      ok: true,
      result: {
        "@type": "smc.runResult",
        exit_code: 0,
        gas_used: 0,
        stack: [["num", value]],
      },
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

/** The dumped localnet fields that the config patch reads and rewrites. */
type LocalnetState = {
  globals: {config_boc_hash: string} & Record<string, unknown>
  cas_entries: [string, string][]
} & Record<string, unknown>

/** Narrows an untrusted state dump to the fields required by the config patch. */
function isLocalnetState(value: unknown): value is LocalnetState {
  return (
    typeof value === "object" &&
    value !== null &&
    "globals" in value &&
    typeof value.globals === "object" &&
    value.globals !== null &&
    "config_boc_hash" in value.globals &&
    typeof value.globals.config_boc_hash === "string" &&
    "cas_entries" in value &&
    Array.isArray(value.cas_entries) &&
    value.cas_entries.every(
      entry =>
        Array.isArray(entry) &&
        entry.length === 2 &&
        typeof entry[0] === "string" &&
        typeof entry[1] === "string",
    )
  )
}

/** The public wallet state stored in the trampoline account's data cell. */
type WalletStorage = {
  readonly seqno: number
  readonly subwalletId: number
  readonly publicKey: Buffer
}

/** Extracts the account data BoC from a Toncenter-compatible response. */
function accountData(value: unknown): string | undefined {
  if (
    typeof value !== "object" ||
    value === null ||
    !("result" in value) ||
    typeof value.result !== "object" ||
    value.result === null ||
    !("data" in value.result) ||
    typeof value.result.data !== "string"
  ) {
    return undefined
  }
  return value.result.data
}

/** Reads the wallet's revision-00 storage layout, or throws for other cells. */
function parseWalletStorage(root: Cell): WalletStorage {
  const slice = root.beginParse()
  if (slice.remainingBits !== WALLET_STORAGE_BITS || slice.remainingRefs !== 0) {
    throw new Error("The account data cell is not wallet revision-00 storage")
  }
  slice.skip(8)
  const seqno: number = slice.loadUint(32)
  const subwalletId: number = slice.loadUint(32)
  const publicKey: Buffer = slice.loadBuffer(32)
  return {publicKey, seqno, subwalletId}
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
