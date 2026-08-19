import {ArrowLeft, CheckCircle, PaperPlaneTilt, Warning, X} from "@phosphor-icons/react"
import type {NftItem, SendPreview, SendResult} from "@ton/wallet-engine"
import {type ChangeEvent, type ReactElement, type SyntheticEvent, useId, useState} from "react"

import {NftArtwork} from "@/components/nft-card"
import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert"
import {Button} from "@/components/ui/button"
import {Card, CardContent, CardHeader, CardTitle} from "@/components/ui/card"
import {Input} from "@/components/ui/input"
import {compactAddress, formatNanogramBalance} from "@/lib/format"
import {nftDisplayName} from "@/lib/nft-display"
import {NFT_TRANSFER_FUNDING} from "@/lib/nft-transfer-policy"

export interface NftSendFormProps {
  readonly item: NftItem
  readonly canForceRetry: boolean
  readonly onClose: () => void
  readonly onCancelPreview: () => Promise<void>
  readonly onPreview: (
    operationId: string,
    nftAddress: string,
    recipient: string,
  ) => Promise<SendPreview>
  readonly onSend: (
    operationId: string,
    nftAddress: string,
    recipient: string,
    force: boolean,
  ) => Promise<SendResult>
}

type Progress = "idle" | "previewing" | "sending"

