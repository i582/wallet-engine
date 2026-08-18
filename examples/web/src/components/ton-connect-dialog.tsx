import type {TonConnectInteraction} from "@ton/wallet-engine"
import {Warning} from "@phosphor-icons/react"
import {type ChangeEvent, type ReactElement, useId, useState} from "react"

import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert"
import {Button} from "@/components/ui/button"
import {Input} from "@/components/ui/input"
import {TransferPreview} from "@/components/transfer-preview"
import {compactAddress, formatNanogramBalance} from "@/lib/format"

export interface TonConnectDialogProps {
  readonly canForceRetry: boolean
  readonly connectedDappName?: string
  readonly interaction?: TonConnectInteraction
  readonly onClose: () => void
  readonly onDisconnect: () => Promise<void>
  readonly onRespond: (interactionId: string, approved: boolean, force: boolean) => void
  readonly onStart: (link: string) => Promise<void>
}

export function TonConnectDialog({
  canForceRetry,
  connectedDappName,
  interaction,
  onClose,
  onDisconnect,
  onRespond,
  onStart,
}: TonConnectDialogProps): ReactElement {
  const [link, setLink] = useState<string>("")
  const [busy, setBusy] = useState<boolean>(false)
  const [error, setError] = useState<string>()
  const [force, setForce] = useState<boolean>(false)
  const titleId: string = useId()
  const forceId: string = useId()

  async function connect(): Promise<void> {
    const value: string = link.trim()
    if (!value) {
      setError("Paste a TON Connect link")
      return
    }
    setBusy(true)
    setError(undefined)
    try {
      await onStart(value)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "TON Connect failed")
    } finally {
      setBusy(false)
    }
  }

  function respond(approved: boolean): void {
    if (!interaction) {
      return
    }
    onRespond(interaction.id, approved, approved && force)
  }

  async function disconnect(): Promise<void> {
    setBusy(true)
    setError(undefined)
    try {
      await onDisconnect()
      onClose()
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not disconnect the dApp")
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/40 p-3 sm:items-center sm:p-5">
      <div
        aria-labelledby={titleId}
        aria-modal="true"
        className="w-full max-w-md rounded-2xl border border-border bg-background p-5"
        role="dialog"
      >
        {interaction ? <ApprovalContent interaction={interaction} titleId={titleId} /> : null}
        {interaction?.kind === "transaction" && canForceRetry ? (
          <Alert className="mt-4">
            <Warning aria-hidden="true" className="mt-0.5 text-amber-500" size={19} />
            <AlertTitle>Previous transfer is unresolved</AlertTitle>
            <AlertDescription>
              Its signed message may still execute. If you approve this request, both transfers can
              affect the balance.
              <label className="mt-3 flex cursor-pointer items-start gap-2" htmlFor={forceId}>
                <input
                  checked={force}
                  className="mt-0.5 size-4 accent-current"
                  disabled={busy}
                  id={forceId}
                  type="checkbox"
                  onChange={(event: ChangeEvent<HTMLInputElement>) =>
                    setForce(event.target.checked)
                  }
                />
                <span>I understand. Approve this transaction anyway.</span>
              </label>
            </AlertDescription>
          </Alert>
        ) : null}
        {!interaction && connectedDappName ? (
          <>
            <p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
              TON Connect
            </p>
            <h2 className="mt-2 text-xl font-semibold tracking-[-0.03em]" id={titleId}>
              {connectedDappName}
            </h2>
            <p className="mt-2 text-sm leading-5 text-muted-foreground">
              This app is connected to your wallet.
            </p>
          </>
        ) : null}
        {!interaction && !connectedDappName ? (
          <>
            <p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
              Connect an app
            </p>
            <h2 className="mt-2 text-xl font-semibold tracking-[-0.03em]" id={titleId}>
              Paste a connection link
            </h2>
            <p className="mt-2 text-sm leading-5 text-muted-foreground">
              Copy the link from the app you want to use with this wallet.
            </p>
            <Input
              aria-label="TON Connect link"
              autoFocus
              className="mt-5"
              placeholder="tc://?v=2&id=…"
              value={link}
              onChange={event => setLink(event.target.value)}
            />
          </>
        ) : null}
        {error ? <p className="mt-3 text-sm text-destructive">{error}</p> : null}

        <div className="mt-6 grid grid-cols-2 gap-3">
          {interaction ? (
            <>
              <Button disabled={busy} variant="secondary" onClick={() => respond(false)}>
                Cancel
              </Button>
              <Button
                disabled={busy || (interaction.kind === "transaction" && canForceRetry && !force)}
                onClick={() => respond(true)}
              >
                {interaction.kind === "connect" ? "Connect" : "Confirm"}
              </Button>
            </>
          ) : connectedDappName ? (
            <>
              <Button disabled={busy} variant="secondary" onClick={onClose}>
                Close
              </Button>
              <Button disabled={busy} variant="destructive" onClick={disconnect}>
                {busy ? "Disconnecting" : "Disconnect"}
              </Button>
            </>
          ) : (
            <>
              <Button disabled={busy} variant="secondary" onClick={onClose}>
                Close
              </Button>
              <Button disabled={busy} onClick={connect}>
                {busy ? "Connecting" : "Continue"}
              </Button>
            </>
          )}
        </div>
      </div>
    </div>
  )
}

