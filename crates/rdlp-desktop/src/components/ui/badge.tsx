import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"
import type { LiteralUnion } from "./_internal/types"

const badgeVariants = cva(
  [
    "inline-flex items-center justify-center rounded-md border px-2 py-0.5 text-xs font-medium w-fit whitespace-nowrap shrink-0",
    "[&>svg]:size-3 gap-1 [&>svg]:pointer-events-none",
    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
    "transition-[color,box-shadow] overflow-hidden",
  ],
  {
    variants: {
      variant: {
        default:
          "border-transparent bg-primary text-primary-foreground [a&]:hover:bg-primary/90",
        secondary:
          "border-transparent bg-secondary text-secondary-foreground [a&]:hover:bg-secondary/90",
        destructive:
          "border-transparent bg-destructive text-white [a&]:hover:bg-destructive/90",
        outline:
          "text-foreground [a&]:hover:bg-accent [a&]:hover:text-accent-foreground",
      },
    },
    defaultVariants: { variant: "default" },
  },
)

interface BadgeProps
  extends React.ComponentProps<"span">,
    Omit<VariantProps<typeof badgeVariants>, "variant"> {
  /**
   * Visual variant. Canonical values are autocompleted by the IDE; consumers
   * may pass arbitrary strings if they need a project-specific variant added
   * via additional className overrides.
   */
  variant?: LiteralUnion<"default" | "secondary" | "destructive" | "outline">
}

function Badge({ className, variant, ...props }: BadgeProps) {
  return (
    <span
      data-slot="badge"
      className={cn(badgeVariants({ variant: variant as VariantProps<typeof badgeVariants>["variant"] }), className)}
      {...props}
    />
  )
}

export { Badge, badgeVariants }
export type { BadgeProps }
