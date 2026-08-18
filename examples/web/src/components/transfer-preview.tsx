import {Check, Copy, PaperPlaneTilt, Warning} from "@phosphor-icons/react"
import type {SendPreview} from "@ton/wallet-engine"
import {type ReactElement, useState} from "react"

import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert"
import {Button} from "@/components/ui/button"
import {compactAddress, formatNanogramBalance} from "@/lib/format"

export interface TransferPreviewProps {
  readonly amount: string
  readonly destination: string
  readonly preview?: SendPreview
}

/** Renders the same emulation evidence for direct and TON Connect transfers. */
export function TransferPreview({
  amount,
  destination,
  preview,
}: TransferPreviewProps): ReactElement {
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

function TransferHero({destination}: {readonly destination: string}): ReactElement {
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

function BocSummaryRow({value}: {readonly value: string}): ReactElement {
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

function humanizeAction(kind: string): string {
  return kind.replaceAll("_", " ")
}