function ApprovalContent({
  interaction,
  titleId,
}: {
  readonly interaction: TonConnectInteraction
  readonly titleId: string
}): ReactElement {
  if (interaction.kind === "connect") {
    return (
      <>
        <p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
          TON Connect
        </p>
        <div className="mt-4 flex items-center gap-4">
          <img
            alt=""
            className="size-12 rounded-xl bg-secondary object-cover"
            height={48}
            src={interaction.iconUrl}
            width={48}
          />
          <div className="min-w-0">
            <h2 className="truncate text-xl font-semibold tracking-[-0.03em]" id={titleId}>
              {interaction.dappName}
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">wants to connect to your wallet</p>
          </div>
        </div>
        <div className="mt-5 rounded-xl bg-secondary px-4 py-4">
          <p className="text-sm font-medium">This app can</p>
          <ul className="mt-2 space-y-1.5 text-sm leading-5 text-muted-foreground">
            <li>View your wallet address</li>
            <li>Ask you to approve transactions</li>
            {interaction.proofPayload ? <li>Verify that you own this wallet</li> : null}
          </ul>
        </div>
        <details className="mt-4 text-xs text-muted-foreground">
          <summary className="cursor-pointer select-none font-medium text-foreground">
            Details
          </summary>
          <div className="mt-3 space-y-2 rounded-xl border border-border px-3 py-3">
            <p className="break-all">{interaction.origin}</p>
            <p className="font-mono">{compactAddress(interaction.account)}</p>
          </div>
        </details>
      </>
    )
  }

  return (
    <>
      <p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
        Review transaction
      </p>
      <h2 className="mt-2 text-xl font-semibold tracking-[-0.03em]" id={titleId}>
        {interaction.dappName} wants to send
      </h2>
      <div className="mt-5">
        <TransferPreview
          amount={formatNanogramBalance(interaction.amountNanograms)}
          destination={interaction.destination}
          preview={interaction.preview}
        />
      </div>
      {interaction.hasPayload || interaction.deploysContract ? (
        <details className="mt-4 text-xs text-muted-foreground">
          <summary className="cursor-pointer select-none font-medium text-foreground">
            Technical details
          </summary>
          <div className="mt-3 rounded-xl border border-border px-3 py-3">
            {interaction.deploysContract ? <p>Deploys a contract</p> : null}
            {interaction.hasPayload ? <p>Includes a contract payload</p> : null}
          </div>
        </details>
      ) : null}
    </>
  )
}
