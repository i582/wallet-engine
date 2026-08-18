import {canonicalRequestId, isUnsignedDecimal} from "./ton-connect-protocol"
import type {SendRequest, SignMessageRequest} from "./send-types"
import type {
  AppRequest,
  RawMessage,
  RawTransactionPayload,
  TonConnectAccountInfo,
  TonConnectInteraction,
} from "./ton-connect-types"
import type {WalletDescriptor} from "./types"

export type PreparedTransaction =
  | {
      readonly ok: true
      readonly interaction: Omit<
        Extract<TonConnectInteraction, {readonly kind: "transaction"}>,
        "preview"
      >
      readonly method: "sendTransaction" | "signMessage"
      readonly walletRequest: SendRequest | SignMessageRequest
    }
  | {
      readonly ok: false
      readonly code: number
      readonly message: string
    }

export interface PrepareTransactionOptions {
  readonly request: AppRequest
  readonly account: TonConnectAccountInfo
  readonly descriptor: WalletDescriptor
  readonly sessionId: string
  readonly dappName: string
}

export function prepareTransaction(options: PrepareTransactionOptions): PreparedTransaction {
  const {request, account, descriptor, sessionId, dappName} = options
  if (request.method !== "sendTransaction" && request.method !== "signMessage") {
    return {ok: false, code: 400, message: "Method is not supported"}
  }
  if (request.params.length !== 1) {
    return {ok: false, code: 1, message: "Malformed request"}
  }
  let payload: RawTransactionPayload
  try {
    payload = JSON.parse(request.params[0] ?? "") as RawTransactionPayload
  } catch {
    return {ok: false, code: 1, message: "Malformed request"}
  }
  if (
    payload.messages === undefined ||
    payload.messages.length === 0 ||
    payload.messages.length > 255 ||
    payload.items !== undefined
  ) {
    return {ok: false, code: 400, message: "Unsupported transaction shape"}
  }
  if ("validUntil" in payload) {
    return {ok: false, code: 1, message: "Malformed transaction"}
  }
  const messages: readonly RawMessage[] = payload.messages
  if (messages.some(message => !isValidMessage(message, payload, account, descriptor))) {
    return {ok: false, code: 1, message: "Malformed transaction"}
  }
  const first: RawMessage = messages[0] as RawMessage
  return {
    ok: true,
    method: request.method,
    interaction: {
      kind: "transaction",
      id: crypto.randomUUID(),
      dappName,
      method: request.method,
      destination: first.address,
      amountNanograms: first.amount,
      messageCount: messages.length,
      deploysContract: messages.some(message => message.stateInit !== undefined),
      hasPayload: messages.some(message => message.payload !== undefined),
    },
    walletRequest: {
      operationId: `ton-connect:${sessionId}:${canonicalRequestId(request.id)}`,
      intent: {
        expiration:
          payload.valid_until === undefined
            ? {kind: "engineDefault"}
            : {kind: "exact", unixTimestamp: payload.valid_until},
        messages: messages.map(message => ({
          destination: message.address,
          amount: {kind: "exact" as const, nanograms: message.amount},
          body:
            message.payload === undefined
              ? ({kind: "empty"} as const)
              : ({kind: "rawPayload", boc: message.payload} as const),
          stateInit: message.stateInit,
        })),
      },
    },
  }
}

function isValidMessage(
  message: RawMessage | undefined,
  payload: RawTransactionPayload,
  account: TonConnectAccountInfo,
  descriptor: WalletDescriptor,
): message is RawMessage {
  if (!message || !isUnsignedDecimal(message.amount) || typeof message.address !== "string") {
    return false
  }
  if (message.extra_currency !== undefined) {
    return false
  }
  if ("extraCurrency" in message) {
    return false
  }
  if (payload.network !== undefined && payload.network !== account.network) {
    return false
  }
  return (
    payload.from === undefined ||
    payload.from === account.address ||
    payload.from === descriptor.address
  )
}
