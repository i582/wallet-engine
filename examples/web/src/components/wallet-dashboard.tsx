import {
  ArrowClockwise,
  ArrowDownLeft,
  ArrowUpRight,
  Check,
  Copy,
  PaperPlaneTilt,
  PlugsConnected,
  Receipt,
  Trash,
  WarningCircle,
  X,
} from "@phosphor-icons/react"
import type {Icon} from "@phosphor-icons/react"
import type {
  ActivityItem,
  SendPreview,
  SendResult,
  TonConnectInteraction,
  WalletSnapshot,
} from "@ton/wallet-engine"
import {type ReactElement, useEffect, useId, useState} from "react"

import {SendForm} from "@/components/send-form"
import {TonConnectDialog} from "@/components/ton-connect-dialog"
import {TransactionView} from "@/components/transaction-view"
import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert"
import {Button} from "@/components/ui/button"
import {Card, CardContent} from "@/components/ui/card"
import {Skeleton} from "@/components/ui/skeleton"
import {
  compactAddress,
  formatActivityAmount,
  formatNanogramBalance,
  formatTimestamp,
  formatUsdNanograms,
} from "@/lib/format"

export interface WalletDashboardProps {
  readonly snapshot: WalletSnapshot
  readonly gramUsdRate?: number
  readonly refreshing: boolean
  readonly loadingMore: boolean
  readonly error?: string
  readonly connectedDappName?: string
  readonly tonConnectInteraction?: TonConnectInteraction
  readonly onDismissError: () => void
  readonly onRefresh: () => Promise<void>
  readonly onLoadMore: () => Promise<void>
  readonly onForget: () => Promise<void>
  readonly onPreviewSend: (destination: string, amountNanograms: string) => Promise<SendPreview>
  readonly onCancelSendPreview: () => Promise<void>
  readonly onSend: (
    destination: string,
    amountNanograms: string,
    force: boolean,
  ) => Promise<SendResult>
  readonly onStartTonConnect: (link: string) => Promise<void>
  readonly onRespondTonConnect: (interactionId: string, approved: boolean, force: boolean) => void
  readonly onDisconnectTonConnect: () => Promise<void>
}

