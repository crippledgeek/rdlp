"use client"

import * as React from "react"
import {
  Input as AriaInput,
  type InputProps as AriaInputProps,
} from "react-aria-components"

import { cn } from "@/lib/utils"

/**
 * Standalone <Input> wrapper around react-aria-components' Input.
 *
 * Works without a parent TextField — accepts the standard React input
 * event model (`value`, `onChange`, `type`, `placeholder`, etc.) so the
 * 4 settings-section consumers don't change. When/if a section adopts
 * RAC's TextField wrapper for native validation orchestration, this
 * Input becomes the inner element of that TextField.
 */
const Input = React.forwardRef<HTMLInputElement, AriaInputProps>(
  ({ className, ...props }, ref) => (
    <AriaInput
      ref={ref}
      className={cn(
        "flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm",
        "file:border-0 file:bg-transparent file:text-sm file:font-medium",
        "placeholder:text-muted-foreground",
        /* Focused */
        "data-[focus-visible]:outline-none data-[focus-visible]:ring-1 data-[focus-visible]:ring-ring",
        /* Disabled */
        "data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50",
        /* Invalid */
        "data-[invalid]:border-destructive data-[invalid]:ring-destructive",
        className,
      )}
      {...props}
    />
  ),
)
Input.displayName = "Input"

export { Input }
export type { AriaInputProps as InputProps }
