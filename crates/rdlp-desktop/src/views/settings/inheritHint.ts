// The one sentence every "leave empty to inherit" numeric setting appends to
// its helper text. The placeholder shows the inherited value itself (sourced
// over IPC from `EffectiveNetwork`, #611); this names where it comes from.
//
// Wording: "inherited from the base configuration", never "default" — the
// value shown may come from the user's `config.toml`, not the built-in default.

export const INHERIT_HINT = "Leave empty to inherit the value shown, from the base configuration.";

/** Append the inherit hint to a field-specific description. */
export function withInheritHint(description: string): string {
    return `${description} ${INHERIT_HINT}`;
}
