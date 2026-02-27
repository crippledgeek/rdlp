// Bundled JS port of rdlp-crypto PRNG algorithms for XHamster URL decryption.
//
// XHamster encrypts video format URLs by embedding hex-encoded ciphertext in
// the URL path. byte[0] = algorithm ID (1-7), bytes[1..5] = seed (little-endian
// i32), bytes[5..] = ciphertext XOR'd with PRNG stream.
//
// All PRNG algorithms operate on signed 32-bit integers, matching JavaScript's
// native bitwise operator semantics. Use Math.imul for multiplications to avoid
// 64-bit float precision loss on overflow.

// =============================================================================
// Constants
// =============================================================================

var PHI = 0x9e3779b9 | 0;           // golden ratio (wraps to signed i32)
var FMIX32_C1 = 0x85ebca77 | 0;
var FMIX32_C2 = 0xc2b2ae3d | 0;
var LCG_MULT = 1664525;
var LCG_INC = 1013904223;
var PCG_MULT = 0x2c9277b5 | 0;
var PCG_INC = 0xac564b05 | 0;
var WEYL_ROL_INC = 0x6d2b79f5 | 0;
var ROL_SCRAMBLE_MULT = 0x27d4eb2d | 0;
var XORSHIFT_ADD_CONST = 0xa5a5a5a5 | 0;
var MXS_MULT1 = 0x7feb352d | 0;
var MXS_MULT2 = 0x846ca68b | 0;

// =============================================================================
// Helper functions
// =============================================================================

/// Rotate left on unsigned 32 bits, return signed i32.
function rotateLeft(n, rotation) {
    var u = n >>> 0;
    return ((u << rotation) | (u >>> (32 - rotation))) | 0;
}

/// XOR-shift: left a, unsigned-right b, left c. Returns signed i32.
function xorshift(s, a, b, c) {
    var x = s | 0;
    x = (x ^ (x << a)) | 0;
    x = (x ^ ((x >>> 0) >>> b)) | 0;
    x = (x ^ (x << c)) | 0;
    return x;
}

/// fmix32: MurmurHash3 32-bit finalizer.
function fmix32(s) {
    var x = s | 0;
    x = (x ^ ((x >>> 0) >>> 16)) | 0;
    x = Math.imul(x, FMIX32_C1) | 0;
    x = (x ^ ((x >>> 0) >>> 13)) | 0;
    x = Math.imul(x, FMIX32_C2) | 0;
    x = (x ^ ((x >>> 0) >>> 16)) | 0;
    return x;
}

/// ROL scramble: rotate-left, add PHI, xor-shift-11, multiply.
function rolScramble(s, rotation) {
    var x = rotateLeft(s, rotation);
    x = (x + PHI) | 0;
    x = (x ^ ((x >>> 0) >>> 11)) | 0;
    x = Math.imul(x, ROL_SCRAMBLE_MULT) | 0;
    return x;
}

// =============================================================================
// PRNG algorithms — each returns a closure { nextByte() }
// =============================================================================

function lcgPrng(seed) {
    var state = seed | 0;
    return {
        nextByte: function() {
            state = (Math.imul(state, LCG_MULT) + LCG_INC) | 0;
            return state & 0xFF;
        }
    };
}

function xorshift32Prng(seed) {
    var state = seed | 0;
    return {
        nextByte: function() {
            state = xorshift(state, 13, 17, 5);
            return state & 0xFF;
        }
    };
}

function weylFmix32Prng(seed) {
    var state = seed | 0;
    return {
        nextByte: function() {
            state = (state + PHI) | 0;
            return fmix32(state) & 0xFF;
        }
    };
}

function weylRol7Prng(seed) {
    var state = seed | 0;
    return {
        nextByte: function() {
            state = (state + WEYL_ROL_INC) | 0;
            return rolScramble(state, 7) & 0xFF;
        }
    };
}

function xorshiftAddPrng(seed) {
    var state = seed | 0;
    return {
        nextByte: function() {
            state = (xorshift(state, 7, 9, 8) + XORSHIFT_ADD_CONST) | 0;
            return state & 0xFF;
        }
    };
}

function lcgPcgPrng(seed) {
    var state = seed | 0;
    return {
        nextByte: function() {
            state = (Math.imul(state, PCG_MULT) + PCG_INC) | 0;
            // PCG output: xor-shift then variable right-shift
            var s2 = (state ^ ((state >>> 0) >>> 18)) | 0;
            var shift = ((state >>> 0) >>> 27) & 31;
            var out = ((s2 >>> 0) >>> shift) | 0;
            return out & 0xFF;
        }
    };
}

