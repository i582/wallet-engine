import {Check, Copy} from "@phosphor-icons/react"
import {type ChangeEvent, type ReactElement, useState} from "react"

import {Button} from "@/components/ui/button"
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from "@/components/ui/card"

export interface RecoveryScreenProps {
  readonly words: string[]
  readonly onContinue: () => Promise<void>
}

export function RecoveryScreen({words, onContinue}: RecoveryScreenProps): ReactElement {
  const [saved, setSaved] = useState<boolean>(false)
  const [copied, setCopied] = useState<boolean>(false)

  async function copyPhrase(): Promise<void> {
    await navigator.clipboard.writeText(words.join(" "))
    setCopied(true)
    globalThis.setTimeout(() => setCopied(false), 1600)
  }

  function changeSaved(event: ChangeEvent<HTMLInputElement>): void {
    setSaved(event.target.checked)
  }

  return (
    <main className="mx-auto h-[100dvh] w-full max-w-lg overflow-hidden bg-background">
      <Card className="flex h-full flex-col overflow-hidden rounded-none border-y-0 bg-background shadow-none">
        <CardHeader className="border-b border-border pb-5 pt-6">
          <CardTitle className="text-2xl tracking-[-0.035em]">Back up your wallet</CardTitle>
          <CardDescription>Write these words down in order and keep them private.</CardDescription>
        </CardHeader>
        <CardContent className="flex min-h-0 flex-1 flex-col px-5 pb-6 pt-5">
          <ol className="grid grid-cols-2 gap-x-5 rounded-2xl border border-border bg-secondary/35 px-4 py-2 sm:grid-cols-3">
            {words.map((word, index) => (
              <li
                className="grid grid-cols-[1.5rem_1fr] items-baseline border-b border-border/70 py-2 text-sm [&:nth-last-child(-n+2)]:border-b-0 sm:[&:nth-last-child(-n+3)]:border-b-0"
                key={`${index}-${word}`}
              >
                <span className="text-xs text-muted-foreground">{index + 1}</span>
                <span>{word}</span>
              </li>
            ))}
          </ol>

          <div className="mt-auto pt-5">
            <Button className="w-full" variant="outline" onClick={copyPhrase}>
              {copied ? (
                <Check aria-hidden="true" size={18} />
              ) : (
                <Copy aria-hidden="true" size={18} />
              )}
              {copied ? "Copied" : "Copy phrase"}
            </Button>
            <label className="mt-3 flex cursor-pointer items-start gap-3 rounded-xl border border-border px-4 py-3 text-sm leading-5">
              <input
                checked={saved}
                className="mt-0.5 size-4 accent-primary"
                type="checkbox"
                onChange={changeSaved}
              />
              <span>I saved these words.</span>
            </label>

            <Button className="mt-3 w-full" disabled={!saved} onClick={onContinue}>
              Continue
            </Button>
          </div>
        </CardContent>
      </Card>
    </main>
  )
}