export function NftSendForm({
  item,
  canForceRetry,
  onClose,
  onCancelPreview,
  onPreview,
  onSend,
}: NftSendFormProps): ReactElement {
  const recipientId: string = useId()
  const forceId: string = useId()
  const [operationId] = useState<string>(() => crypto.randomUUID())
  const [recipient, setRecipient] = useState<string>("")
  const [preview, setPreview] = useState<SendPreview>()
  const [progress, setProgress] = useState<Progress>("idle")
  const [force, setForce] = useState<boolean>(false)
  const [error, setError] = useState<string>()
  const busy: boolean = progress !== "idle"
  const reviewing: boolean = preview !== undefined

  async function submit(event: SyntheticEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault()
    const normalizedRecipient: string = recipient.trim()
    setError(undefined)

    if (!reviewing) {
      setProgress("previewing")
      try {
        setPreview(await onPreview(operationId, item.address, normalizedRecipient))
      } catch (cause) {
        setError(errorMessage(cause, "NFT transfer preview failed"))
      } finally {
        setProgress("idle")
      }
      return
    }

    setProgress("sending")
    try {
      const result: SendResult = await onSend(operationId, item.address, normalizedRecipient, force)
      if (result.phase === "submitted") {
        onClose()
      } else if (result.phase === "submissionUnknown") {
        setError("Submission is unresolved. Do not retry until wallet status is refreshed.")
      } else {
        setError(`NFT transfer ended with status: ${result.phase}`)
      }
    } catch (cause) {
      setError(errorMessage(cause, "NFT transfer failed"))
    } finally {
      setProgress("idle")
    }
  }

  async function editRecipient(): Promise<void> {
    await onCancelPreview()
    setPreview(undefined)
    setForce(false)
    setError(undefined)
  }

  async function close(): Promise<void> {
    await onCancelPreview()
    onClose()
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/60 p-3 backdrop-blur-[2px] sm:items-center sm:p-5">
      <Card className="max-h-[calc(100dvh-1.5rem)] w-full max-w-md overflow-y-auto bg-background shadow-none sm:max-h-[calc(100dvh-2.5rem)]">
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>{reviewing ? "Review NFT transfer" : "Send collectible"}</CardTitle>
          <Button
            aria-label="Close NFT send form"
            disabled={busy}
            size="icon"
            type="button"
            variant="ghost"
            onClick={close}
          >
            <X aria-hidden="true" size={18} />
          </Button>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={submit}>
            <div className="flex items-center gap-3 rounded-2xl border border-border bg-secondary/35 p-3">
              <div className="size-16 shrink-0 overflow-hidden rounded-xl border border-border bg-secondary">
                <NftArtwork item={item} name={nftDisplayName(item)} />
              </div>
              <div className="min-w-0">
                <p className="truncate font-semibold">{nftDisplayName(item)}</p>
                <p className="mt-1 truncate text-xs text-muted-foreground">
                  {compactAddress(item.address)}
                </p>
              </div>
            </div>

            {preview ? (
              <NftTransferSummary preview={preview} recipient={recipient.trim()} />
            ) : (
              <label className="block text-sm font-medium" htmlFor={recipientId}>
                New owner
                <Input
                  required
                  autoComplete="off"
                  className="mt-2"
                  id={recipientId}
                  placeholder="EQ… or 0:…"
                  value={recipient}
                  onChange={(event: ChangeEvent<HTMLInputElement>) => {
                    setForce(false)
                    setRecipient(event.target.value)
                  }}
                />
                <span className="mt-2 block text-xs leading-5 text-muted-foreground">
                  Ownership changes on-chain and cannot be undone by this wallet.
                </span>
              </label>
            )}

            {error ? (
              <Alert>
                <Warning aria-hidden="true" className="mt-0.5 text-amber-500" size={19} />
                <AlertTitle>NFT transfer not ready</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            ) : null}

            {preview && !preview.emulation.traceSucceeded ? (
              <Alert>
                <Warning aria-hidden="true" className="mt-0.5 text-amber-500" size={19} />
                <AlertTitle>NFT transfer action succeeded</AlertTitle>
                <AlertDescription>
                  A follow-up notification or excess message may fail. The item contract still
                  accepted this ownership change in emulation.
                </AlertDescription>
              </Alert>
            ) : null}

            {reviewing && canForceRetry ? (
              <Alert>
                <Warning aria-hidden="true" className="mt-0.5 text-amber-500" size={19} />
                <AlertTitle>Previous transfer is unresolved</AlertTitle>
                <AlertDescription>
                  Both signed messages can execute. Continue only after checking the wallet state.
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
                    <span>I understand. Submit this NFT transfer anyway.</span>
                  </label>
                </AlertDescription>
              </Alert>
            ) : null}

            {reviewing ? (
              <div className="grid grid-cols-[auto_1fr] gap-2">
                <Button disabled={busy} type="button" variant="outline" onClick={editRecipient}>
                  <ArrowLeft aria-hidden="true" size={18} />
                  Back
                </Button>
                <Button disabled={busy || (canForceRetry && !force)} type="submit">
                  <PaperPlaneTilt aria-hidden="true" size={18} />
                  {progress === "sending" ? "Sending" : "Transfer NFT"}
                </Button>
              </div>
            ) : (
              <Button className="w-full" disabled={busy} type="submit">
                <CheckCircle aria-hidden="true" size={18} />
                {progress === "previewing" ? "Checking ownership" : "Review transfer"}
              </Button>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

function NftTransferSummary({
  preview,
  recipient,
}: {
  readonly preview: SendPreview
  readonly recipient: string
}): ReactElement {
  const message = preview.messages[0]
  const attached: string =
    message?.amount.kind === "exact"
      ? `${formatNanogramBalance(message.amount.nanograms)} GRAM`
      : "Unknown"

  return (
    <div className="overflow-hidden rounded-xl border border-border">
      <SummaryRow label="New owner" value={compactAddress(recipient)} />
      <SummaryRow label="Attached to item" value={attached} />
      <SummaryRow
        label="Forwarded to owner"
        value={`${formatNanogramBalance(NFT_TRANSFER_FUNDING.forwardNanograms)} GRAM`}
      />
      <SummaryRow
        label="Network fee"
        value={`${formatNanogramBalance(preview.emulation.walletFeesNanograms)} GRAM`}
      />
      <SummaryRow label="Transactions" value={preview.emulation.transactionCount.toString()} />
    </div>
  )
}

function SummaryRow({
  label,
  value,
}: {
  readonly label: string
  readonly value: string
}): ReactElement {
  return (
    <div className="flex items-center justify-between gap-5 border-b border-border px-4 py-3 last:border-b-0">
      <span className="text-sm text-muted-foreground">{label}</span>
      <span className="truncate text-sm font-medium">{value}</span>
    </div>
  )
}

function errorMessage(cause: unknown, fallback: string): string {
  return cause instanceof Error ? cause.message : fallback
}
