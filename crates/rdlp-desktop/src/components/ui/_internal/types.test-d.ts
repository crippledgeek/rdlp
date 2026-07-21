/**
 * Type-inference assertions for the Prism _internal type utilities.
 *
 * This file runs under Vitest's typecheck mode only:
 *
 *   pnpm test -- --typecheck --run
 *
 * Files matching `*.test-d.{ts,tsx}` are excluded from the runtime test pool
 * (see vitest.config.ts) so the assertions here are evaluated by `tsc` via
 * Vitest's typecheck integration, not by the runner. Use `assertType` and
 * `expectTypeOf` for compile-time checks; do not put runtime expectations in
 * this file.
 */
import { describe, test, expectTypeOf, assertType } from "vitest"
import type { Branded, LiteralUnion, Handlers, WithUndefined } from "./types"

type ToastKey = Branded<string, "PrismToast">

describe("Prism _internal type utilities", () => {
  test("Branded — prevents passing a plain string where a brand is required", () => {
    const issuedKey = "abc" as ToastKey

    assertType<ToastKey>(issuedKey)

    // @ts-expect-error — plain string is not a ToastKey
    assertType<ToastKey>("not-a-key")
  })

  test("LiteralUnion — preserves canonical members AND accepts custom strings", () => {
    type ButtonVariant = LiteralUnion<"default" | "destructive" | "ghost">

    assertType<ButtonVariant>("default")
    assertType<ButtonVariant>("custom")

    expectTypeOf<ButtonVariant>().toMatchTypeOf<string>()
  })

  test("Handlers — derives on*<EventName> shape from event-name union", () => {
    type DialogStates = "open" | "close" | "focusOut"
    type DialogHandlers = Handlers<DialogStates>

    const handlers: DialogHandlers = {
      onOpen: () => {},
      onClose: () => {},
      onFocusOut: () => {},
    }

    expectTypeOf(handlers).toMatchTypeOf<{
      onOpen?: () => void
      onClose?: () => void
      onFocusOut?: () => void
    }>()
  })

  test("WithUndefined — widens selected optional keys to accept undefined explicitly", () => {
    interface Strict { items?: string[]; textValue?: string; other?: number }
    type Widened = WithUndefined<Strict, "items" | "textValue">

    // Widened keys accept explicit undefined
    assertType<Widened>({ items: undefined, textValue: undefined })
    assertType<Widened>({ items: ["a"], textValue: "x" })
    assertType<Widened>({})

    // Non-widened keys keep their strict shape
    expectTypeOf<Widened["other"]>().toEqualTypeOf<number | undefined>() // optional inherits | undefined from T[P]
    // ^ note: under exactOptionalPropertyTypes, `other?: number` inferred via Omit<T, K>
    //   keeps the original strict shape — assertType<Widened>({ other: undefined })
    //   would still fail compile-time, which is the desired behavior.
  })
})
