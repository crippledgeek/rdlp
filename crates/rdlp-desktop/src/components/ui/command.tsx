"use client"

import { Search } from "lucide-react"
import {
  Autocomplete as AriaAutocomplete,
  Menu as AriaMenu,
  MenuItem as AriaMenuItem,
  MenuSection as AriaMenuSection,
  Header as AriaHeader,
  Separator as AriaSeparator,
  SearchField as AriaSearchField,
  Input as AriaInput,
  type MenuItemProps as AriaMenuItemProps,
  type MenuSectionProps as AriaMenuSectionProps,
  type SeparatorProps as AriaSeparatorProps,
  useFilter,
} from "react-aria-components"

import { cn } from "@/lib/utils"

/* Smart parent — wraps Autocomplete with a default contains-filter. */
function Command({ children, className }: { children: React.ReactNode; className?: string }) {
  const { contains } = useFilter({ sensitivity: "base" })
  return (
    <div className={cn("flex h-full w-full flex-col overflow-hidden rounded-md bg-popover text-popover-foreground", className)}>
      <AriaAutocomplete filter={contains}>
        {children}
      </AriaAutocomplete>
    </div>
  )
}

function CommandInput({ placeholder, className }: { placeholder?: string; className?: string }) {
  return (
    <AriaSearchField className="flex items-center border-b px-3" aria-label="Search">
      <Search aria-hidden="true" className="mr-2 size-4 shrink-0 opacity-50" />
      <AriaInput
        {...(placeholder !== undefined && { placeholder })}
        className={cn(
          "flex h-11 w-full bg-transparent py-3 text-sm outline-none",
          "placeholder:text-muted-foreground",
          "data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50",
          className,
        )}
      />
    </AriaSearchField>
  )
}

function CommandList({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <AriaMenu
      className={cn("max-h-[300px] overflow-y-auto overflow-x-hidden p-1", className)}
    >
      {children}
    </AriaMenu>
  )
}

function CommandItem({ className, children, ...props }: AriaMenuItemProps) {
  return (
    <AriaMenuItem
      className={cn(
        "relative flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none",
        "data-[focused]:bg-accent data-[focused]:text-accent-foreground",
        "data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
        className,
      )}
      {...props}
    >
      {children}
    </AriaMenuItem>
  )
}

/**
 * CommandEmpty — RAC's Autocomplete shows an empty state automatically when
 * the filter excludes all items. This component is a no-op shim kept for
 * API parity with cmdk; consumers can pass it but its content is overridden
 * by RAC's renderEmptyState slot if used.
 */
function CommandEmpty({ children }: { children: React.ReactNode }) {
  return <div className="py-6 text-center text-sm">{children}</div>
}

/**
 * CommandGroup — wraps a labeled section of items.
 *
 * cmdk used `<CommandGroup heading="…">…</CommandGroup>`. We map `heading`
 * to a RAC <Header> element rendered above the section's items, matching
 * the canonical RAC Menu+Section composition.
 */
interface CommandGroupProps extends Omit<AriaMenuSectionProps<object>, "children"> {
  heading?: string
  className?: string
  children: React.ReactNode
}

function CommandGroup({ heading, className, children, ...props }: CommandGroupProps) {
  return (
    <AriaMenuSection
      className={cn("overflow-hidden p-1 text-foreground", className)}
      {...props}
    >
      {heading ? (
        <AriaHeader className="px-2 py-1.5 text-xs font-medium text-muted-foreground">
          {heading}
        </AriaHeader>
      ) : null}
      {children}
    </AriaMenuSection>
  )
}

/**
 * CommandSeparator — visual divider between groups.
 */
function CommandSeparator({ className, ...props }: AriaSeparatorProps) {
  return (
    <AriaSeparator
      className={cn("-mx-1 h-px bg-border", className)}
      {...props}
    />
  )
}

export {
  Command,
  CommandInput,
  CommandList,
  CommandItem,
  CommandEmpty,
  CommandGroup,
  CommandSeparator,
}
