import {PaperPlaneTilt, X} from "@phosphor-icons/react"
import type {SendResult} from "@ton/wallet-engine"
import {type ChangeEvent, type ReactElement, type SyntheticEvent, useId, useState} from "react"

import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert"
import {Button} from "@/components/ui/button"
import {Card, CardContent, CardHeader, CardTitle} from "@/components/ui/card"
import {Input} from "@/components/ui/input"
import {gramsToNanograms} from "@/lib/format"

export interface SendFormProps {
  readonly onClose: () => void
  readonly onSend: (destination: string, amountNanograms: string) => Promise<SendResult>
}

export function SendForm({onClose, onSend}: SendFormProps): ReactElement {
  const destinationId: string = useId()
  const amountId: string = useId()
  const [destination, setDestination] = useState<string>("")
  const [amount, setAmount] = useState<string>("")
  const [sending, setSending] = useState<boolean>(false)
  const [error, setError] = useState<string>()

  async function submit(event: SyntheticEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault()
    setSending(true)
    setError(undefined)
    try {
      const result: SendResult = await onSend(destination.trim(), gramsToNanograms(amount))
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
      setError(cause instanceof Error ? cause.message : "The transfer failed")
    } finally {
      setSending(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/35 p-3 sm:items-center sm:p-5">
      <Card className="max-h-[calc(100dvh-1.5rem)] w-full max-w-md overflow-y-auto bg-background shadow-none sm:max-h-[calc(100dvh-2.5rem)]">
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle>Send GRAM</CardTitle>
          <Button aria-label="Close send form" size="icon" variant="ghost" onClick={onClose}>
            <X aria-hidden="true" size={18} />
          </Button>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={submit}>
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
                  onChange={(event: ChangeEvent<HTMLInputElement>) => setAmount(event.target.value)}
                />
                <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-xs font-medium text-muted-foreground">
                  GRAM
                </span>
              </div>
            </label>

            {error ? (
              <Alert>
                <AlertTitle>Transfer not completed</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            ) : null}
            <Button className="w-full" disabled={sending} type="submit">
              <PaperPlaneTilt aria-hidden="true" size={18} />
              {sending ? "Sending" : "Send"}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
