// DownloadSection: throughput and chunking tunables.
//
// The two byte-valued settings are stored in bytes and displayed in MiB. Conversion
// happens HERE, at the value/onCommit boundary, so NumericField stays unit-agnostic.

import { Gauge } from "lucide-react";
import { NumericField } from "@/views/settings/NumericField";
import { bytesToMibDisplay, mibDisplayToBytes } from "@/views/settings/byteUnits";
import type { AppSettings } from "@/types";

interface Props {
    draft: AppSettings;
    onChange: (update: Partial<AppSettings>) => void;
}

/**
 * A stored byte value below ~512 KiB rounds to 0 MiB under `bytesToMibDisplay`
 * (`Math.round`), and 0 is below the field's `minValue={1}`. Passing that 0 straight
 * through as `NumericField`'s controlled `value` is worse than a clamp-on-blur: React
 * Aria's `useNumberFieldState` clamps a controlled `value` prop to `[minValue, maxValue]`
 * BEFORE it ever becomes `numberValue` (verified against the installed
 * `@react-stately/numberfield` source), so the field renders "1" on the very first
 * paint — no interaction required. From that point the input's "true" value (per React
 * Aria) already IS 1, so a plain focus+blur with no typing does not re-fire `onChange`
 * (nothing changed from the field's own point of view) — but the display is already
 * lying about the stored 500,000-byte value, and any subsequent edit (typing, a
 * stepper click) commits from that wrong baseline.
 *
 * The backend intentionally accepts sub-MiB values (`test_sub_mib_byte_values_are_accepted`
 * in `app_settings.rs`) — only the MiB-granularity UI can't represent them. Render the
 * field EMPTY for a sub-MiB value instead: there is no `value` prop for the clamp to act
 * on, so the misleading "1" never appears and a no-op focus/blur stays a no-op. The
 * placeholder/helper text carry the true byte count so the state stays visible, and stay
 * distinguishable from a genuinely unset (`null`) field, which keeps the existing
 * "inherit default" placeholder.
 */
function isSubMibBytes(bytes: number): boolean {
    return bytesToMibDisplay(bytes) === 0;
}

function byteFieldValue(bytes: number | null): number | null {
    if (bytes === null || isSubMibBytes(bytes)) {
        return null;
    }
    return bytesToMibDisplay(bytes);
}

function byteFieldPlaceholder(bytes: number | null, unsetPlaceholder: string): string {
    if (bytes !== null && isSubMibBytes(bytes)) {
        return `${bytes.toLocaleString()} B`;
    }
    return unsetPlaceholder;
}

function byteFieldHelper(bytes: number | null, defaultHelper: string): string {
    if (bytes !== null && isSubMibBytes(bytes)) {
        return `Currently set to ${bytes.toLocaleString()} bytes via the config file (below 1 MiB, so this field shows it as blank). Entering a value here replaces it with a whole-MiB size.`;
    }
    return defaultHelper;
}

export function DownloadSection({ draft, onChange }: Props) {
    return (
        <section
            id="settings-download"
            aria-labelledby="settings-download-heading"
            className="settings-panel"
        >
            <h3 id="settings-download-heading" className="settings-panel-title">
                <Gauge className="size-3.5" />
                Download Behaviour
            </h3>
            <div className="grid grid-cols-2 gap-x-4 gap-y-3">
                <NumericField
                    id="concurrent-fragments"
                    placeholder="8"
                    label="Concurrent Fragments"
                    helper="Parallel fragment downloads. Higher uses more memory. Default 8."
                    value={draft.concurrent_fragments}
                    minValue={1}
                    maxValue={64}
                    onCommit={(v) => onChange({ concurrent_fragments: v })}
                />
                <NumericField
                    id="hls-head-probe-timeout"
                    placeholder="5"
                    label="HLS Probe Timeout"
                    helper="Timeout for the HLS HEAD probe. Default 5."
                    value={draft.hls_head_probe_timeout}
                    minValue={1}
                    maxValue={300}
                    onCommit={(v) => onChange({ hls_head_probe_timeout: v })}
                    suffix="s"
                />
                <NumericField
                    id="buffer-size"
                    placeholder={byteFieldPlaceholder(draft.buffer_size, "2")}
                    label="Buffer Size (MiB)"
                    helper={byteFieldHelper(
                        draft.buffer_size,
                        "Download buffer per connection. Default 2 MiB. Values below 1 MiB can be set in the config file.",
                    )}
                    value={byteFieldValue(draft.buffer_size)}
                    minValue={1}
                    maxValue={1024}
                    onCommit={(mib) =>
                        onChange({ buffer_size: mib === null ? null : mibDisplayToBytes(mib) })
                    }
                    suffix="MiB"
                />
                <NumericField
                    id="parallel-threshold"
                    placeholder={byteFieldPlaceholder(draft.parallel_threshold, "10")}
                    label="Parallel Threshold (MiB)"
                    helper={byteFieldHelper(
                        draft.parallel_threshold,
                        "Files at least this large are downloaded in parallel chunks. Default 10 MiB.",
                    )}
                    value={byteFieldValue(draft.parallel_threshold)}
                    minValue={1}
                    maxValue={1024}
                    onCommit={(mib) =>
                        onChange({ parallel_threshold: mib === null ? null : mibDisplayToBytes(mib) })
                    }
                    suffix="MiB"
                />
            </div>
        </section>
    );
}
