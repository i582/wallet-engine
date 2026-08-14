import {Slot} from "@radix-ui/react-slot"
import {cva, type VariantProps} from "class-variance-authority"
import type {ButtonHTMLAttributes, ReactElement} from "react"

import {cn} from "@/lib/utils"

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-xl text-sm font-medium transition-[background-color,color,transform,box-shadow] duration-200 ease-out outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-45 active:scale-[0.98] [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground shadow-[0_8px_24px_-12px_rgba(47,111,221,0.7)] hover:bg-primary/90",
        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/75",
        outline: "border border-border bg-background hover:bg-secondary/55",
        ghost: "hover:bg-secondary/65",
        destructive: "bg-destructive text-white hover:bg-destructive/90",
      },
      size: {
        default: "h-11 px-5",
        sm: "h-9 rounded-lg px-3.5 text-xs",
        icon: "size-11 p-0",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
)

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  readonly asChild?: boolean
}

export function Button({
  className,
  variant,
  size,
  asChild = false,
  ...props
}: ButtonProps): ReactElement {
  const Component = asChild ? Slot : "button"
  return <Component className={cn(buttonVariants({variant, size}), className)} {...props} />
}
