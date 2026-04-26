"use client"

import { Label as AriaLabel, type LabelProps as AriaLabelProps } from "react-aria-components"

import { cn } from "@/lib/utils"

/**
 * Standalone <Label> component.
 *
 * Backed by react-aria-components' Label primitive. Outside a TextField
 * parent context this just renders a plain `<label>` element; the
 * `htmlFor` association via standard HTML still works. Inside a Field /
 * TextField context, RAC auto-wires the for/id binding via React Aria
 * context — no `htmlFor` prop needed.
 */
function Label({ className, ...props }: AriaLabelProps) {
  return (
    <AriaLabel
      className={cn(
        "text-sm font-medium leading-none",
        /* Disabled — RAC sets data-disabled on the Label when inside a disabled Field */
        "data-[disabled]:cursor-not-allowed data-[disabled]:opacity-70",
        className,
      )}
      {...props}
    />
  )
}

export { Label }
export type { AriaLabelProps as LabelProps }
