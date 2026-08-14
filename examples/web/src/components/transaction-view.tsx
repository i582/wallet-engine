import {ArrowDownLeft, ArrowLeft, ArrowUpRight, Check, Copy} from "@phosphor-icons/react"
import type {Icon} from "@phosphor-icons/react"
import type {ActivityItem} from "@ton/wallet-engine"
import {type ReactElement, useState} from "react"

import {Button} from "@/components/ui/button"
import {formatActivityAmount, formatUsdNanograms} from "@/lib/format"

export interface TransactionViewProps {
  readonly item: ActivityItem
  readonly gramUsdRate?: number
  readonly onClose: () => void
}

export function TransactionView({item, gramUsdRate, onClose}: TransactionViewProps): ReactElement {
  const [copiedValue, setCopiedValue] = useState<string>()
  const received: boolean = item.direction === "received"
  const Icon: Icon = received ? ArrowDownLeft : ArrowUpRight

  async function copy(value: string): Promise<void> {
    await navigator.clipboard.writeText(value)
    setCopiedValue(value)
    globalThis.setTimeout(() => setCopiedValue(undefined), 1600)
  }

  return (
    <div className="fixed inset-0 z-50 mx-auto flex h-[100dvh] w-full max-w-lg flex-col overflow-hidden bg-background">
      <header className="flex items-center gap-3 border-b border-border px-5 py-4">
        <Button aria-label="Back" size="icon" variant="ghost" onClick={onClose}>
          <ArrowLeft aria-hidden="true" size={20} />
        </Button>
        <h2 className="font-semibold">Transaction</h2>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-8">
        <div className="text-center">
          <span className="mx-auto flex size-14 items-center justify-center rounded-full bg-secondary">
            <Icon aria-hidden="true" size={26} />
          </span>
          <p className="mt-4 text-sm text-muted-foreground">{received ? "Received" : "Sent"}</p>
          <p className="mt-1 text-3xl font-semibold tracking-[-0.04em]">
            {formatActivityAmount(item.amountNanograms, item.direction)}
          </p>
          <p className="mt-2 text-sm text-muted-foreground">
            {formatUsdNanograms(item.amountNanograms, gramUsdRate)}
          </p>
        </div>

        <dl className="mt-10 divide-y divide-border border-y border-border">
          <DetailRow label="Date" value={formatFullTimestamp(item.timestamp)} />
          {item.counterparty ? (
            <CopyableDetailRow
              copied={copiedValue === item.counterparty}
              label={received ? "From" : "To"}
              value={item.counterparty}
              onCopy={copy}
            />
          ) : null}
          <CopyableDetailRow
            copied={copiedValue === item.transactionHash}
            label="Transaction hash"
            value={item.transactionHash}
            onCopy={copy}
          />
          <CopyableDetailRow
            copied={copiedValue === item.logicalTime}
            label="Logical time"
            value={item.logicalTime}
            onCopy={copy}
          />
        </dl>
      </div>
    </div>
  )
}

function DetailRow({label, value}: {readonly label: string; readonly value: string}): ReactElement {
  return (
    <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-4 py-4">
      <dt className="text-sm text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-right text-sm">{value}</dd>
    </div>
  )
}

function CopyableDetailRow({
  label,
  value,
  copied,
  onCopy,
}: {
  readonly label: string
  readonly value: string
  readonly copied: boolean
  readonly onCopy: (value: string) => Promise<void>
}): ReactElement {
  return (
    <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-4 py-4">
      <dt className="text-sm text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-right">
        <button
          className="inline-flex max-w-full items-center gap-2 text-left text-xs"
          type="button"
          onClick={() => onCopy(value)}
        >
          <span className="truncate">{value}</span>
          {copied ? (
            <Check aria-hidden="true" className="shrink-0 text-primary" size={15} />
          ) : (
            <Copy aria-hidden="true" className="shrink-0 text-muted-foreground" size={15} />
          )}
        </button>
      </dd>
    </div>
  )
}

function formatFullTimestamp(timestamp: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(timestamp * 1000))
}
