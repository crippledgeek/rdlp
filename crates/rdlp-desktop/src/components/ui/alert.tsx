import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"
import type { LiteralUnion } from "./_internal/types"

const alertVariants = cva(
  [
    "relative w-full rounded-lg border px-4 py-3 text-sm",
    "[&>svg]:absolute [&>svg]:left-4 [&>svg]:top-4 [&>svg]:text-foreground",
    "[&>svg~*]:pl-7",
  ],
  {
    variants: {
      variant: {
        default: "bg-background text-foreground",
        destructive: "border-destructive/50 text-destructive [&>svg]:text-destructive",
        warning: "border-yellow-500/50 text-yellow-600 [&>svg]:text-yellow-600",
        success: "border-green-500/50 text-green-600 [&>svg]:text-green-600",
      },
    },
    defaultVariants: { variant: "default" },
  },
)

interface AlertProps
  extends React.ComponentProps<"div">,
    Omit<VariantProps<typeof alertVariants>, "variant"> {
  variant?: LiteralUnion<"default" | "destructive" | "warning" | "success">
}

function Alert({ className, variant, ...props }: AlertProps) {
  return (
    <div
      role="alert"
      aria-live={variant === "destructive" ? "assertive" : "polite"}
      className={cn(
        alertVariants({ variant: variant as VariantProps<typeof alertVariants>["variant"] }),
        className,
      )}
      {...props}
    />
  )
}

function AlertTitle({ className, ...props }: React.ComponentProps<"h5">) {
  return <h5 className={cn("mb-1 font-medium leading-none tracking-tight", className)} {...props} />
}

function AlertDescription({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("text-sm [&_p]:leading-relaxed", className)} {...props} />
}

export { Alert, AlertTitle, AlertDescription, alertVariants }
export type { AlertProps }
