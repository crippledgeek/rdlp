// A stub `NetworkDefaults` payload for section tests.
//
// Every value is deliberately DISTINCT from `EffectiveNetwork::DEFAULT`
// (30/60/60/3600/1800/8/2 MiB/10 MiB/5/60) and `EffectiveNetwork::RANGES`,
// and from every other field, so a placeholder or bound assertion can only
// pass if the section really derives it from this payload — a leftover
// literal, or a field wired to the wrong key, fails.

import { BYTES_PER_MIB } from "@/views/settings/byteUnits";
import type { EffectiveNetwork, NetworkDefaults, NetworkRanges } from "@/types";

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
    hls_expansion_timeout_secs: 61,
};

/**
 * A stub for the BUILT-IN defaults (`EffectiveNetwork::DEFAULT`). Distinct
 * from `effectiveNetworkStub` in every field so a component that reads the
 * effective value where the built-in one is required (or vice versa) fails
 * its assertion.
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
    hls_expansion_timeout_secs: 71,
};

/**
 * A stub for the owning ranges (`EffectiveNetwork::RANGES`). Every bound is
 * distinct from the real one, so a control that clamps to a literal instead of
 * the payload fails. `pool_idle_timeout_secs.min` stays `0`: it is the
 * "eviction disabled" sentinel the section must exclude from its numeric
 * control, which is exactly what its test asserts.
 */
export const networkRangesStub: NetworkRanges = {
    socket_timeout_secs: { min: 2, max: 302 },
    read_timeout_secs: { min: 3, max: 603 },
    pool_idle_timeout_secs: { min: 0, max: 3604 },
    download_timeout_secs: { min: 5, max: 86405 },
    merge_timeout_secs: { min: 6, max: 86406 },
    concurrent_fragments: { min: 2, max: 67 },
    buffer_size: { min: 1, max: 512 * BYTES_PER_MIB },
    parallel_threshold: { min: 1, max: 256 * BYTES_PER_MIB },
    hls_head_probe_timeout_secs: { min: 2, max: 309 },
    hls_expansion_timeout_secs: { min: 2, max: 610 },
};

export const networkDefaultsStub: NetworkDefaults = {
    effective: effectiveNetworkStub,
    builtin: builtinNetworkStub,
    ranges: networkRangesStub,
};
