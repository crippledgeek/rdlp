"use client"

import { cva } from "class-variance-authority"
import {
  Label as AriaLabel,
  type LabelProps as AriaLabelProps,
  ProgressBar as AriaProgressBar,
  type ProgressBarProps as AriaProgressBarProps,
  composeRenderProps,
} from "react-aria-components"

import { cn } from "@/lib/utils"

const labelVariants = cva([
  "text-sm font-medium leading-none",
  /* Disabled */
  "data-[disabled]:cursor-not-allowed data-[disabled]:opacity-70",
  /* Invalid */
  "group-data-[invalid]:text-destructive",
])

const Label = ({ className, ...props }: AriaLabelProps) => (
  <AriaLabel className={cn(labelVariants(), className)} {...props} />
)

interface ProgressProps extends AriaProgressBarProps {
  barClassName?: string
  fillClassName?: string
}

const Progress = ({
  className,
  barClassName,
  fillClassName,
  children,
  ...props
}: ProgressProps) => (
  <AriaProgressBar
    className={composeRenderProps(className, (className) =>
      cn("w-full", className)
    )}
    {...props}
  >
    {composeRenderProps(children, (children, renderProps) => (
      <>
        {children}
        <div
          className={cn(
            "relative h-2 w-full overflow-hidden rounded-full bg-primary/20",
            barClassName
          )}
        >
          <div
            className={cn(
              "size-full flex-1 bg-primary transition-all",
              fillClassName
            )}
            style={{
              transform: `translateX(-${100 - (renderProps.percentage || 0)}%)`,
            }}
          />
        </div>
      </>
    ))}
  </AriaProgressBar>
)

interface JollyProgressBarProps extends ProgressProps {
  label?: string
  showValue?: boolean
}

function JollyProgressBar({
  label,
  className,
  showValue = true,
  ...props
}: JollyProgressBarProps) {
  return (
    <Progress
      className={composeRenderProps(className, (className) =>
        cn("group flex flex-col gap-2", className)
      )}
      {...props}
    >
      {({ valueText }) => (
        <div className="flex w-full justify-between">
          <Label>{label}</Label>
          {showValue && <span className={labelVariants()}>{valueText}</span>}
        </div>
      )}
    </Progress>
  )
}

export { Progress, JollyProgressBar }
export type { ProgressProps, JollyProgressBarProps }
