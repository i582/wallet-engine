import type {HTMLAttributes, ReactElement} from "react"

import {cn} from "@/lib/utils"

export function Skeleton({className, ...props}: HTMLAttributes<HTMLDivElement>): ReactElement {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "animate-pulse rounded-lg bg-[linear-gradient(100deg,var(--secondary)_25%,var(--muted)_45%,var(--secondary)_65%)] bg-[length:240%_100%]",
        className,
      )}
      {...props}
    />
  )
}