export function WalletDashboard({
  snapshot,
  gramUsdRate,
  refreshing,
  loadingMore,
  error,
  connectedDappName,
  tonConnectInteraction,
  onDismissError,
  onRefresh,
  onLoadMore,
  onForget,
  onPreviewSend,
  onCancelSendPreview,
  onSend,
  onStartTonConnect,
  onRespondTonConnect,
  onDisconnectTonConnect,
}: WalletDashboardProps): ReactElement {
  const [copied, setCopied] = useState<boolean>(false)
  const [sendOpen, setSendOpen] = useState<boolean>(false)
  const [deleteOpen, setDeleteOpen] = useState<boolean>(false)
  const [tonConnectOpen, setTonConnectOpen] = useState<boolean>(false)
  const [selectedActivity, setSelectedActivity] = useState<ActivityItem>()

  useEffect(() => {
    if (connectedDappName) {
      setTonConnectOpen(false)
    }
  }, [connectedDappName])

  async function copyAddress(): Promise<void> {
    await navigator.clipboard.writeText(snapshot.address)
    setCopied(true)
    globalThis.setTimeout(() => setCopied(false), 1600)
  }

  return (
    <main className="mx-auto flex h-[100dvh] w-full max-w-lg flex-col overflow-hidden bg-background px-5 pb-6 pt-4 sm:pb-8 sm:pt-5">
      <header className="flex min-h-10 items-center">
        <div>
          <div className="flex items-center gap-2 leading-5">
            <p className="font-semibold tracking-tight">My wallet</p>
            <span className="text-xs text-muted-foreground">Testnet</span>
            <Button
              aria-label="Delete wallet"
              className="size-7 text-destructive hover:bg-destructive/10 hover:text-destructive"
              size="icon"
              variant="ghost"
              onClick={() => setDeleteOpen(true)}
            >
              <Trash aria-hidden="true" size={16} />
            </Button>
          </div>
          <button
            aria-label="Copy wallet address"
            className="mt-0.5 block text-xs leading-4 text-muted-foreground hover:text-foreground"
            type="button"
            onClick={copyAddress}
          >
            {compactAddress(snapshot.address)}
          </button>
        </div>
      </header>

      <Card className="mt-7 overflow-hidden border-primary/10 bg-primary text-primary-foreground">
        <CardContent className="px-6 pb-4 pt-6">
          <p className="text-xs font-semibold uppercase tracking-[0.16em] text-white/65">Balance</p>
          {refreshing && !snapshot.account ? (
            <Skeleton className="mt-5 h-14 w-56 bg-white/15" />
          ) : (
            <p className="mt-4 text-5xl font-semibold tracking-[-0.055em]">
              {formatNanogramBalance(snapshot.account?.balanceNanograms)}
              <span className="ml-3 text-lg font-medium tracking-normal text-white/65">GRAM</span>
            </p>
          )}
          <p className="mt-2 text-lg font-medium text-white/70">
            {formatUsdNanograms(snapshot.account?.balanceNanograms, gramUsdRate)}
          </p>
        </CardContent>
      </Card>

      <div className="mt-5 grid grid-cols-4 gap-3">
        <WalletAction
          icon={<PaperPlaneTilt aria-hidden="true" size={21} />}
          label="Send"
          onClick={() => setSendOpen(true)}
        />
        <WalletAction
          icon={
            copied ? <Check aria-hidden="true" size={21} /> : <Copy aria-hidden="true" size={21} />
          }
          label={copied ? "Address copied" : "Receive"}
          onClick={copyAddress}
        />
        <WalletAction
          disabled={refreshing}
          icon={
            <ArrowClockwise
              aria-hidden="true"
              className={refreshing ? "animate-spin" : undefined}
              size={21}
            />
          }
          label="Refresh"
          onClick={onRefresh}
        />
        <WalletAction
          icon={<PlugsConnected aria-hidden="true" size={21} />}
          label={connectedDappName ? "Connected" : "Connect"}
          onClick={() => setTonConnectOpen(true)}
        />
      </div>

      {sendOpen ? (
        <SendForm
          canForceRetry={snapshot.send.resolution?.canForceRetry === true}
          onCancelPreview={onCancelSendPreview}
          onClose={() => setSendOpen(false)}
          onPreview={onPreviewSend}
          onSend={onSend}
        />
      ) : null}
      {selectedActivity ? (
        <TransactionView
          gramUsdRate={gramUsdRate}
          item={selectedActivity}
          onClose={() => setSelectedActivity(undefined)}
        />
      ) : null}
      {deleteOpen ? (
        <DeleteWalletDialog onCancel={() => setDeleteOpen(false)} onConfirm={onForget} />
      ) : null}
      {tonConnectOpen || tonConnectInteraction ? (
        <TonConnectDialog
          canForceRetry={snapshot.send.resolution?.canForceRetry === true}
          connectedDappName={connectedDappName}
          interaction={tonConnectInteraction}
          onClose={() => setTonConnectOpen(false)}
          onDisconnect={onDisconnectTonConnect}
          onRespond={onRespondTonConnect}
          onStart={onStartTonConnect}
        />
      ) : null}

      <div className="mt-5 min-h-0 flex-1 overflow-y-auto pb-2">
        {error ? (
          <Alert className="relative pr-12">
            <WarningCircle aria-hidden="true" className="mt-0.5 text-destructive" size={19} />
            <AlertTitle>Something went wrong</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
            <Button
              aria-label="Dismiss error"
              className="absolute right-2 top-2 size-8 text-muted-foreground"
              size="icon"
              variant="ghost"
              onClick={onDismissError}
            >
              <X aria-hidden="true" size={17} />
            </Button>
          </Alert>
        ) : null}

        {snapshot.accountResource.phase === "failed" ? (
          <ResourceError label="Account data is unavailable." onRetry={onRefresh} />
        ) : null}

        <section className={error ? "mt-7" : "mt-2"}>
          <div className="flex items-end justify-between gap-4">
            <h2 className="text-xl font-semibold tracking-[-0.03em]">Recent activity</h2>
          </div>
          <ActivityList
            gramUsdRate={gramUsdRate}
            snapshot={snapshot}
            refreshing={refreshing}
            onSelect={setSelectedActivity}
          />

          {snapshot.activity.hasMore ? (
            <Button
              className="mt-5 w-full"
              disabled={loadingMore || snapshot.activity.paginationResource.phase === "loading"}
              variant="outline"
              onClick={onLoadMore}
            >
              {loadingMore ? "Loading history" : "Load more"}
            </Button>
          ) : null}
        </section>
      </div>
    </main>
  )
}

