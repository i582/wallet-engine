import { readFileSync } from "node:fs";
import type { IncomingMessage, ServerResponse } from "node:http";
import { createServer } from "node:https";

import {
  TonConnect,
  type IStorage,
  type SendTransactionRequest,
  type Wallet
} from "@tonconnect/sdk";

interface ActorEvent {
  readonly at: number;
  readonly type: string;
  readonly details?: unknown;
}

interface ActorState {
  status: "idle" | "connecting" | "connected" | "disconnecting" | "disconnected" | "error";
  connectLink: string | null;
  account: {
    address: string;
    chain: string;
    publicKey: string | null;
    walletStateInit: string;
  } | null;
  device: Wallet["device"] | null;
  config: DappActorConfig;
  error: ActorError | null;
  transaction: TransactionState;
  journal: ActorEvent[];
}

interface ActorError {
  readonly name: string;
  readonly message: string;
  readonly cause?: unknown;
}

interface TransactionState {
  status: "idle" | "pending" | "success" | "error";
  request: SendTransactionRequest | null;
  result: unknown | null;
  error: ActorError | null;
}

type ActorCommand =
  | { readonly type: "connect"; readonly proofPayload?: string }
  | { readonly type: "restore" | "disconnect" | "clear_storage" }
  | { readonly type: "send_transaction"; readonly transaction: SendTransactionRequest };

interface DappActorConfig {
  readonly bridgeUrl: string;
  readonly manifestUrl: string;
  readonly universalLink: string;
  readonly inNetwork?: string;
  readonly manifest: {
    readonly url: string;
    readonly name: string;
    readonly iconUrl: string;
  };
}

class MemoryStorage implements IStorage {
  readonly #items = new Map<string, string>();

  /** Persists one SDK session value for the lifetime of this actor process. */
  async setItem(key: string, value: string): Promise<void> {
    this.#items.set(key, value);
  }

  /** Returns a stored SDK session value without exposing the backing map. */
  async getItem(key: string): Promise<string | null> {
    return this.#items.get(key) ?? null;
  }

  /** Removes one SDK session value during disconnect or session replacement. */
  async removeItem(key: string): Promise<void> {
    this.#items.delete(key);
  }

  /** Clears all SDK persistence for deterministic restore and reset scenarios. */
  clear(): void {
    this.#items.clear();
  }
}

const port = parsePort(process.env.PORT, 4173);
const config = parseConfig(process.env.TON_CONNECT_DAPP_CONFIG);
const tlsKeyPath = requiredEnvironment("TON_CONNECT_TLS_KEY");
const tlsCertificatePath = requiredEnvironment("TON_CONNECT_TLS_CERTIFICATE");
const storage = new MemoryStorage();
const state: ActorState = {
  status: "idle",
  connectLink: null,
  account: null,
  device: null,
  config,
  error: null,
  transaction: {
    status: "idle",
    request: null,
    result: null,
    error: null
  },
  journal: []
};

const connector = new TonConnect({
  manifestUrl: config.manifestUrl,
  storage,
  disableAutoPauseConnection: true
});
if (config.inNetwork !== undefined) {
  connector.setConnectionNetwork(config.inNetwork);
}

connector.onStatusChange(
  wallet => {
    applyWallet(wallet);
    record(wallet ? "wallet_connected" : "wallet_disconnected", state.account);
  },
  error => {
    state.status = "error";
    state.error = actorError(error);
    record("connector_error", state.error);
  }
);

const server = createServer(
  {
    key: readFileSync(tlsKeyPath),
    cert: readFileSync(tlsCertificatePath)
  },
  (request, response) => {
    void route(request, response).catch(error => {
      const message = error instanceof Error ? error.message : String(error);
      state.status = "error";
      state.error = actorError(error);
      record("command_error", { message });
      sendJson(response, 400, { error: message });
    });
  }
);

server.listen(port, "127.0.0.1");

process.on("SIGTERM", () => {
  server.close();
});

/** Dispatches the actor's health, state, manifest, icon, and command endpoints. */
async function route(request: IncomingMessage, response: ServerResponse): Promise<void> {
  if (request.method === "GET" && request.url === "/health") {
    sendJson(response, 200, { status: "ok" });
    return;
  }

  if (request.method === "GET" && request.url === "/state") {
    sendJson(response, 200, state);
    return;
  }

  if (request.method === "GET" && request.url === new URL(config.manifestUrl).pathname) {
    sendJson(response, 200, config.manifest);
    return;
  }

  if (request.method === "GET" && request.url === new URL(config.manifest.iconUrl).pathname) {
    sendPng(response);
    return;
  }

  if (request.method === "POST" && request.url === "/command") {
    const command = await readCommand(request);
    await executeCommand(command, response);
    return;
  }

  sendJson(response, 404, { error: "not found" });
}

