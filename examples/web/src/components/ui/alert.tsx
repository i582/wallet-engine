import type {HTMLAttributes, ReactElement} from "react"

import {cn} from "@/lib/utils"

export function Alert({className, ...props}: HTMLAttributes<HTMLDivElement>): ReactElement {
  return (
    <div
      role="alert"
      className={cn(
        "grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 rounded-2xl border border-border bg-secondary/45 p-4 text-sm",
        className,
      )}
      {...props}
    />
  )
}

export function AlertTitle({
  className,
  ...props
}: HTMLAttributes<HTMLHeadingElement>): ReactElement {
  return <h3 className={cn("font-semibold leading-5", className)} {...props} />
}

export function AlertDescription({
  className,
  ...props
}: HTMLAttributes<HTMLParagraphElement>): ReactElement {
  return <p className={cn("col-start-2 leading-5 text-muted-foreground", className)} {...props} />
}