function DeleteWalletDialog({
  onCancel,
  onConfirm,
}: {
  readonly onCancel: () => void
  readonly onConfirm: () => Promise<void>
}): ReactElement {
  const [deleting, setDeleting] = useState<boolean>(false)
  const titleId: string = useId()

  async function confirm(): Promise<void> {
    setDeleting(true)
    try {
      await onConfirm()
    } finally {
      setDeleting(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/35 p-3 sm:items-center sm:p-5">
      <div
        aria-labelledby={titleId}
        aria-modal="true"
        className="w-full max-w-sm rounded-2xl border border-border bg-background p-5"
        role="alertdialog"
      >
        <h2 className="text-lg font-semibold" id={titleId}>
          Delete wallet?
        </h2>
        <p className="mt-2 text-sm leading-5 text-muted-foreground">
          Make sure you saved the recovery words. This action cannot be undone.
        </p>
        <div className="mt-6 grid grid-cols-2 gap-3">
          <Button disabled={deleting} variant="secondary" onClick={onCancel}>
            Cancel
          </Button>
          <Button disabled={deleting} variant="destructive" onClick={confirm}>
            <Trash aria-hidden="true" size={18} />
            {deleting ? "Deleting" : "Delete"}
          </Button>
        </div>
      </div>
    </div>
  )
}

function WalletAction({
  icon,
  label,
  disabled = false,
  onClick,
}: {
  readonly icon: ReactElement
  readonly label: string
  readonly disabled?: boolean
  readonly onClick: () => void | Promise<void>
}): ReactElement {
  return (
    <Button
      className="h-auto flex-col gap-2 py-4"
      disabled={disabled}
      variant="secondary"
      onClick={onClick}
    >
      {icon}
      <span className="text-xs">{label}</span>
    </Button>
  )
}

function ActivityList({
  gramUsdRate,
  snapshot,
  refreshing,
  onSelect,
}: {
  readonly gramUsdRate?: number
  readonly snapshot: WalletSnapshot
  readonly refreshing: boolean
  readonly onSelect: (item: ActivityItem) => void
}): ReactElement {
  if (refreshing && snapshot.activity.items.length === 0) {
    return (
      <div className="mt-4 divide-y divide-border">
        {[0, 1, 2].map(index => (
          <div className="flex items-center gap-4 py-4" key={index}>
            <Skeleton className="size-11 rounded-full" />
            <div className="flex-1 space-y-2">
              <Skeleton className="h-4 w-32" />
              <Skeleton className="h-3 w-44" />
            </div>
            <Skeleton className="h-4 w-20" />
          </div>
        ))}
      </div>
    )
  }

  if (snapshot.activity.resource.phase === "failed" && snapshot.activity.items.length === 0) {
    return <ResourceError label="Activity is unavailable." />
  }

  if (snapshot.activity.items.length === 0) {
    return (
      <div className="mt-4 rounded-2xl border border-border bg-card py-10 text-center">
        <Receipt aria-hidden="true" className="mx-auto text-muted-foreground" size={27} />
        <p className="mt-3 text-sm font-medium">No activity yet</p>
        <p className="mt-1 text-sm text-muted-foreground">Receive test GRAM to get started.</p>
      </div>
    )
  }

  return (
    <div className="mt-4 divide-y divide-border">
      {snapshot.activity.items.map((item: ActivityItem) => (
        <ActivityRow gramUsdRate={gramUsdRate} item={item} key={item.id} onSelect={onSelect} />
      ))}
    </div>
  )
}

function ActivityRow({
  gramUsdRate,
  item,
  onSelect,
}: {
  readonly gramUsdRate?: number
  readonly item: ActivityItem
  readonly onSelect: (item: ActivityItem) => void
}): ReactElement {
  const received: boolean = item.direction === "received"
  const Icon: Icon = received ? ArrowDownLeft : ArrowUpRight
  return (
    <button
      className="flex w-full items-center gap-4 py-4 text-left"
      data-activity-amount={item.amountNanograms}
      data-activity-direction={item.direction}
      data-activity-id={item.id}
      type="button"
      onClick={() => onSelect(item)}
    >
      <span className="flex size-11 shrink-0 items-center justify-center rounded-full bg-secondary">
        <Icon aria-hidden="true" size={20} weight="regular" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{received ? "Received" : "Sent"}</p>
        <p className="mt-1 truncate text-xs text-muted-foreground">
          {item.counterparty
            ? compactAddress(item.counterparty)
            : compactAddress(item.transactionHash)}
          <span aria-hidden="true"> · </span>
          {formatTimestamp(item.timestamp)}
        </p>
      </div>
      <div className="shrink-0 text-right">
        <p className="text-sm font-medium">
          {formatActivityAmount(item.amountNanograms, item.direction)}
        </p>
        <p className="mt-1 text-xs text-muted-foreground">
          {formatUsdNanograms(item.amountNanograms, gramUsdRate)}
        </p>
      </div>
    </button>
  )
}

function ResourceError({
  label,
  onRetry,
}: {
  readonly label: string
  readonly onRetry?: () => Promise<void>
}): ReactElement {
  return (
    <Alert className="mt-5">
      <WarningCircle aria-hidden="true" className="mt-0.5 text-muted-foreground" size={19} />
      <AlertTitle>{label}</AlertTitle>
      <AlertDescription>
        {onRetry ? (
          <button
            className="font-medium text-primary hover:underline"
            type="button"
            onClick={onRetry}
          >
            Try again
          </button>
        ) : (
          "Refresh the wallet to try again."
        )}
      </AlertDescription>
    </Alert>
  )
}
