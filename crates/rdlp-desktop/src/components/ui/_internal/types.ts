/**
 * Prism UI — internal type utilities.
 *
 * Co-located with the wrappers; not top-level exported. When Prism is
 * extracted into a standalone package these become public API.
 *
 * Patterns sourced from typed-rocks/typescript.
 */

declare const __brand: unique symbol

/**
 * Opaque-handle type. Prevents arbitrary strings from being passed
 * where a brand-issued key is expected.
 *
 * @example
 *   type ToastKey = Branded<string, "PrismToast">
 *   function dismissToast(key: ToastKey): void
 *   dismissToast("typo")     // ✗ TS error: not a ToastKey
 *   dismissToast(showToast()) // ✓
 */
export type Branded<T, B> = T & { readonly [__brand]: B }

/**
 * String-union with autocomplete preserved AND extensibility allowed.
 * IDE suggests the canonical members; consumers can still pass arbitrary
 * strings without TS complaining.
 *
 * @example
 *   type ButtonVariant = LiteralUnion<"default" | "destructive" | "ghost">
 *   <Button variant="default" />  // suggested by IDE
 *   <Button variant="custom" />   // accepted, no TS error
 */
export type LiteralUnion<T extends string> = T | (string & {})

/**
 * Mapped-type that derives an `on*` handler shape from an event-name union.
 * Used for components that expose a constellation of related handlers,
 * potentially debounced/throttled via usehooks-ts.
 *
 * @example
 *   type DialogStates = "open" | "close" | "focusOut"
 *   type DialogHandlers = Handlers<DialogStates>
 *   //   ^? { onOpen?: () => void; onClose?: () => void; onFocusOut?: () => void }
 */
export type Handlers<Events extends string> = {
  [E in Events as `on${Capitalize<E>}`]?: () => void
}

/**
 * Widens selected optional keys of T to also accept `undefined` explicitly,
 * restoring pre-`exactOptionalPropertyTypes` semantics for those keys only.
 *
 * Mirrors the inverse of `type-fest`'s `SetNonNullable<T, K>`. Useful at
 * Prism wrapper boundaries when forwarding to upstream `react-aria-components`
 * primitives whose interfaces use `prop?: T` rather than `prop?: T | undefined`
 * (see adobe/react-spectrum#8717 — Adobe has not committed to fixing upstream).
 *
 * Pair with conditional spread inside the wrapper to bridge the widened
 * wrapper signature back to the strict RAC signature:
 *
 * @example
 *   type PrismSelectProps<T extends object> = WithUndefined<
 *     AriaSelectProps<T>,
 *     "items" | "textValue"
 *   >
 *
 *   function PrismSelect<T extends object>({ items, textValue, ...rest }: PrismSelectProps<T>) {
 *     return (
 *       <AriaSelect
 *         {...rest}
 *         {...(items !== undefined && { items })}
 *         {...(textValue !== undefined && { textValue })}
 *       />
 *     )
 *   }
 */
export type WithUndefined<T, K extends keyof T> = Omit<T, K> & {
  [P in K]?: T[P] | undefined
}
