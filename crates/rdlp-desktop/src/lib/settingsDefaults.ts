// Product defaults applied when a settings value is not (yet) available.
//
// These name the *fact* — "what does rdlp do when the user has expressed no
// preference" — so the answer lives in one place. They deliberately do NOT wrap
// the `settings?.x ?? DEFAULT` expression in a shared helper: the call sites
// apply the same fact in different roles (DownloadConfig seeds a form
// control's initial value; playlistDownloadOptions computes a value that is
// sent), and a shared resolver would couple a form default to a batch default
// so that changing one changes the other silently.
//
// The Rust side has its own copy of each of these in `AppSettings::default()`;
// it is authoritative once the settings query resolves. These cover only the
// window before that, where the frontend has no value to read.

/** Embed thumbnails when the user has expressed no preference. */
export const DEFAULT_EMBED_THUMBNAIL = true;
