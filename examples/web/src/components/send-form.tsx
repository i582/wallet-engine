import {ArrowLeft, CheckCircle, PaperPlaneTilt, Warning, X} from "@phosphor-icons/react"
import type {SendPreview, SendResult} from "@ton/wallet-engine"
import {type ChangeEvent, type ReactElement, type SyntheticEvent, useId, useState} from "react"

import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert"
import {TransferPreview} from "@/components/transfer-preview"
import {Button} from "@/components/ui/button"
import {Card, CardContent, CardHeader, CardTitle} from "@/components/ui/card"
import {Input} from "@/components/ui/input"
import {gramsToNanograms} from "@/lib/format"

export interface SendFormProps {
  readonly canForceRetry: boolean
  readonly onClose: () => void
  readonly onPreview: (destination: string, amountNanograms: string) => Promise<SendPreview>
  readonly onCancelPreview: () => Promise<void>
  readonly onSend: (
    destination: string,
    amountNanograms: string,
    force: boolean,
  ) => Promise<SendResult>
}

type SendProgress = "idle" | "previewing" | "sending"

export function SendForm({
  canForceRetry,
  onClose,
  onPreview,
  onCancelPreview,
  onSend,
}: SendFormProps): ReactElement {
  const destinationId: string = useId()
  const amountId: string = useId()
  const forceId: string = useId()
  const [destination, setDestination] = useState<string>("")
  const [amount, setAmount] = useState<string>("")
  const [progress, setProgress] = useState<SendProgress>("idle")
  const [preview, setPreview] = useState<SendPreview>()
  const [previewUnavailable, setPreviewUnavailable] = useState<boolean>(false)
  const [force, setForce] = useState<boolean>(false)
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
      const result: SendResult = await onSend(normalizedDestination, amountNanograms, force)
      if (result.phase === "submitted") {
        onClose()
        return
      }
      if (result.phase === "submissionUnknown") {
        setError("The transfer may have been submitted. Review the warning before sending again.")
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
    setForce(false)
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
              <TransferPreview amount={amount} destination={destination.trim()} preview={preview} />
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
                    onChange={(event: ChangeEvent<HTMLInputElement>) => {
                      setForce(false)
                      setDestination(event.target.value)
                    }}
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
                      onChange={(event: ChangeEvent<HTMLInputElement>) => {
                        setForce(false)
                        setAmount(event.target.value)
                      }}
                    />
                    <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-xs font-medium text-muted-foreground">
                      GRAM
                    </span>
                  </div>
                </label>
              </>
            )}

            <TransferAlert error={error} previewUnavailable={previewUnavailable} />

            {canForceRetry ? (
              <Alert>
                <Warning aria-hidden="true" className="mt-0.5 text-amber-500" size={19} />
                <AlertTitle>Previous transfer is unresolved</AlertTitle>
                <AlertDescription>
                  Its signed message may still execute. If you send another transfer, both can
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
                    <span>I understand. Submit this transfer anyway.</span>
                  </label>
                </AlertDescription>
              </Alert>
            ) : null}

            {reviewing ? (
              <div className="grid grid-cols-[auto_1fr] gap-2">
                <Button disabled={busy} type="button" variant="outline" onClick={backToEdit}>
                  <ArrowLeft aria-hidden="true" size={18} />
                  Back
                </Button>
                <Button disabled={busy || (canForceRetry && !force)} type="submit">
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
