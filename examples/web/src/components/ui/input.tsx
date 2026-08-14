import type {InputHTMLAttributes, ReactElement} from "react"

import {cn} from "@/lib/utils"

export function Input({className, ...props}: InputHTMLAttributes<HTMLInputElement>): ReactElement {
  return (
    <input
      className={cn(
        "h-11 w-full rounded-xl border border-border bg-background px-3 text-sm outline-none transition-[border-color,box-shadow] placeholder:text-muted-foreground focus:border-primary focus:ring-2 focus:ring-primary/15 disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  )
}