/** Executes one validated test command against the official TON Connect SDK instance. */
async function executeCommand(command: ActorCommand, response: ServerResponse): Promise<void> {
  switch (command.type) {
    case "connect": {
      state.status = "connecting";
      state.error = null;
      const options = command.proofPayload
        ? { request: { tonProof: command.proofPayload } }
        : undefined;
      const link = connector.connect(
        { universalLink: config.universalLink, bridgeUrl: config.bridgeUrl },
        options
      );
      if (typeof link !== "string") {
        throw new Error("HTTP bridge connection did not produce a connect link");
      }
      state.connectLink = link;
      record("connect_link_created", { link });
      sendJson(response, 200, { link });
      return;
    }
    case "restore": {
      await connector.restoreConnection();
      applyWallet(connector.wallet);
      record("connection_restored", state.account);
      sendJson(response, 200, { status: state.status });
      return;
    }
    case "disconnect": {
      state.status = "disconnecting";
      record("disconnect_requested");
      void connector.disconnect().then(
        () => {
          applyWallet(null);
          record("dapp_disconnected");
        },
        error => {
          const message = error instanceof Error ? error.message : String(error);
          state.status = "error";
          state.error = actorError(error);
          record("disconnect_error", { message });
        }
      );
      sendJson(response, 200, { status: state.status });
      return;
    }
    case "send_transaction": {
      state.transaction = {
        status: "pending",
        request: command.transaction,
        result: null,
        error: null
      };
      record("transaction_requested", command.transaction);
      void connector.sendTransaction(command.transaction, {
        onRequestSent: () => record("transaction_sent")
      }).then(
        result => {
          state.transaction.status = "success";
          state.transaction.result = result;
          record("transaction_succeeded", result);
        },
        error => {
          state.transaction.status = "error";
          state.transaction.error = actorError(error);
          record("transaction_failed", state.transaction.error);
        }
      );
      sendJson(response, 202, { status: state.transaction.status });
      return;
    }
    case "clear_storage": {
      storage.clear();
      record("storage_cleared");
      sendJson(response, 200, { status: state.status });
      return;
    }
  }
}

/** Copies the SDK wallet state into stable JSON fields consumed by Rust assertions. */
function applyWallet(wallet: Wallet | null): void {
  if (!wallet) {
    state.status = "disconnected";
    state.account = null;
    state.device = null;
    return;
  }

  state.status = "connected";
  state.account = {
    address: wallet.account.address,
    chain: wallet.account.chain,
    publicKey: wallet.account.publicKey ?? null,
    walletStateInit: wallet.account.walletStateInit
  };
  state.device = wallet.device;
  state.error = null;
}

/** Appends a timestamped observation to the actor journal without mutating its details. */
function record(type: string, details?: unknown): void {
  const event: ActorEvent = details === undefined
    ? { at: Date.now(), type }
    : { at: Date.now(), type, details };
  state.journal.push(event);
}

/** Reads and validates a bounded JSON command body from the local HTTP client. */
async function readCommand(request: IncomingMessage): Promise<ActorCommand> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    size += buffer.length;
    if (size > 1024 * 1024) {
      throw new Error("command body is too large");
    }
    chunks.push(buffer);
  }

  const value: unknown = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  if (!isActorCommand(value)) {
    throw new Error("invalid actor command");
  }
  return value;
}

/** Narrows untrusted JSON to the actor command discriminated union. */
function isActorCommand(value: unknown): value is ActorCommand {
  if (typeof value !== "object" || value === null || !("type" in value)) {
    return false;
  }
  const type = value.type;
  if (type === "send_transaction") {
    return "transaction" in value
      && typeof value.transaction === "object"
      && value.transaction !== null;
  }
  return type === "connect"
    || type === "restore"
    || type === "disconnect"
    || type === "clear_storage";
}

/** Sends one complete JSON response unless another branch has already committed headers. */
function sendJson(response: ServerResponse, status: number, value: unknown): void {
  if (response.headersSent) {
    return;
  }
  const body = JSON.stringify(value);
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(body)
  });
  response.end(body);
}

/** Serves the deterministic one-pixel PNG referenced by the local dApp manifest. */
function sendPng(response: ServerResponse): void {
  const body = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
    "base64"
  );
  response.writeHead(200, {
    "content-type": "image/png",
    "content-length": body.length
  });
  response.end(body);
}

/** Parses the required process configuration and rejects incomplete actor fixtures. */
function parseConfig(value: string | undefined): DappActorConfig {
  if (value === undefined) {
    throw new Error("TON_CONNECT_DAPP_CONFIG is required");
  }
  const parsed: unknown = JSON.parse(value);
  if (!isDappActorConfig(parsed)) {
    throw new Error("TON_CONNECT_DAPP_CONFIG is invalid");
  }
  return parsed;
}

/** Validates every field needed to construct the SDK and serve the dApp manifest. */
function isDappActorConfig(value: unknown): value is DappActorConfig {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  const manifest = candidate.manifest;
  if (typeof manifest !== "object" || manifest === null) {
    return false;
  }
  const manifestRecord = manifest as Record<string, unknown>;
  return typeof candidate.bridgeUrl === "string"
    && typeof candidate.manifestUrl === "string"
    && typeof candidate.universalLink === "string"
    && (candidate.inNetwork === undefined || typeof candidate.inNetwork === "string")
    && typeof manifestRecord.url === "string"
    && typeof manifestRecord.name === "string"
    && typeof manifestRecord.iconUrl === "string";
}

/** Reads one mandatory environment variable and reports its exact missing name. */
function requiredEnvironment(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

/** Serializes JavaScript errors, including structured causes, for cross-process assertions. */
function actorError(error: unknown): ActorError {
  if (!(error instanceof Error)) {
    return { name: "Error", message: String(error) };
  }
  const name = error.name === "Error" ? error.constructor.name : error.name;
  return error.cause === undefined
    ? { name, message: error.message }
    : { name, message: error.message, cause: error.cause };
}

/** Parses a TCP port or returns the supplied local-development fallback. */
function parsePort(value: string | undefined, fallback: number): number {
  if (value === undefined) {
    return fallback;
  }
  const parsed = Number.parseInt(value, 10);
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
    throw new Error(`invalid PORT: ${value}`);
  }
  return parsed;
}
