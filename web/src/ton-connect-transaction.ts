import {canonicalRequestId, isUnsignedDecimal} from "./ton-connect-protocol"
import type {
  AppRequest,
  RawMessage,
  RawTransactionPayload,
  TonConnectAccountInfo,
  TonConnectInteraction,
} from "./ton-connect-types"
import type {SendRequest, WalletDescriptor} from "./types"

export type PreparedTransaction =
  | {
      readonly ok: true
      readonly interaction: Omit<
        Extract<TonConnectInteraction, {readonly kind: "transaction"}>,
        "preview"
      >
      readonly sendRequest: SendRequest
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
  if (request.params.length !== 1) {
    return {ok: false, code: 1, message: "Malformed request"}
  }
  let payload: RawTransactionPayload
  try {
    payload = JSON.parse(request.params[0] ?? "") as RawTransactionPayload
  } catch {
    return {ok: false, code: 1, message: "Malformed request"}
  }
  if (payload.messages?.length !== 1 || payload.items !== undefined) {
    return {ok: false, code: 400, message: "Unsupported transaction shape"}
  }
  const message: RawMessage | undefined = payload.messages[0]
  if (!isValidMessage(message, payload, account, descriptor)) {
    return {ok: false, code: 1, message: "Malformed transaction"}
  }
  return {
    ok: true,
    interaction: {
      kind: "transaction",
      id: crypto.randomUUID(),
      dappName,
      destination: message.address,
      amountNanograms: message.amount,
      deploysContract: message.stateInit !== undefined,
      hasPayload: message.payload !== undefined,
    },
    sendRequest: {
      operationId: `ton-connect:${sessionId}:${canonicalRequestId(request.id)}`,
      destination: message.address,
      amount: {kind: "exact", nanograms: message.amount},
      validUntil: payload.valid_until ?? payload.validUntil,
      payload: message.payload,
      stateInit: message.stateInit,
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
  if (payload.network !== undefined && payload.network !== account.network) {
    return false
  }
  return (
    payload.from === undefined ||
    payload.from === account.address ||
    payload.from === descriptor.address
  )
}
