import {
  ArrowLeft,
  Check,
  CheckCircle,
  Copy,
  PaperPlaneTilt,
  Warning,
  X,
} from "@phosphor-icons/react"
import type {SendPreview, SendResult} from "@ton/wallet-engine"
import {type ChangeEvent, type ReactElement, type SyntheticEvent, useId, useState} from "react"

import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert"
import {Button} from "@/components/ui/button"
import {Card, CardContent, CardHeader, CardTitle} from "@/components/ui/card"
import {Input} from "@/components/ui/input"
import {compactAddress, formatNanogramBalance, gramsToNanograms} from "@/lib/format"

export interface SendFormProps {
  readonly onClose: () => void
  readonly onPreview: (destination: string, amountNanograms: string) => Promise<SendPreview>
  readonly onCancelPreview: () => Promise<void>
  readonly onSend: (destination: string, amountNanograms: string) => Promise<SendResult>
}

type SendProgress = "idle" | "previewing" | "sending"

export function SendForm({
  onClose,
  onPreview,
  onCancelPreview,
  onSend,
}: SendFormProps): ReactElement {
  const destinationId: string = useId()
  const amountId: string = useId()
  const [destination, setDestination] = useState<string>("")
  const [amount, setAmount] = useState<string>("")
  const [progress, setProgress] = useState<SendProgress>("idle")
  const [preview, setPreview] = useState<SendPreview>()
  const [previewUnavailable, setPreviewUnavailable] = useState<boolean>(false)
  const [error, setError] = useState<string>()

  const reviewing: boolean = preview !== undefined || previewUnavailable
  const busy: boolean = progress !== "idle"

  async function submit(event: SyntheticEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault()
    const normalizedDestination: string = destination.trim()
    const amountNanograms: string = gramsToNanograms(amount)
    setError(undefined)

    if (!reviewing) {
      await previewTransfer(normalizedDestination, amountNanograms)
      return
    }

    await sendTransfer(normalizedDestination, amountNanograms)
  }

  async function previewTransfer(
    normalizedDestination: string,
    amountNanograms: string,
  ): Promise<void> {
    setProgress("previewing")
    try {
      setPreview(await onPreview(normalizedDestination, amountNanograms))
    } catch (cause) {
      setPreviewUnavailable(true)
      setError(errorMessage(cause, "Preview is unavailable"))
    } finally {
      setProgress("idle")
    }
  }

  async function sendTransfer(
    normalizedDestination: string,
    amountNanograms: string,
  ): Promise<void> {
    setProgress("sending")
    try {
      const result: SendResult = await onSend(normalizedDestination, amountNanograms)
      if (result.phase === "submitted") {
        onClose()
        return
      }
      if (result.phase === "submissionUnknown") {
        setError("The result is unknown. Do not submit the transfer again.")
        return
      }
      setError(`The transfer ended with status: ${result.phase}`)
    } catch (cause) {
      setError(errorMessage(cause, "The transfer failed"))
    } finally {
      setProgress("idle")
    }
  }

  async function backToEdit(): Promise<void> {
    await onCancelPreview()
    setPreview(undefined)
    setPreviewUnavailable(false)
    setError(undefined)
  }

  async function close(): Promise<void> {
    await onCancelPreview()
    onClose()
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/55 p-3 sm:items-center sm:p-5">
      <Card className="max-h-[calc(100dvh-1.5rem)] w-full max-w-md overflow-y-auto bg-background shadow-none sm:max-h-[calc(100dvh-2.5rem)]">
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>{reviewing ? "Review transfer" : "Send GRAM"}</CardTitle>
          <Button
            aria-label="Close send form"
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
            {reviewing ? (
              <PreviewSummary amount={amount} destination={destination.trim()} preview={preview} />
            ) : (
              <>
                <label className="block text-sm font-medium" htmlFor={destinationId}>
                  Recipient
                  <Input
                    required
                    autoComplete="off"
                    className="mt-2"
                    id={destinationId}
                    placeholder="EQ… or 0:…"
                    value={destination}
                    onChange={(event: ChangeEvent<HTMLInputElement>) =>
                      setDestination(event.target.value)
                    }
                  />
                </label>
                <label className="block text-sm font-medium" htmlFor={amountId}>
                  Amount
                  <div className="relative mt-2">
                    <Input
                      required
                      className="pr-16"
                      id={amountId}
                      inputMode="decimal"
                      placeholder="0.00"
                      value={amount}
                      onChange={(event: ChangeEvent<HTMLInputElement>) =>
                        setAmount(event.target.value)
                      }
                    />
                    <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-xs font-medium text-muted-foreground">
                      GRAM
                    </span>
                  </div>
                </label>
              </>
            )}

            <TransferAlert error={error} previewUnavailable={previewUnavailable} />

            {reviewing ? (
              <div className="grid grid-cols-[auto_1fr] gap-2">
                <Button disabled={busy} type="button" variant="outline" onClick={backToEdit}>
                  <ArrowLeft aria-hidden="true" size={18} />
                  Back
                </Button>
                <Button disabled={busy} type="submit">
                  <PaperPlaneTilt aria-hidden="true" size={18} />
                  {sendButtonLabel(progress, previewUnavailable)}
                </Button>
              </div>
            ) : (
              <Button className="w-full" disabled={busy} type="submit">
                <CheckCircle aria-hidden="true" size={18} />
                {progress === "previewing" ? "Checking" : "Review"}
              </Button>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

interface PreviewSummaryProps {
  readonly amount: string
  readonly destination: string
  readonly preview?: SendPreview
}

function PreviewSummary({amount, destination, preview}: PreviewSummaryProps): ReactElement {
  const warning: boolean =
    preview !== undefined && (!preview.emulation.traceSucceeded || preview.emulation.isIncomplete)
  const simpleTransfer: boolean = hasSimpleTransferActionSet(preview)

  return (
    <div className="space-y-3">
      {simpleTransfer ? <TransferHero destination={destination} /> : null}

      <div className="overflow-hidden rounded-xl border border-border">
        <SummaryRow label="Send" value={`${amount} GRAM`} />
        <SummaryRow label="To" value={compactAddress(destination)} />
        {preview ? (
          <>
            <SummaryRow
              label="Network fee"
              value={`${formatNanogramBalance(preview.emulation.walletFeesNanograms)} GRAM`}
            />
            <SummaryRow
              label="Transactions"
              value={preview.emulation.transactionCount.toString()}
            />
            <BocSummaryRow value={preview.messageBocBase64} />
          </>
        ) : null}
      </div>

      {!simpleTransfer && preview !== undefined && preview.emulation.actions.length > 0 ? (
        <div className="rounded-xl border border-border px-4 py-3">
          <p className="text-xs font-medium text-muted-foreground">Actions</p>
          <div className="mt-2 flex flex-wrap gap-2">
            {preview.emulation.actions.map(action => (
              <span
                className={action.succeeded ? "text-foreground" : "text-amber-500"}
                key={action.actionId}
              >
                {humanizeAction(action.kind)}
              </span>
            ))}
          </div>
        </div>
      ) : null}

      {warning ? (
        <Alert>
          <Warning aria-hidden="true" className="mt-0.5 text-amber-500" size={19} />
          <AlertTitle>Some actions may fail</AlertTitle>
          <AlertDescription>The network can still accept the wallet transaction.</AlertDescription>
        </Alert>
      ) : null}
    </div>
  )
}

interface TransferHeroProps {
  readonly destination: string
}

function TransferHero({destination}: TransferHeroProps): ReactElement {
  return (
    <div className="flex flex-col items-center px-4 pb-3 pt-1 text-center">
      <div className="flex size-16 items-center justify-center rounded-full bg-primary text-primary-foreground">
        <PaperPlaneTilt aria-hidden="true" size={30} weight="fill" />
      </div>
      <p className="mt-4 text-sm text-muted-foreground">Confirm sending</p>
      <p className="mt-1 max-w-full truncate text-xl font-semibold tracking-[-0.025em]">
        {compactAddress(destination)}
      </p>
    </div>
  )
}

function hasSimpleTransferActionSet(preview: SendPreview | undefined): boolean {
  if (preview === undefined) {
    return false
  }

  const actionKinds: readonly string[] = preview.emulation.actions.map(action => action.kind)
  const transferCount: number = actionKinds.filter(kind => kind === "ton_transfer").length

  return (
    transferCount === 1 &&
    actionKinds.every(kind => kind === "ton_transfer" || kind === "contract_deploy")
  )
}

interface BocSummaryRowProps {
  readonly value: string
}

function BocSummaryRow({value}: BocSummaryRowProps): ReactElement {
  const [copied, setCopied] = useState<boolean>(false)

  async function copyBoc(): Promise<void> {
    await navigator.clipboard.writeText(value)
    setCopied(true)
    globalThis.setTimeout(() => setCopied(false), 1600)
  }

  return (
    <div className="flex min-w-0 items-center gap-3 px-4 py-3">
      <span className="shrink-0 text-sm text-muted-foreground">Message BOC</span>
      <span className="min-w-0 flex-1 truncate text-right text-sm font-medium">
        {compactBoc(value)}
      </span>
      <Button
        aria-label={copied ? "Message BOC copied" : "Copy message BOC"}
        className="size-7 shrink-0"
        size="icon"
        title={copied ? "Copied" : "Copy message BOC"}
        type="button"
        variant="ghost"
        onClick={copyBoc}
      >
        {copied ? <Check aria-hidden="true" size={16} /> : <Copy aria-hidden="true" size={16} />}
      </Button>
    </div>
  )
}

function compactBoc(value: string): string {
  if (value.length <= 24) {
    return value
  }
  return `${value.slice(0, 12)}…${value.slice(-8)}`
}

interface SummaryRowProps {
  readonly label: string
  readonly value: string
}

function SummaryRow({label, value}: SummaryRowProps): ReactElement {
  return (
    <div className="flex items-center justify-between gap-5 border-b border-border px-4 py-3 last:border-b-0">
      <span className="text-sm text-muted-foreground">{label}</span>
      <span className="truncate text-sm font-medium">{value}</span>
    </div>
  )
}

function humanizeAction(kind: string): string {
  return kind.replaceAll("_", " ")
}

interface TransferAlertProps {
  readonly error?: string
  readonly previewUnavailable: boolean
}

function TransferAlert({error, previewUnavailable}: TransferAlertProps): ReactElement | null {
  if (previewUnavailable) {
    return (
      <Alert>
        <Warning aria-hidden="true" className="mt-0.5 text-amber-500" size={19} />
        <AlertTitle>Preview unavailable</AlertTitle>
        <AlertDescription>
          {error ?? "The transfer can still be sent with fresh chain state."}
        </AlertDescription>
      </Alert>
    )
  }

  if (error) {
    return (
      <Alert>
        <AlertTitle>Transfer not completed</AlertTitle>
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    )
  }

  return null
}

function sendButtonLabel(progress: SendProgress, previewUnavailable: boolean): string {
  if (progress === "sending") {
    return "Sending"
  }
  return previewUnavailable ? "Send anyway" : "Send"
}

function errorMessage(cause: unknown, fallback: string): string {
  return cause instanceof Error ? cause.message : fallback
}
