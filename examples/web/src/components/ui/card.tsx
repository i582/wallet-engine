import type {HTMLAttributes, ReactElement} from "react"

import {cn} from "@/lib/utils"

export function Card({className, ...props}: HTMLAttributes<HTMLDivElement>): ReactElement {
  return (
    <div
      className={cn(
        "rounded-[1.75rem] border border-border/80 bg-card text-card-foreground shadow-[0_24px_60px_-34px_rgba(24,24,27,0.32)]",
        className,
      )}
      {...props}
    />
  )
}

export function CardHeader({className, ...props}: HTMLAttributes<HTMLDivElement>): ReactElement {
  return <div className={cn("flex flex-col gap-1.5 px-6 pt-6", className)} {...props} />
}

export function CardTitle({className, ...props}: HTMLAttributes<HTMLHeadingElement>): ReactElement {
  return <h2 className={cn("text-lg font-semibold tracking-tight", className)} {...props} />
}

export function CardDescription({
  className,
  ...props
}: HTMLAttributes<HTMLParagraphElement>): ReactElement {
  return <p className={cn("text-sm leading-6 text-muted-foreground", className)} {...props} />
}

export function CardContent({className, ...props}: HTMLAttributes<HTMLDivElement>): ReactElement {
  return <div className={cn("px-6 pb-6 pt-5", className)} {...props} />
}
