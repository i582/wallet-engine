import type {
  SendPreview,
  SendResult,
  TonConnectInteraction,
  TonConnectWalletEvent,
  WalletSnapshot,
} from "@ton/wallet-engine"
import {parseTonConnectLink} from "@ton/wallet-engine"
import {type ReactElement, useEffect, useState} from "react"

import {CollectiblesPage} from "@/components/collectibles-page"
import {NftDetailPage} from "@/components/nft-detail-page"
import {RecoveryScreen} from "@/components/recovery-screen"
import {WalletDashboard} from "@/components/wallet-dashboard"
import {WelcomeScreen} from "@/components/welcome-screen"
import {errorMessage} from "@/lib/error-message"
import {type NftRoute, parseNftRoute} from "@/lib/nft-route"
import {fetchGramUsdRate} from "@/lib/tonapi-rates"
import {WalletSession} from "@/lib/wallet-session"

interface TonConnectConnection {
  readonly kind: "connected"
  readonly dappName: string
}

export function App(): ReactElement {
  const [session, setSession] = useState<WalletSession>()
  const [recoveryWords, setRecoveryWords] = useState<string[]>()
  const [snapshot, setSnapshot] = useState<WalletSnapshot>()
  const [creating, setCreating] = useState<boolean>(false)
  const [refreshing, setRefreshing] = useState<boolean>(false)
  const [loadingMore, setLoadingMore] = useState<boolean>(false)
  const [loadingMoreNfts, setLoadingMoreNfts] = useState<boolean>(false)
  const [error, setError] = useState<string>()
  const [restoring, setRestoring] = useState<boolean>(true)
  const [gramUsdRate, setGramUsdRate] = useState<number>()
  const [tonConnectInteraction, setTonConnectInteraction] = useState<TonConnectInteraction>()
  const [tonConnectConnection, setTonConnectConnection] = useState<TonConnectConnection>()
  const [nftRoute, setNftRoute] = useState<NftRoute>(() => parseNftRoute(globalThis.location.hash))
  const connectedDappName: string | undefined =
    tonConnectConnection?.kind === "connected" ? tonConnectConnection.dappName : undefined

  useEffect(() => {
    // biome-ignore lint/correctness/useQwikValidLexicalScope: this is a React hashchange subscription.
    const syncNftRoute = (): void => setNftRoute(parseNftRoute(globalThis.location.hash))
    globalThis.addEventListener("hashchange", syncNftRoute)
    return () => globalThis.removeEventListener("hashchange", syncNftRoute)
  }, [])

  useEffect(() => {
    let active: boolean = true
    fetchGramUsdRate()
      .then((rate: number) => {
        if (active) {
          setGramUsdRate(rate)
        }
      })
      .catch(() => undefined)
    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    let active: boolean = true
    WalletSession.restore()
      .then(async (restored: WalletSession | undefined) => {
        if (!active || !restored) {
          return
        }
        setSession(restored)
        setSnapshot(restored.snapshot())
        try {
          await Promise.all([restored.refresh(), restored.refreshNfts()])
          setSnapshot(restored.snapshot())
        } catch (cause) {
          setError(errorMessage(cause))
        }
      })
      .catch((cause: unknown) => {
        if (active) {
          setError(errorMessage(cause))
        }
      })
      .finally(() => {
        if (active) {
          setRestoring(false)
        }
      })
    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    return () => {
      if (session) {
        void session.close()
      }
    }
  }, [session])

  useEffect(() => {
    if (!session) {
      return
    }
    // biome-ignore lint/correctness/useQwikValidLexicalScope: this is a browser paste listener owned by a React effect.
    const receivePaste = (event: ClipboardEvent): void => {
      const value: string = event.clipboardData?.getData("text").trim() ?? ""
      if (!isTonConnectLink(value)) {
        return
      }
      event.preventDefault()
      setError(undefined)
      session.startTonConnect(value).catch((cause: unknown) => setError(errorMessage(cause)))
    }
    globalThis.addEventListener("paste", receivePaste)
    return () => globalThis.removeEventListener("paste", receivePaste)
  }, [session])

  useEffect(() => {
    if (!session) {
      return
    }
    // biome-ignore lint/correctness/useQwikValidLexicalScope: this is a React effect subscription, not a Qwik resumable closure.
    const receiveTonConnectEvent = (event: TonConnectWalletEvent): void => {
      if (event.kind === "interaction") {
        setTonConnectInteraction(event.interaction)
        return
      }
      if (event.kind === "connected") {
        setTonConnectInteraction(undefined)
        setTonConnectConnection({kind: "connected", dappName: event.dappName})
        setError(undefined)
        return
      }
      if (event.kind === "transactionFinished") {
        setTonConnectInteraction(undefined)
        setSnapshot(session.snapshot())
        return
      }
      if (event.kind === "disconnected") {
        setTonConnectInteraction(undefined)
        setTonConnectConnection(undefined)
        setError(undefined)
        return
      }
      setError(event.message)
    }
    const unsubscribe = session.onTonConnectEvent(receiveTonConnectEvent)
    session.restoreTonConnect().catch((cause: unknown) => setError(errorMessage(cause)))
    return unsubscribe
  }, [session])

  async function createWallet(): Promise<void> {
    setCreating(true)
    setError(undefined)
    try {
      const created = await WalletSession.create()
      setSession(created.session)
      setSnapshot(created.session.snapshot())
      setRecoveryWords(created.recoveryPhrase.phrase.trim().split(/\s+/u))
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setCreating(false)
    }
  }

  async function openWallet(): Promise<void> {
    if (!session) {
      return
    }
    setRecoveryWords(undefined)
    await refreshWallet()
  }

  async function refreshWallet(): Promise<void> {
    if (!session || refreshing) {
      return
    }
    setRefreshing(true)
    setError(undefined)
    const rateRequest: Promise<void> = fetchGramUsdRate()
      .then((rate: number) => setGramUsdRate(rate))
      .catch(() => undefined)
    try {
      await Promise.all([session.refresh(), session.refreshNfts()])
      setSnapshot(session.snapshot())
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      await rateRequest
      setRefreshing(false)
    }
  }

  async function loadMoreActivity(): Promise<void> {
    if (!session || loadingMore) {
      return
    }
    setLoadingMore(true)
    setError(undefined)
    try {
      setSnapshot(await session.loadMoreActivity())
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setLoadingMore(false)
    }
  }

  async function loadMoreNfts(): Promise<void> {
    if (!session || loadingMoreNfts || refreshing) {
      return
    }
    setLoadingMoreNfts(true)
    setError(undefined)
    try {
      setSnapshot(await session.loadMoreNfts())
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setLoadingMoreNfts(false)
    }
  }

  async function forgetWallet(): Promise<void> {
    if (!session) {
      return
    }
    await session.forget()
    setSession(undefined)
    setSnapshot(undefined)
    setRecoveryWords(undefined)
    setError(undefined)
  }

  async function previewTransfer(
    destination: string,
    amountNanograms: string,
  ): Promise<SendPreview> {
    if (!session) {
      throw new Error("The wallet is not open")
    }
    return await session.previewSend(destination, amountNanograms)
  }

  async function cancelTransferPreview(): Promise<void> {
    await session?.cancelSendPreview()
  }

  async function sendTransfer(
    destination: string,
    amountNanograms: string,
    force: boolean,
  ): Promise<SendResult> {
    if (!session) {
      throw new Error("The wallet is not open")
    }
    try {
      return await session.send(destination, amountNanograms, force)
    } finally {
      setSnapshot(session.snapshot())
    }
  }

  async function startTonConnect(link: string): Promise<void> {
    if (!session) {
      throw new Error("The wallet is not open")
    }
    setError(undefined)
    session.startTonConnect(link).catch((cause: unknown) => setError(errorMessage(cause)))
  }

  function respondTonConnect(interactionId: string, approved: boolean, force: boolean): void {
    session?.respondTonConnect(interactionId, approved, force)
    setTonConnectInteraction(undefined)
  }

  async function disconnectTonConnect(): Promise<void> {
    await session?.disconnectTonConnect()
    setTonConnectConnection(undefined)
    setTonConnectInteraction(undefined)
  }

  if (session && recoveryWords) {
    return <RecoveryScreen words={recoveryWords} onContinue={openWallet} />
  }

  if (restoring) {
    return <WalletLoading />
  }

  if (session && snapshot) {
    if (nftRoute.kind === "collection") {
      return (
        <CollectiblesPage
          loadingMore={loadingMoreNfts}
          nfts={snapshot.nfts}
          refreshing={refreshing}
          onLoadMore={loadMoreNfts}
          onRefresh={refreshWallet}
        />
      )
    }
    if (nftRoute.kind === "detail") {
      return (
        <NftDetailPage
          hasMore={snapshot.nfts.hasMore}
          item={snapshot.nfts.items.find(item => item.address === nftRoute.address)}
          loadingMore={loadingMoreNfts}
          onLoadMore={loadMoreNfts}
        />
      )
    }
    return (
      <WalletDashboard
        error={error}
        gramUsdRate={gramUsdRate}
        loadingMore={loadingMore}
        loadingMoreNfts={loadingMoreNfts}
        refreshing={refreshing}
        snapshot={snapshot}
        connectedDappName={connectedDappName}
        tonConnectInteraction={tonConnectInteraction}
        onDismissError={() => setError(undefined)}
        onForget={forgetWallet}
        onLoadMore={loadMoreActivity}
        onLoadMoreNfts={loadMoreNfts}
        onCancelSendPreview={cancelTransferPreview}
        onPreviewSend={previewTransfer}
        onRefresh={refreshWallet}
        onSend={sendTransfer}
        onStartTonConnect={startTonConnect}
        onRespondTonConnect={respondTonConnect}
        onDisconnectTonConnect={disconnectTonConnect}
      />
    )
  }

  return <WelcomeScreen busy={creating} error={error} onCreate={createWallet} />
}

function WalletLoading(): ReactElement {
  return (
    <main className="flex h-[100dvh] items-center justify-center bg-background">
      <p className="text-sm text-muted-foreground">Opening wallet…</p>
    </main>
  )
}

function isTonConnectLink(value: string): boolean {
  if (!value.startsWith("tc://")) {
    return false
  }
  try {
    parseTonConnectLink(value)
    return true
  } catch {
    return false
  }
}
