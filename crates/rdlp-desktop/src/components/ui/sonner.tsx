"use client"

/**
 * Prism Toast — wraps react-aria-components' UNSTABLE_Toast family.
 *
 * NOTE: RAC's Toast components are still UNSTABLE_-prefixed (March 2025
 * release; pre-stable API). When RAC drops the prefix, update the imports
 * here. No consumer-side change required at that point.
 */

import {
  UNSTABLE_Toast as AriaToast,
  UNSTABLE_ToastContent as AriaToastContent,
  UNSTABLE_ToastQueue as AriaToastQueue,
  UNSTABLE_ToastRegion as AriaToastRegion,
} from "react-aria-components"
import { CircleCheck, CircleX, Info, AlertTriangle } from "lucide-react"

import { cn } from "@/lib/utils"
import type { Branded, LiteralUnion } from "./_internal/types"

/** Brand keys returned by the queue so consumers can't pass arbitrary strings to dismiss. */
export type ToastKey = Branded<string, "PrismToast">

export type ToastSeverity = LiteralUnion<"info" | "success" | "warning" | "error">

interface ToastContent {
  title: string
  description?: string | undefined
  severity?: ToastSeverity | undefined
}

/** Singleton queue. Imported once at app root via <ToastRegion />. */
const queue = new AriaToastQueue<ToastContent>({
  maxVisibleToasts: 5,
})

/** App-root component. Render once in App.tsx. */
function ToastRegion() {
  return (
    <AriaToastRegion
      queue={queue}
      className="fixed bottom-0 right-0 flex flex-col gap-2 p-4 outline-none"
    >
      {({ toast }) => (
        <AriaToast
          toast={toast}
          className={cn(
            "flex items-center gap-3 rounded-md border bg-background p-3 shadow-lg",
            "data-[entering]:animate-in data-[entering]:slide-in-from-right",
            "data-[exiting]:animate-out data-[exiting]:slide-out-to-right",
            severityClass(toast.content.severity),
          )}
        >
          <SeverityIcon severity={toast.content.severity} />
          <AriaToastContent className="flex-1">
            <div className="text-sm font-medium">{toast.content.title}</div>
            {toast.content.description && (
              <div className="text-xs text-muted-foreground">{toast.content.description}</div>
            )}
          </AriaToastContent>
        </AriaToast>
      )}
    </AriaToastRegion>
  )
}

function SeverityIcon({ severity }: { severity?: ToastSeverity | undefined }) {
  switch (severity) {
    case "success":
      return <CircleCheck aria-hidden="true" className="size-5 text-green-500" />
    case "error":
      return <CircleX aria-hidden="true" className="size-5 text-destructive" />
    case "warning":
      return <AlertTriangle aria-hidden="true" className="size-5 text-yellow-500" />
    case "info":
    default:
      return <Info aria-hidden="true" className="size-5 text-primary" />
  }
}

function severityClass(severity?: ToastSeverity) {
  switch (severity) {
    case "success": return "border-green-500/30"
    case "error":   return "border-destructive/30"
    case "warning": return "border-yellow-500/30"
    default:        return "border-input"
  }
}

/** Programmatic toast API — drop-in for sonner's toast.* shape. */
const toast = {
  success(title: string, options?: { description?: string; timeout?: number }): ToastKey {
    return queue.add({ title, severity: "success", description: options?.description }, { timeout: options?.timeout ?? 5000 }) as ToastKey
  },
  error(title: string, options?: { description?: string; timeout?: number }): ToastKey {
    return queue.add({ title, severity: "error", description: options?.description }, { timeout: options?.timeout ?? 8000 }) as ToastKey
  },
  warning(title: string, options?: { description?: string; timeout?: number }): ToastKey {
    return queue.add({ title, severity: "warning", description: options?.description }, { timeout: options?.timeout ?? 6000 }) as ToastKey
  },
  info(title: string, options?: { description?: string; timeout?: number }): ToastKey {
    return queue.add({ title, severity: "info", description: options?.description }, { timeout: options?.timeout ?? 5000 }) as ToastKey
  },
  dismiss(key: ToastKey): void {
    queue.close(key)
  },
}

export { ToastRegion, toast }
export type { ToastContent }
