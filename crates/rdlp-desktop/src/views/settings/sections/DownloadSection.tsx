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
                    placeholder="2"
                    label="Buffer Size (MiB)"
                    helper="Download buffer per connection. Default 2 MiB. Values below 1 MiB can be set in the config file."
                    value={draft.buffer_size === null ? null : bytesToMibDisplay(draft.buffer_size)}
                    minValue={1}
                    maxValue={1024}
                    onCommit={(mib) =>
                        onChange({ buffer_size: mib === null ? null : mibDisplayToBytes(mib) })
                    }
                    suffix="MiB"
                />
                <NumericField
                    id="parallel-threshold"
                    placeholder="10"
                    label="Parallel Threshold (MiB)"
                    helper="Files at least this large are downloaded in parallel chunks. Default 10 MiB."
                    value={
                        draft.parallel_threshold === null
                            ? null
                            : bytesToMibDisplay(draft.parallel_threshold)
                    }
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
