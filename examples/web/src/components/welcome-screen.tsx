import {ArrowRight, ShieldCheck, Wallet} from "@phosphor-icons/react"
import type {ReactElement} from "react"

import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert"
import {Button} from "@/components/ui/button"

export interface WelcomeScreenProps {
  readonly busy: boolean
  readonly error?: string
  readonly onCreate: () => Promise<void>
}

export function WelcomeScreen({busy, error, onCreate}: WelcomeScreenProps): ReactElement {
  return (
    <main className="mx-auto grid h-[100dvh] w-full max-w-md grid-rows-[auto_1fr_auto] overflow-hidden bg-background px-5 py-6 sm:py-8">
      <header className="flex items-center gap-3">
        <span className="flex size-10 items-center justify-center rounded-xl bg-foreground text-background">
          <Wallet aria-hidden="true" size={20} weight="regular" />
        </span>
        <p className="font-semibold tracking-tight">Wallet</p>
      </header>

      <section className="self-center py-4 text-center">
        <div className="mx-auto flex size-16 items-center justify-center rounded-2xl bg-primary text-primary-foreground shadow-[0_18px_40px_-22px_rgba(47,111,221,0.75)]">
          <Wallet aria-hidden="true" size={30} weight="regular" />
        </div>
        <h1 className="mt-5 text-2xl font-semibold tracking-[-0.04em]">Create wallet</h1>
        <p className="mx-auto mt-2 max-w-sm text-sm leading-5 text-muted-foreground">
          Save your recovery words to keep access to your funds.
        </p>

        {error ? (
          <Alert className="mt-5 text-left">
            <ShieldCheck aria-hidden="true" className="mt-0.5 text-destructive" size={18} />
            <AlertTitle>Could not create wallet</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}
      </section>

      <div>
        <Button className="h-12 w-full justify-between" disabled={busy} onClick={onCreate}>
          <span>{busy ? "Creating" : "Create wallet"}</span>
          <ArrowRight aria-hidden="true" size={18} weight="regular" />
        </Button>
      </div>
    </main>
  )
}
