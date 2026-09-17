// A stub `EffectiveNetwork` payload for section tests.
//
// Every value is deliberately DISTINCT from `EffectiveNetwork::DEFAULT`
// (30/60/60/3600/1800/8/2 MiB/10 MiB/5) and from every other field, so a
// placeholder assertion can only pass if the section really derives it from
// this payload — a leftover literal, or a field wired to the wrong key, fails.

import { BYTES_PER_MIB } from "@/views/settings/byteUnits";
import type { EffectiveNetwork } from "@/types";

export const effectiveNetworkStub: EffectiveNetwork = {
    socket_timeout_secs: 31,
    read_timeout_secs: 62,
    pool_idle_timeout_secs: 63,
    download_timeout_secs: 3601,
    merge_timeout_secs: 1801,
    concurrent_fragments: 9,
    buffer_size: 3 * BYTES_PER_MIB,
    parallel_threshold: 11 * BYTES_PER_MIB,
    hls_head_probe_timeout_secs: 6,
};

/**
 * A stub for the BUILT-IN defaults (`EffectiveNetwork::DEFAULT`, served by
 * `builtin_network_defaults`). Distinct from `effectiveNetworkStub` in every
 * field so a component that reads the effective value where the built-in one
 * is required (or vice versa) fails its assertion.
 */
export const builtinNetworkStub: EffectiveNetwork = {
    socket_timeout_secs: 41,
    read_timeout_secs: 72,
    pool_idle_timeout_secs: 61,
    download_timeout_secs: 3701,
    merge_timeout_secs: 1901,
    concurrent_fragments: 12,
    buffer_size: 4 * BYTES_PER_MIB,
    parallel_threshold: 13 * BYTES_PER_MIB,
    hls_head_probe_timeout_secs: 7,
};