function weylMxsPrng(seed) {
    var state = seed | 0;
    return {
        nextByte: function() {
            state = (state + PHI) | 0;
            // MXS: xor with left-shift 5, multiply, xor with right-shift 15, multiply
            var x = (state ^ (state << 5)) | 0;
            x = Math.imul(x, MXS_MULT1) | 0;
            x = (x ^ ((x >>> 0) >>> 15)) | 0;
            x = Math.imul(x, MXS_MULT2) | 0;
            return x & 0xFF;
        }
    };
}

/// Create a PRNG for the given algorithm ID and seed. Returns null for unknown IDs.
function createPrng(algoId, seed) {
    switch (algoId) {
        case 1: return lcgPrng(seed);
        case 2: return xorshift32Prng(seed);
        case 3: return weylFmix32Prng(seed);
        case 4: return weylRol7Prng(seed);
        case 5: return xorshiftAddPrng(seed);
        case 6: return lcgPcgPrng(seed);
        case 7: return weylMxsPrng(seed);
        default: return null;
    }
}

// =============================================================================
// Hex decoding
// =============================================================================

/// Decode a hex string to an array of bytes. Returns null on failure.
function hexDecode(hex) {
    if (!hex || hex.length < 2 || hex.length % 2 !== 0) {
        return null;
    }
    var bytes = [];
    for (var i = 0; i < hex.length; i += 2) {
        var hi = parseInt(hex[i], 16);
        var lo = parseInt(hex[i + 1], 16);
        if (isNaN(hi) || isNaN(lo)) {
            return null;
        }
        bytes.push((hi << 4) | lo);
    }
    return bytes;
}

/// Read a little-endian i32 from 4 bytes at the given offset.
function readI32LE(bytes, offset) {
    var b0 = bytes[offset] & 0xFF;
    var b1 = bytes[offset + 1] & 0xFF;
    var b2 = bytes[offset + 2] & 0xFF;
    var b3 = bytes[offset + 3] & 0xFF;
    return (b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)) | 0;
}

// =============================================================================
// Core decryption
// =============================================================================

/// Decipher hex-encoded ciphertext (bare hex string).
/// Returns the deciphered latin-1 string, or null on failure.
function decipherHexBytes(hexStr) {
    var bytes = hexDecode(hexStr);
    if (!bytes || bytes.length < 6) {
        return null;
    }

    var algoId = bytes[0];
    var seed = readI32LE(bytes, 1);

    var prng = createPrng(algoId, seed);
    if (!prng) {
        return null;
    }

    // XOR remaining bytes with PRNG stream
    var result = '';
    for (var i = 5; i < bytes.length; i++) {
        result += String.fromCharCode((bytes[i] ^ prng.nextByte()) & 0xFF);
    }
    return result;
}

/// Decipher an XHamster format URL.
///
/// Supports two input formats:
/// 1. Full URL with hex path: "https://host/{hex}/{remainder}"
/// 2. Bare hex string: raw hex-encoded ciphertext
///
/// Returns the deciphered URL string, or null if not recognized.
function decipherFormatUrl(formatUrl) {
    if (!formatUrl || typeof formatUrl !== 'string') {
        return null;
    }

    // Try as full URL with hex path (must contain "://")
    var schemeIdx = formatUrl.indexOf('://');
    if (schemeIdx !== -1) {
        // Find the path start after host
        var hostStart = schemeIdx + 3;
        var pathStart = formatUrl.indexOf('/', hostStart);
        if (pathStart !== -1) {
            var path = formatUrl.substring(pathStart);
            // Match /{hex}{remainder} where hex is 12+ hex chars and remainder starts with / or ,
            var match = path.match(/^\/([0-9a-fA-F]{12,})([\/,].+)$/);
            if (match) {
                var hexPart = match[1];
                var remainder = match[2];
                var deciphered = decipherHexBytes(hexPart);
                if (deciphered !== null) {
                    var newPath = '/' + deciphered + remainder;
                    return formatUrl.substring(0, pathStart) + newPath;
                }
            }
        }
    }

    // Fallback: treat entire string as bare hex-encoded ciphertext
    if (formatUrl.length >= 12 && formatUrl.length % 2 === 0) {
        // Quick check: must look like hex
        if (/^[0-9a-fA-F]+$/.test(formatUrl)) {
            return decipherHexBytes(formatUrl);
        }
    }

    return null;
}
