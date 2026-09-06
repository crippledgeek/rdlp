// E2E support file — loaded before every test.
//
// The Tauri IPC mock itself lives in ./tauriMock; this file wires it into the
// per-test lifecycle and gates on commands that fell through it.

import "./commands";
import "cypress-axe";

import { unregisteredCommands, unregisteredCommandError } from "./tauriMock";

// ---------------------------------------------------------------------------
// Inject Tauri mock before each test
// ---------------------------------------------------------------------------

beforeEach(() => {
    unregisteredCommands.clear();
    cy.visitWithMock();
});

// Rejecting is necessary but NOT sufficient to surface a harness gap, and this
// was measured rather than assumed: unregistering `available_codecs` and
// running the a11y spec against the rejecting fallback left all 8 tests GREEN.
// The rejection lands in a TanStack Query error state, `data` is undefined,
// `SystemSection` falls to `?? []` and returns null on the empty check — so the
// section simply vanishes and nothing asserts on its absence. Silent in a
// different way than the fabricated null was, but still silent.
//
// So the fallback also fails the run, naming the command. Note the blast
// radius: this hook is declared at column 0 in the support file, so it belongs
// to Mocha's ROOT suite, and a hook failure aborts outward to the suite that
// OWNS the hook — the whole SPEC FILE, not merely the enclosing describe.
// Measured, not reasoned: a probe spec with two top-level describes, provoking
// one unregistered command in the first test, reported 1 failing and 2 SKIPPED
// — the rest of the first describe and the whole second one. That is the right
// trade for a harness gate, since every one of those tests was running against
// a stub with a hole in it, but it is much wider than one test and the aborted
// tests are reported as skipped rather than failed.
afterEach(() => {
    const missing = [...unregisteredCommands];
    unregisteredCommands.clear();
    if (missing.length > 0) {
        throw unregisteredCommandError(missing);
    }
});
